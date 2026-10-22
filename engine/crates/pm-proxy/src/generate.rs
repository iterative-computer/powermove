use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use tempfile::NamedTempFile;

use crate::{ProxyError, ProxyRecipe, ProxyStore, Result, StoredProxy, disk_guard, free_bytes};
use pm_protocol::priority::{ProxyFormat, ProxySize};

pub struct GenerationControl<'a> {
    pub cancel: &'a AtomicBool,
    /// The job engine supplies playback/export contention. Generation stops
    /// its codec process while this is true, then resumes the same process.
    pub pause: &'a dyn Fn() -> bool,
    pub on_pause: &'a dyn Fn(Option<PauseReason>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    InteractiveWork,
    LowDisk,
}

pub struct GenerationRequest<'a> {
    pub source: &'a Path,
    pub source_hash: &'a str,
    pub format: ProxyFormat,
    pub size: ProxySize,
    pub ffmpeg: &'a Path,
    pub ffprobe: &'a Path,
    pub store: &'a ProxyStore,
    pub control: GenerationControl<'a>,
}

#[derive(Debug, Clone)]
pub struct GeneratedProxy {
    pub stored: StoredProxy,
    pub reused: bool,
}

/// Generates a whole clip, verifies frame timestamps and source metadata,
/// then atomically installs it. The source hash is rechecked immediately
/// before and after encoding so a linked file changed underfoot cannot
/// poison a content-addressed entry.
pub fn generate(request: GenerationRequest<'_>) -> Result<GeneratedProxy> {
    generate_with_disk_probe(request, &free_bytes)
}

/// Uses the host's available/capacity measurement (for example a volume
/// quota). The same guard is enforced before allocation and while encoding.
pub fn generate_with_disk_probe(
    request: GenerationRequest<'_>,
    disk_space: &dyn Fn(&Path) -> Result<(u64, u64)>,
) -> Result<GeneratedProxy> {
    if request.control.cancel.load(Ordering::Relaxed) {
        return Err(ProxyError::Cancelled);
    }
    if let Some(manifest) = crate::sequence::read_manifest(request.source)? {
        return crate::sequence::generate_sequence(request, disk_space, manifest);
    }
    let input = probe(request.ffprobe, request.source)?;
    let native_source = pm_media::probe::probe(request.source)
        .map_err(|e| ProxyError::InvalidMedia(format!("source structure: {e}")))?;
    let source_video = video(&input)?;
    let width = u32_field(source_video, "width")?;
    let height = u32_field(source_video, "height")?;
    let depth = bit_depth(source_video)?.max(
        native_source
            .probe
            .video
            .as_ref()
            .map_or(8, |v| v.codec.bit_depth),
    );
    let alpha = has_alpha(source_video)
        || native_source
            .probe
            .video
            .as_ref()
            .is_some_and(|v| v.alpha.is_some());
    let recipe = ProxyRecipe::new(request.format, request.size, width, height, depth, alpha)?;
    // A host grant may refer to a file that has changed since import. Even a
    // cache hit must prove that the bytes now under the handle match its key.
    verify_source_hash(request.source, request.source_hash, &request.control)?;
    match request.store.lookup(request.source_hash, &recipe) {
        Ok(Some(stored)) => {
            return Ok(GeneratedProxy {
                stored,
                reused: true,
            });
        }
        Err(ProxyError::CorruptStore(_)) => request.store.remove(request.source_hash, &recipe)?,
        Ok(None) => (),
        Err(e) => return Err(e),
    }
    let mut low_disk_reported = false;
    while !disk_guard_pair(request.store.root(), disk_space)? {
        if request.control.cancel.load(Ordering::Relaxed) {
            return Err(ProxyError::Cancelled);
        }
        if !low_disk_reported {
            (request.control.on_pause)(Some(PauseReason::LowDisk));
            low_disk_reported = true;
        }
        thread::sleep(Duration::from_secs(2));
    }
    if low_disk_reported {
        (request.control.on_pause)(None);
    }
    let temp = NamedTempFile::new_in(request.store.root())?;
    transcode(&request, &input, &recipe, temp.path(), disk_space)?;
    if request.control.cancel.load(Ordering::Relaxed) {
        return Err(ProxyError::Cancelled);
    }
    verify_source_hash(request.source, request.source_hash, &request.control)?;
    let output = probe(request.ffprobe, temp.path())?;
    validate(
        request.source,
        temp.path(),
        request.ffmpeg,
        request.ffprobe,
        &native_source,
        &input,
        &output,
        &recipe,
    )?;
    let stored = request.store.commit(request.source_hash, &recipe, temp)?;
    Ok(GeneratedProxy {
        stored,
        reused: false,
    })
}

fn verify_source_hash(
    source: &Path,
    expected: &str,
    control: &GenerationControl<'_>,
) -> Result<()> {
    let mut file = File::open(source)?;
    let before = file.metadata()?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut was_paused = false;
    loop {
        if control.cancel.load(Ordering::Relaxed) {
            return Err(ProxyError::Cancelled);
        }
        while (control.pause)() {
            if !was_paused {
                (control.on_pause)(Some(PauseReason::InteractiveWork));
                was_paused = true;
            }
            if control.cancel.load(Ordering::Relaxed) {
                return Err(ProxyError::Cancelled);
            }
            thread::sleep(Duration::from_millis(200));
        }
        if was_paused {
            (control.on_pause)(None);
            was_paused = false;
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let after = std::fs::metadata(source)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(ProxyError::InvalidMedia(
            "source changed while it was hashed".into(),
        ));
    }
    let actual = hasher.finalize().to_hex().to_string();
    if actual != expected.to_ascii_lowercase() {
        return Err(ProxyError::InvalidMedia(
            "source changed since its content hash was recorded".into(),
        ));
    }
    Ok(())
}

fn probe(ffprobe: &Path, path: &Path) -> Result<Value> {
    let result = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()?;
    if !result.status.success() {
        return Err(ProxyError::InvalidMedia(
            String::from_utf8_lossy(&result.stderr).trim().to_owned(),
        ));
    }
    serde_json::from_slice(&result.stdout).map_err(|e| ProxyError::InvalidMedia(e.to_string()))
}

fn video(value: &Value) -> Result<&Value> {
    value["streams"]
        .as_array()
        .and_then(|s| s.iter().find(|v| v["codec_type"] == "video"))
        .ok_or_else(|| ProxyError::InvalidMedia("no video stream".into()))
}

fn audio(value: &Value) -> Vec<&Value> {
    value["streams"].as_array().map_or_else(Vec::new, |s| {
        s.iter().filter(|v| v["codec_type"] == "audio").collect()
    })
}

fn pcm_source_codec(stream: &Value) -> Result<Option<&str>> {
    let codec = string(stream, "codec_name").unwrap_or("");
    if !codec.starts_with("pcm_") {
        return Ok(None);
    }
    match codec {
        "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" | "pcm_f64le" | "pcm_s16be"
        | "pcm_s24be" | "pcm_s32be" => Ok(Some(codec)),
        _ => Err(ProxyError::InvalidMedia(format!(
            "PCM coding {codec} cannot be preserved by this MOV recipe"
        ))),
    }
}

fn string<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v[key].as_str()
}

fn u32_field(v: &Value, key: &str) -> Result<u32> {
    v[key]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| ProxyError::InvalidMedia(format!("missing {key}")))
}

fn bit_depth(v: &Value) -> Result<u8> {
    if let Some(bits) = string(v, "bits_per_raw_sample").and_then(|s| s.parse::<u8>().ok()) {
        if bits > 0 {
            return Ok(bits);
        }
    }
    let pix = string(v, "pix_fmt")
        .ok_or_else(|| ProxyError::InvalidMedia("unknown pixel format".into()))?;
    let depth = if pix.contains("p16") || pix.contains("p016") || pix.contains("64") {
        16
    } else if pix.contains("p12") || pix.contains("p012") {
        12
    } else if pix.contains("p10") || pix.contains("p010") {
        10
    } else if pix.contains("p9") {
        9
    } else {
        8
    };
    Ok(depth)
}

fn has_alpha(v: &Value) -> bool {
    string(v, "pix_fmt").is_some_and(|p| {
        p.starts_with("yuva")
            || p.starts_with("gbrap")
            || p.starts_with("rgba")
            || p.starts_with("bgra")
    })
}

fn transcode(
    request: &GenerationRequest<'_>,
    input: &Value,
    recipe: &ProxyRecipe,
    destination: &Path,
    disk_space: &dyn Fn(&Path) -> Result<(u64, u64)>,
) -> Result<()> {
    let source_video = video(input)?;
    let mut cmd = Command::new(request.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-y",
        "-copyts",
        "-avoid_negative_ts",
        "disabled",
        "-noautorotate",
    ]);
    cmd.arg("-i").arg(request.source);
    cmd.args([
        "-map",
        "0:v:0",
        "-map",
        "0:a?",
        "-map_metadata",
        "0",
        "-map_chapters",
        "0",
    ]);
    cmd.args(["-map_metadata:s:v:0", "0:s:v:0"]);
    let rgb_source = string(source_video, "color_space") == Some("gbr")
        || string(source_video, "pix_fmt")
            .is_some_and(|p| p.starts_with("rgb") || p.starts_with("gbr") || p.starts_with("bgr"));
    let output_matrix = if string(source_video, "color_primaries") == Some("bt2020") {
        "bt2020nc"
    } else {
        "bt709"
    };
    let scale = if rgb_source {
        format!(
            "scale={}:{}:flags=lanczos:in_range=full:out_range=tv:out_color_matrix={output_matrix}",
            recipe.width, recipe.height
        )
    } else {
        format!("scale={}:{}:flags=lanczos", recipe.width, recipe.height)
    };
    cmd.args([
        "-vf",
        &scale,
        "-fps_mode",
        "passthrough",
        "-enc_time_base",
        "demux",
    ]);
    let pix = match recipe.codec() {
        "prores_4444" => "ayuv64le",
        "hevc_10" => "p010le",
        _ => "p216le",
    };
    cmd.args(["-pix_fmt", pix]);
    match recipe.codec() {
        "prores_4444" => {
            cmd.args(["-c:v", "prores_videotoolbox", "-profile:v", "4444"]);
        }
        "prores_proxy" => {
            cmd.args(["-c:v", "prores_videotoolbox", "-profile:v", "proxy"]);
        }
        "prores_lt" => {
            cmd.args(["-c:v", "prores_videotoolbox", "-profile:v", "lt"]);
        }
        "hevc_10" => {
            cmd.args([
                "-c:v",
                "hevc_videotoolbox",
                "-profile:v",
                "main10",
                "-tag:v",
                "hvc1",
            ]);
        }
        _ => return Err(ProxyError::InvalidMedia("unknown codec recipe".into())),
    }
    // Set tags explicitly: VideoToolbox otherwise may write a different
    // matrix and still leave the source's container tags copied over.
    for (field, option) in [
        ("color_primaries", "-color_primaries"),
        ("color_transfer", "-color_trc"),
        ("color_space", "-colorspace"),
        ("color_range", "-color_range"),
    ] {
        if rgb_source && matches!(field, "color_space" | "color_range") {
            continue;
        }
        if let Some(value) =
            string(source_video, field).filter(|s| *s != "unknown" && *s != "reserved")
        {
            cmd.arg(option).arg(value);
        }
    }
    if rgb_source {
        cmd.args(["-colorspace", output_matrix, "-color_range", "tv"]);
    }
    // `scale` adjusts sample aspect for the rounded output dimensions to
    // preserve display aspect. `-aspect` would override that adjustment.
    let audio_streams = audio(input);
    if !audio_streams.is_empty() {
        // PCM in MOV avoids another lossy generation. Select float PCM if a
        // source is float; otherwise 24-bit integer PCM. Existing PCM keeps
        // its exact coding and bit depth, including 24-bit samples that
        // FFmpeg reports with an s32 decoder sample format.
        for (index, stream) in audio_streams.iter().enumerate() {
            let codec = pcm_source_codec(stream)?.unwrap_or_else(|| {
                match string(stream, "sample_fmt").unwrap_or("") {
                    s if s.starts_with("dbl") => "pcm_f64le",
                    s if s.starts_with("flt") => "pcm_f32le",
                    s if s.starts_with("s32") => "pcm_s32le",
                    _ => "pcm_s24le",
                }
            });
            cmd.arg(format!("-c:a:{index}")).arg(codec);
        }
    }
    // MOV's timescale follows source video units when their denominator fits.
    if let Some((num, den)) = string(source_video, "time_base").and_then(parse_ratio) {
        if num == 1 && den > 0 && den <= i32::MAX as i128 {
            cmd.arg("-video_track_timescale").arg(den.to_string());
        }
    }
    cmd.args(["-movflags", "use_metadata_tags", "-f", "mov"]);
    cmd.arg(destination);
    let log = NamedTempFile::new_in(request.store.root())?;
    let stderr = log.reopen()?;
    cmd.stdout(Stdio::null()).stderr(Stdio::from(stderr));
    let mut child = ChildGuard::new(cmd.spawn()?);
    let mut stopped = false;
    let mut last_reason = None;
    loop {
        if request.control.cancel.load(Ordering::Relaxed) {
            return Err(ProxyError::Cancelled);
        }
        if let Some(status) = child.child.try_wait()? {
            child.finished = true;
            if status.success() {
                return Ok(());
            }
            let log = std::fs::read_to_string(log.path()).unwrap_or_default();
            let tail = log
                .chars()
                .rev()
                .take(2048)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            return Err(ProxyError::Codec(format!("{status}: {tail}")));
        }
        let disk_low = !disk_guard_pair(request.store.root(), disk_space)?;
        let reason = if disk_low {
            Some(PauseReason::LowDisk)
        } else if (request.control.pause)() {
            Some(PauseReason::InteractiveWork)
        } else {
            None
        };
        let pause = reason.is_some();
        if reason != last_reason {
            (request.control.on_pause)(reason);
            last_reason = reason;
        }
        if pause != stopped {
            signal(
                &mut child.child,
                if pause { libc::SIGSTOP } else { libc::SIGCONT },
            )?;
            stopped = pause;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn disk_guard_pair(root: &Path, disk_space: &dyn Fn(&Path) -> Result<(u64, u64)>) -> Result<bool> {
    let (free, capacity) = disk_space(root)?;
    Ok(disk_guard(free, capacity))
}

fn signal(child: &mut Child, sig: i32) -> Result<()> {
    // SAFETY: pid belongs to this child, and sig is SIGSTOP or SIGCONT.
    if unsafe { libc::kill(child.id() as i32, sig) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

struct ChildGuard {
    child: Child,
    finished: bool,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self {
            child,
            finished: false,
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.finished {
            // A stopped child must be continued before termination on macOS.
            let _ = signal(&mut self.child, libc::SIGCONT);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn parse_ratio(raw: &str) -> Option<(i128, i128)> {
    let (a, b) = raw.split_once('/')?;
    let n = a.parse::<i128>().ok()?;
    let d = b.parse::<i128>().ok()?;
    (d > 0).then_some((n, d))
}

fn integer(v: &Value, key: &str) -> Option<i128> {
    v[key]
        .as_i64()
        .map(i128::from)
        .or_else(|| string(v, key).and_then(|s| s.parse::<i128>().ok()))
}

fn display_aspect(stream: &Value) -> Result<(i128, i128)> {
    let width = i128::from(u32_field(stream, "width")?);
    let height = i128::from(u32_field(stream, "height")?);
    let (sar_n, sar_d) = string(stream, "sample_aspect_ratio")
        .and_then(|s| s.split_once(':'))
        .and_then(|(a, b)| Some((a.parse::<i128>().ok()?, b.parse::<i128>().ok()?)))
        .filter(|(a, b)| *a > 0 && *b > 0)
        .unwrap_or((1, 1));
    Ok((width * sar_n, height * sar_d))
}

fn side_data<'a>(stream: &'a Value, kind: &str) -> Option<&'a Value> {
    stream["side_data_list"].as_array().and_then(|list| {
        list.iter()
            .find(|item| string(item, "side_data_type") == Some(kind))
    })
}

fn audio_sample_count(ffprobe: &Path, path: &Path, stream: usize) -> Result<u64> {
    let mut child = ChildGuard::new(
        Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                &format!("a:{stream}"),
                "-show_frames",
                "-show_entries",
                "frame=nb_samples",
                "-of",
                "csv=p=0",
            ])
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let stdout = child.child.stdout.take().expect("piped above");
    let mut total = 0u64;
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let samples = line.trim().parse::<u64>().map_err(|_| {
            ProxyError::Validation("audio frame sample count was not numeric".into())
        })?;
        total = total
            .checked_add(samples)
            .ok_or_else(|| ProxyError::Validation("audio sample count overflows".into()))?;
    }
    let status = child.child.wait()?;
    child.finished = true;
    if !status.success() || total == 0 {
        return Err(ProxyError::Validation(
            "audio sample count unavailable".into(),
        ));
    }
    Ok(total)
}

fn canonical_pcm_hash(ffmpeg: &Path, path: &Path, stream: usize) -> Result<Vec<u8>> {
    let result = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-map",
            &format!("0:a:{stream}"),
            "-vn",
            "-c:a",
            "pcm_f64le",
            "-f",
            "hash",
            "-hash",
            "SHA256",
            "-",
        ])
        .output()?;
    if !result.status.success() || !result.stdout.starts_with(b"SHA256=") {
        return Err(ProxyError::Validation(format!(
            "could not hash decoded PCM: {}",
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    Ok(result.stdout)
}

fn validate(
    source_path: &Path,
    output_path: &Path,
    ffmpeg: &Path,
    ffprobe: &Path,
    source_native: &pm_media::probe::Probed,
    source: &Value,
    output: &Value,
    recipe: &ProxyRecipe,
) -> Result<()> {
    let src = video(source)?;
    let dst = video(output)?;
    let fail = |message: &str| ProxyError::Validation(message.into());
    if u32_field(dst, "width")? != recipe.width || u32_field(dst, "height")? != recipe.height {
        return Err(fail("wrong proxy dimensions"));
    }
    let tag = string(dst, "codec_tag_string").unwrap_or("");
    let expected = match recipe.codec() {
        "prores_4444" => "ap4h",
        "prores_proxy" => "apco",
        "prores_lt" => "apcs",
        "hevc_10" => "hvc1",
        _ => "",
    };
    if tag != expected {
        return Err(fail("wrong codec or ProRes profile"));
    }
    if bit_depth(dst)? < recipe.source_depth.min(10)
        || (recipe.source_depth > 10 && bit_depth(dst)? < 12)
    {
        return Err(fail("proxy lost source bit depth"));
    }
    if recipe.source_has_alpha && !has_alpha(dst) {
        return Err(fail("proxy lost alpha"));
    }
    for kind in [
        "Display Matrix",
        "Mastering display metadata",
        "Content light level metadata",
        "Frame Cropping",
    ] {
        if let Some(a) = side_data(src, kind) {
            if side_data(dst, kind) != Some(a) {
                return Err(ProxyError::Validation(format!("proxy changed {kind}")));
            }
        }
    }
    let output_native = pm_media::probe::probe(output_path)
        .map_err(|e| ProxyError::Validation(format!("proxy structure: {e}")))?;
    let (source_video, output_video) = (
        source_native.probe.video.as_ref(),
        output_native.probe.video.as_ref(),
    );
    if let (Some(source_video), Some(output_video)) = (source_video, output_video) {
        if source_video.rotation != output_video.rotation {
            return Err(fail("proxy changed display rotation"));
        }
        if source_video.color.mastering != output_video.color.mastering
            || source_video.color.content_light != output_video.color.content_light
            || source_video.color.icc.as_ref().map(|p| &p.blake3)
                != output_video.color.icc.as_ref().map(|p| &p.blake3)
        {
            return Err(fail("proxy changed HDR or ICC metadata"));
        }
        if source_video.fields.order != output_video.fields.order {
            return Err(fail("proxy changed field order"));
        }
        match (source_video.clean_aperture, output_video.clean_aperture) {
            (Some(a), Some(b)) => {
                let equivalent = (a.width / f64::from(u32_field(src, "width")?)
                    - b.width / f64::from(recipe.width))
                .abs()
                    < 1e-8
                    && (a.height / f64::from(u32_field(src, "height")?)
                        - b.height / f64::from(recipe.height))
                    .abs()
                        < 1e-8
                    && (a.horizontal_offset / f64::from(u32_field(src, "width")?)
                        - b.horizontal_offset / f64::from(recipe.width))
                    .abs()
                        < 1e-8
                    && (a.vertical_offset / f64::from(u32_field(src, "height")?)
                        - b.vertical_offset / f64::from(recipe.height))
                    .abs()
                        < 1e-8;
                if !equivalent {
                    return Err(fail("proxy changed clean aperture"));
                }
            }
            (None, None) => (),
            _ => return Err(fail("proxy changed clean aperture")),
        }
    }
    let src_dar = display_aspect(src)?;
    let dst_dar = display_aspect(dst)?;
    if src_dar.0 * dst_dar.1 != dst_dar.0 * src_dar.1 {
        return Err(fail("proxy changed display aspect"));
    }
    for key in [
        "color_primaries",
        "color_transfer",
        "color_space",
        "color_range",
    ] {
        let rgb_source = string(src, "color_space") == Some("gbr")
            || string(src, "pix_fmt").is_some_and(|p| {
                p.starts_with("rgb") || p.starts_with("gbr") || p.starts_with("bgr")
            });
        if rgb_source && matches!(key, "color_space" | "color_range") {
            continue;
        }
        if let Some(source_value) = string(src, key).filter(|s| *s != "unknown" && *s != "N/A") {
            if string(dst, key) != Some(source_value) {
                return Err(ProxyError::Validation(format!("proxy changed {key}")));
            }
        }
    }
    if string(src, "color_space") == Some("gbr") {
        let expected_matrix = if string(src, "color_primaries") == Some("bt2020") {
            "bt2020nc"
        } else {
            "bt709"
        };
        if string(dst, "color_space") != Some(expected_matrix)
            || string(dst, "color_range") != Some("tv")
        {
            return Err(fail("proxy RGB conversion tags are wrong"));
        }
    }
    for key in ["timecode", "reel_name", "reel", "tape_name"] {
        let source_tag = src["tags"][key]
            .as_str()
            .or_else(|| source["format"]["tags"][key].as_str());
        if let Some(value) = source_tag {
            let output_tag = dst["tags"][key]
                .as_str()
                .or_else(|| output["format"]["tags"][key].as_str());
            if output_tag != Some(value) {
                return Err(ProxyError::Validation(format!("proxy changed {key}")));
            }
        }
    }
    let source_index = source_native
        .index
        .as_ref()
        .ok_or_else(|| fail("source frame index missing"))?;
    let output_index = output_native
        .index
        .as_ref()
        .ok_or_else(|| fail("proxy frame index missing"))?;
    if source_index.frames.is_empty() || source_index.frames.len() != output_index.frames.len() {
        return Err(fail("proxy changed frame count"));
    }
    let a_tb = source_index.timebase;
    let b_tb = output_index.timebase;
    for (a, b) in source_index.frames.iter().zip(&output_index.frames) {
        if i128::from(a.pts) * i128::from(a_tb.num()) * i128::from(b_tb.den())
            != i128::from(b.pts) * i128::from(b_tb.num()) * i128::from(a_tb.den())
        {
            return Err(fail("proxy changed exact presentation timestamps"));
        }
    }
    let source_audio = audio(source);
    let output_audio = audio(output);
    if source_audio.len() != output_audio.len() {
        return Err(fail("proxy changed audio stream count"));
    }
    for (stream, (a, b)) in source_audio.iter().zip(output_audio).enumerate() {
        if !string(b, "codec_name").is_some_and(|s| s.starts_with("pcm_")) {
            return Err(fail("proxy audio is not PCM"));
        }
        if let Some(codec) = pcm_source_codec(a)? {
            if string(b, "codec_name") != Some(codec) {
                return Err(fail("proxy changed PCM coding or sample depth"));
            }
        }
        for key in ["sample_rate", "channels", "channel_layout"] {
            if !a[key].is_null() && a[key] != b[key] {
                return Err(ProxyError::Validation(format!("proxy changed audio {key}")));
            }
        }
        let src_tb = string(a, "time_base").and_then(parse_ratio);
        let dst_tb = string(b, "time_base").and_then(parse_ratio);
        for key in ["start_pts", "duration_ts"] {
            if let Some(x) = integer(a, key) {
                let y = integer(b, key)
                    .ok_or_else(|| ProxyError::Validation(format!("proxy lost audio {key}")))?;
                let tx = src_tb.ok_or_else(|| fail("source audio timebase missing"))?;
                let ty = dst_tb.ok_or_else(|| fail("proxy audio timebase missing"))?;
                if x * tx.0 * ty.1 != y * ty.0 * tx.1 {
                    return Err(ProxyError::Validation(format!("proxy changed audio {key}")));
                }
            }
        }
        if pcm_source_codec(a)?.is_some() {
            if audio_sample_count(ffprobe, source_path, stream)?
                != audio_sample_count(ffprobe, output_path, stream)?
            {
                return Err(fail("proxy changed PCM sample count"));
            }
            if canonical_pcm_hash(ffmpeg, source_path, stream)?
                != canonical_pcm_hash(ffmpeg, output_path, stream)?
            {
                return Err(fail("proxy changed PCM sample payload"));
            }
        }
    }
    Ok(())
}
