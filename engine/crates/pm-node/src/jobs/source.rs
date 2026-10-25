//! Source-only jobs: batch filmstrips and frozen browser renditions.

use std::fs::OpenOptions;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pm_export::render::GpuGate;
use pm_protocol::common::{ByteSize, Count, JobId};
use pm_protocol::envelope::{Request, Response};
use pm_protocol::error::ErrorCode;
use pm_protocol::host::{
    CapabilityKind, CommitOutput, HandleAccess, MintHandle, OutputVerification, PortId,
    ResolveHandle,
};
use pm_protocol::priority::{BrowserRenditionFormat, JobSpec, JobState, SubmitJob};
use pm_protocol::snapshot::{SnapshotPixels, SourceColor, SourceFit};
use pm_render::Renderer;
use pm_render::vector::svg_rasters::{SvgRasterNeed, SvgRasterSource};

use super::gate::LaneGate;
use super::{Shared, destination, job_error, note, resolve_media, status_event, status_of};
use pm_engine::jobs::{JobDocument, WorkNote};

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    shared: &Arc<Shared>,
    renderer: &mut Renderer,
    gate: &LaneGate,
    job: &JobId,
    host: PortId,
    submit: &SubmitJob,
    document: &JobDocument,
    cancel: &Arc<AtomicBool>,
) {
    shared.status(host, job, |r| {
        r.state = JobState::Running;
        r.progress = 0.0;
    });
    let media = resolve_media(shared, document);
    let token = {
        let mut st = shared.lock();
        st.next_token += 1;
        st.next_token
    };
    note(
        shared,
        WorkNote::Started {
            token,
            scopes: super::asset_scopes(&media),
        },
    );
    let session = document.session;
    let output = (|| -> Result<String, String> {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if matches!(
            &submit.spec,
            JobSpec::SourceFilmstrip {
                color: SourceColor::Compositor,
                ..
            }
        ) {
            return Err(
                "Compositor source video planes are available only in source snapshots".into(),
            );
        }
        let asset = match &submit.spec {
            JobSpec::SourceFilmstrip { asset, .. } | JobSpec::BrowserRendition { asset, .. } => {
                asset
            }
            _ => return Err("invalid source job".into()),
        };
        let original = media
            .paths
            .get(asset.as_str())
            .ok_or_else(|| "The source media is offline".to_owned())?;
        let source = if matches!(
            &submit.spec,
            JobSpec::SourceFilmstrip {
                color: SourceColor::Display,
                ..
            }
        ) {
            resolve_source_media(shared, session, asset)?
        } else {
            original.clone()
        };
        let record = pm_eval::node::member(document.document.root(), "assets")
            .and_then(|assets| pm_eval::node::member(Some(assets), asset.as_str()));
        let choice = submit
            .output
            .as_ref()
            .ok_or("The source job needs an output")?;
        let dest = destination(shared, job, session, choice, false).map_err(|e| e.message)?;
        let result = match &submit.spec {
            JobSpec::SourceFilmstrip {
                times,
                output_size,
                fit,
                color,
                ..
            } => filmstrip(
                shared,
                renderer,
                gate,
                job,
                host,
                document,
                &source,
                asset.as_str(),
                times,
                *output_size,
                *fit,
                *color,
                &dest.path,
                cancel,
            )
            .map(|bytes| (bytes, "rgba8".to_owned(), times.len() as u32)),
            JobSpec::BrowserRendition { format, .. } => browser_rendition(
                &source,
                &dest.path,
                *format,
                is_heif_asset(record, &source),
                cancel,
            )
            .map(|bytes| {
                (
                    bytes,
                    match format {
                        BrowserRenditionFormat::Png => "png",
                        BrowserRenditionFormat::Webm => "vp9",
                    }
                    .to_owned(),
                    1,
                )
            }),
            _ => unreachable!(),
        };
        let (bytes, codec, frames) = match result {
            Ok(answer) if !cancel.load(Ordering::Relaxed) => answer,
            Ok(_) => {
                discard(shared, job, session, &dest.handle);
                return Err("cancelled".into());
            }
            Err(error) => {
                discard(shared, job, session, &dest.handle);
                return Err(error);
            }
        };
        let committed = shared
            .call(
                Request::CommitOutput(CommitOutput {
                    job: job.clone(),
                    temporary: dest.handle.clone(),
                    commit: true,
                    verification: Some(OutputVerification {
                        codec,
                        frames,
                        audio_samples: Count::new(0).expect("zero fits"),
                        bytes: ByteSize::new(bytes).expect("validated output size"),
                    }),
                }),
                Some(session),
            )
            .map_err(|e| e.message)?;
        match committed {
            Response::OutputCommitted(placed) => placed
                .path
                .ok_or_else(|| "The host did not place the source output".into()),
            _ => Err("The host answered commitOutput with another response".into()),
        }
    })();
    let mut st = shared.lock();
    st.outbox.work.push(WorkNote::Ended { token });
    if let Some(record) = st.jobs.get_mut(job) {
        let (state, path, error) = match output {
            Ok(path) => (JobState::Done, Some(path), None),
            Err(_) if cancel.load(Ordering::Relaxed) => (JobState::Cancelled, None, None),
            Err(message) => (
                JobState::Failed,
                None,
                Some(job_error(job, ErrorCode::ExportFailed, message)),
            ),
        };
        record.state = state;
        if state == JobState::Done {
            record.progress = 1.0;
        }
        let status = status_of(job, record, path, error);
        st.outbox.messages.push((host, status_event(status)));
        st.retire(job.clone());
    }
    drop(st);
    (shared.wake)();
}

fn discard(
    shared: &Shared,
    job: &JobId,
    session: pm_protocol::common::SessionId,
    handle: &pm_protocol::host::CapabilityHandle,
) {
    let _ = shared.call(
        Request::CommitOutput(CommitOutput {
            job: job.clone(),
            temporary: handle.clone(),
            commit: false,
            verification: None,
        }),
        Some(session),
    );
}

fn resolve_source_media(
    shared: &Shared,
    session: pm_protocol::common::SessionId,
    asset: &pm_protocol::common::AssetId,
) -> Result<PathBuf, String> {
    let minted = shared
        .call(
            Request::MintHandle(MintHandle {
                kind: CapabilityKind::SourceRead {
                    asset: asset.clone(),
                },
                session,
            }),
            Some(session),
        )
        .map_err(|e| e.message)?;
    let Response::Handle(grant) = minted else {
        return Err("The host did not grant source media".into());
    };
    let resolved = shared
        .call(
            Request::ResolveHandle(ResolveHandle {
                handle: grant.handle,
                session,
            }),
            Some(session),
        )
        .map_err(|e| e.message)?;
    match resolved {
        Response::HandleResolved(answer)
            if answer.access == HandleAccess::Read
                && matches!(answer.kind, CapabilityKind::SourceRead { asset: ref a } if a == asset) =>
        {
            Ok(PathBuf::from(answer.path))
        }
        _ => Err("The host did not resolve source media".into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn filmstrip(
    shared: &Arc<Shared>,
    renderer: &mut Renderer,
    gate: &LaneGate,
    job: &JobId,
    host: PortId,
    document: &JobDocument,
    source: &Path,
    asset: &str,
    times: &[pm_protocol::common::Ticks],
    size: pm_protocol::common::Size,
    fit: SourceFit,
    color: SourceColor,
    output: &Path,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    let record = pm_eval::node::member(document.document.root(), "assets")
        .and_then(|assets| pm_eval::node::member(Some(assets), asset));
    let interpretation = if color == SourceColor::Interpreted {
        record.and_then(pm_render::footage::interpretation)
    } else {
        None
    };
    let sequence = record.is_some_and(pm_render::footage::is_sequence);
    let times: Vec<_> = times
        .iter()
        .map(|&time| pm_render::footage::source_time(record, time, interpretation.as_ref()))
        .collect();
    let svg = if Renderer::source_is_svg(source)? {
        if interpretation.is_some() {
            return Err(
                "A source SVG interpretation cannot be applied to the Chromium raster".into(),
            );
        }
        let intrinsic = (
            pm_eval::node::finite_number(pm_eval::node::member(record, "w")).unwrap_or(0.0),
            pm_eval::node::finite_number(pm_eval::node::member(record, "h")).unwrap_or(0.0),
        );
        let request = Renderer::source_svg_request(source, asset, size, fit, intrinsic)?;
        if !renderer.has_svg_raster(&request.key) {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let rasters = super::svg::HostSvgRasters::new(Arc::clone(shared), document);
            let need = SvgRasterNeed {
                request: request.clone(),
                path: source.to_owned(),
                composition: "source".into(),
                layer: asset.into(),
            };
            let pixels = rasters.fetch(&need)?;
            renderer.put_svg_raster(request.key.clone(), pixels)?;
        }
        Some(request.key)
    } else {
        None
    };
    let mut file = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(output)
            .map_err(|e| e.to_string())?,
    );
    let mut bytes = 0u64;
    let mut index = 0usize;
    let pass = 0x534f_5552_4345_u64 ^ u64::from(size.width) ^ (u64::from(size.height) << 32);
    while index < times.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        // The gate may grant fewer frames during playback, down to one.
        // Bound idle submissions too, so a higher-priority waiter gets its
        // turn between short chunks rather than after an entire filmstrip.
        let limit = if svg.is_some() {
            1
        } else {
            Renderer::source_batch_limit(size)
        };
        let requested = limit.min(times.len() - index) as u32;
        let admitted = gate.admit(pass, requested, cancel).ok_or("cancelled")?;
        let count = admitted.min(requested) as usize;
        let started = Instant::now();
        let frames = if let Some(key) = &svg {
            renderer
                .source_svg_rgba8(key, size, fit, interpretation.as_ref())
                .and_then(|pixels| {
                    pixels.ok_or_else(|| {
                        "The source SVG raster was evicted before drawing".to_owned()
                    })
                })
                .map(|pixels| vec![pixels])
        } else if count == 1 {
            let frame = if times.len() == 1 && !gate.gate.is_playing() {
                // A real one-frame preparation also warms the same exact
                // Export decoder/cache used by interactive source seeks.
                // A playback slice of a larger strip remains direct: its
                // CPU look-ahead must not escape the job's admission.
                renderer.source_rgba8(
                    source,
                    times[index],
                    size,
                    fit,
                    interpretation.as_ref(),
                    sequence,
                )
            } else {
                renderer.source_rgba8_export(
                    source,
                    times[index],
                    size,
                    fit,
                    interpretation.as_ref(),
                    sequence,
                )
            };
            frame.map(|pixels| vec![pixels])
        } else {
            renderer.source_rgba8_batch(
                source,
                &times[index..index + count],
                size,
                fit,
                interpretation.as_ref(),
                sequence,
            )
        };
        gate.finished(pass, count as u32, started.elapsed().as_micros() as f64);
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        for (offset, pixels) in frames?.into_iter().enumerate() {
            let pixels = if svg.is_some() {
                pixels
            } else {
                straight_source_pixels(pixels)
            };
            file.write_all(&pixels).map_err(|e| e.to_string())?;
            bytes += pixels.len() as u64;
            shared.status(host, job, |r| {
                r.progress = ((index + offset + 1) as f64 / times.len() as f64).min(0.999)
            });
        }
        index += count;
    }
    file.flush().map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Video pixels are ordinarily opaque, so their premultiplied and straight
/// encodings are byte-identical. Avoid a second full frame allocation and
/// float conversion; retain the existing Canvas2D unpremultiplication for
/// any source with alpha.
fn straight_source_pixels(pixels: Vec<u8>) -> Vec<u8> {
    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 255) {
        pixels
    } else {
        crate::present::snapshot::encode_rgba8_unorm(&pixels, SnapshotPixels::Straight)
    }
}

fn ffmpeg() -> PathBuf {
    std::env::var_os("POWERMOVE_ENGINE_FFMPEG")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("PM_FFMPEG_DIR").map(|root| PathBuf::from(root).join("bin/ffmpeg"))
        })
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

fn is_heif_asset(record: Option<&pm_model::Node>, source: &Path) -> bool {
    let marked = ["format", "name"].iter().any(|field| {
        pm_eval::node::string(pm_eval::node::member(record, field))
            .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
            .is_some_and(|value| {
                let value = value.to_ascii_lowercase();
                value == "heic"
                    || value == "heif"
                    || value.ends_with(".heic")
                    || value.ends_with(".heif")
            })
    });
    marked
        || source
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("heic") || ext.eq_ignore_ascii_case("heif"))
}

fn browser_rendition(
    source: &Path,
    output: &Path,
    format: BrowserRenditionFormat,
    heif: bool,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    let mut command;
    match format {
        BrowserRenditionFormat::Webm => {
            command = Command::new(ffmpeg());
            command
                .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
                .arg(source)
                .args([
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0?",
                    "-c:v",
                    "libvpx-vp9",
                    "-pix_fmt",
                    "yuva420p",
                    "-b:v",
                    "0",
                    "-crf",
                    "18",
                    "-g",
                    "15",
                    "-deadline",
                    "good",
                    "-cpu-used",
                    "4",
                    "-row-mt",
                    "1",
                    "-threads",
                    "4",
                    "-c:a",
                    "libopus",
                    "-b:a",
                    "192k",
                    "-f",
                    "webm",
                    "-y",
                ])
                .arg(output);
        }
        BrowserRenditionFormat::Png if heif => {
            command = Command::new("/usr/bin/sips");
            command
                .args(["-s", "format", "png"])
                .arg(source)
                .arg("--out")
                .arg(output);
        }
        BrowserRenditionFormat::Png => {
            command = Command::new(ffmpeg());
            command
                .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
                .arg(source)
                .args([
                    "-frames:v",
                    "1",
                    "-update",
                    "1",
                    "-c:v",
                    "png",
                    "-f",
                    "image2",
                    "-y",
                ])
                .arg(output);
        }
    }
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start browser rendition converter: {e}"))?;
    let stderr = child.stderr.take().expect("converter stderr is piped");
    let reader = std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut tail = Vec::new();
        let mut chunk = [0u8; 8192];
        while let Ok(n) = stderr.read(&mut chunk) {
            if n == 0 {
                break;
            }
            tail.extend_from_slice(&chunk[..n]);
            if tail.len() > 1 << 20 {
                tail.drain(..tail.len() - (1 << 20));
            }
        }
        tail
    });
    let deadline = Instant::now()
        + match format {
            BrowserRenditionFormat::Webm => Duration::from_secs(60 * 60),
            BrowserRenditionFormat::Png => Duration::from_secs(2 * 60),
        };
    loop {
        if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(if cancel.load(Ordering::Relaxed) {
                "cancelled"
            } else {
                "Browser rendition timed out"
            }
            .into());
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                let stderr = reader.join().unwrap_or_default();
                let detail = String::from_utf8_lossy(&stderr);
                return Err(format!(
                    "Browser rendition converter exited with {status}: {}",
                    detail.trim()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(error.to_string());
            }
        }
    }
    let _ = reader.join();
    let bytes = std::fs::metadata(output).map_err(|e| e.to_string())?.len();
    if bytes == 0 {
        return Err("The browser rendition was empty".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_opaque_fast_path_and_alpha_match_snapshot_encoding() {
        let opaque = vec![0, 45, 255, 255, 138, 100, 21, 255];
        assert_eq!(straight_source_pixels(opaque.clone()), opaque);
        let alpha = vec![128, 0, 0, 128, 10, 20, 30, 0, 0, 0, 0, 255];
        assert_eq!(
            straight_source_pixels(alpha.clone()),
            crate::present::snapshot::encode_rgba8_unorm(&alpha, SnapshotPixels::Straight)
        );
    }

    fn run(command: &mut Command) {
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn decoded(path: &Path, stream: &str) -> Vec<u8> {
        let mut command = Command::new(ffmpeg());
        command
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-map", stream]);
        if stream == "0:v:0" {
            command.args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"]);
        } else {
            command.args(["-ac", "2", "-ar", "48000", "-f", "s16le", "-"]);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
        output.stdout
    }

    #[test]
    fn copied_heif_uses_record_format_when_store_path_has_no_extension() {
        let record = pm_model::Node::from_value(
            pm_model::json::parse(r#"{"name":"IMG_0001.HEIC","format":"heic"}"#).unwrap(),
            1,
        );
        assert!(is_heif_asset(
            Some(&record),
            Path::new("/store/objects/4f0a")
        ));
        let ordinary = pm_model::Node::from_value(
            pm_model::json::parse(r#"{"name":"photo.tiff","format":"tiff"}"#).unwrap(),
            1,
        );
        assert!(!is_heif_asset(
            Some(&ordinary),
            Path::new("/store/objects/4f0a")
        ));
    }

    #[test]
    fn webm_rendition_decodes_identically_to_legacy_playback_converter() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mov");
        run(Command::new(ffmpeg())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x48:rate=10:duration=0.6",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:sample_rate=48000:duration=0.6",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "mpeg4",
                "-c:a",
                "pcm_s16le",
                "-f",
                "mov",
                "-y",
            ])
            .arg(&source));
        let native = dir.path().join("native.webm");
        let legacy = dir.path().join("legacy.webm");
        browser_rendition(
            &source,
            &native,
            BrowserRenditionFormat::Webm,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        run(Command::new(ffmpeg())
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(&source)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-c:v",
                "libvpx-vp9",
                "-pix_fmt",
                "yuva420p",
                "-b:v",
                "0",
                "-crf",
                "18",
                "-g",
                "15",
                "-deadline",
                "good",
                "-cpu-used",
                "4",
                "-row-mt",
                "1",
                "-threads",
                "4",
                "-c:a",
                "libopus",
                "-b:a",
                "192k",
                "-f",
                "webm",
                "-y",
            ])
            .arg(&legacy));
        assert_eq!(decoded(&native, "0:v:0"), decoded(&legacy, "0:v:0"));
        assert_eq!(decoded(&native, "0:a:0"), decoded(&legacy, "0:a:0"));
    }

    #[test]
    fn extensionless_tiff_png_rendition_decodes_like_legacy_converter() {
        let dir = tempfile::tempdir().unwrap();
        let named = dir.path().join("source.tiff");
        run(Command::new(ffmpeg())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=8x8:rate=1:duration=1",
                "-frames:v",
                "1",
                "-c:v",
                "tiff",
                "-f",
                "image2",
                "-y",
            ])
            .arg(&named));
        let source = dir.path().join("content-hash-without-extension");
        std::fs::copy(&named, &source).unwrap();
        let native = dir.path().join("native.png");
        let legacy = dir.path().join("legacy.png");
        browser_rendition(
            &source,
            &native,
            BrowserRenditionFormat::Png,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        run(Command::new(ffmpeg())
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(&named)
            .args([
                "-frames:v",
                "1",
                "-update",
                "1",
                "-c:v",
                "png",
                "-f",
                "image2",
                "-y",
            ])
            .arg(&legacy));
        assert_eq!(
            image::open(&native).unwrap().to_rgba8(),
            image::open(&legacy).unwrap().to_rgba8()
        );
    }

    #[test]
    fn extensionless_heic_png_rendition_decodes_like_legacy_sips() {
        let dir = tempfile::tempdir().unwrap();
        let initial = dir.path().join("initial.png");
        run(Command::new(ffmpeg())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=8x8:rate=1:duration=1",
                "-frames:v",
                "1",
                "-c:v",
                "png",
                "-f",
                "image2",
                "-y",
            ])
            .arg(&initial));
        let named = dir.path().join("source.heic");
        run(Command::new("/usr/bin/sips")
            .args(["-s", "format", "heic"])
            .arg(&initial)
            .arg("--out")
            .arg(&named));
        let source = dir.path().join("content-hash-without-extension");
        std::fs::copy(&named, &source).unwrap();
        let native = dir.path().join("native.png");
        let legacy = dir.path().join("legacy.png");
        browser_rendition(
            &source,
            &native,
            BrowserRenditionFormat::Png,
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        run(Command::new("/usr/bin/sips")
            .args(["-s", "format", "png"])
            .arg(&named)
            .arg("--out")
            .arg(&legacy));
        assert_eq!(
            image::open(&native).unwrap().to_rgba8(),
            image::open(&legacy).unwrap().to_rgba8()
        );
    }
}
