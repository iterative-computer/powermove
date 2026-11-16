//! The Project panel's grid thumbnails (docs/native/design/panels/
//! media-browser.md §2): a video's poster frame (v1.2.0's time: a tenth in,
//! at most half a second), a still's picture, the first frame of an image
//! sequence on a stack, an audio file's waveform. Made off the main thread,
//! two at a time, at thumbnail size, and kept under the profile
//! (`caches/media-thumbnails/<digest>.png`, the digest naming the file's
//! path, length and modification), so a project opens with them drawn.
//!
//! The panel never sees a path: an asset's runtime state carries an opaque
//! key (`thumb/<digest>`) that only the view layer resolves ([`resolve`]).
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// Thumbnail width in pixels: twice the narrowest column (`thumbnail_min`).
const WIDTH: u32 = 320;
const HEIGHT: u32 = WIDTH * 9 / 16;
/// The picture a cached file is made with: a change of recipe makes them again.
const RECIPE: &str = "powermove media thumbnail 2";
/// At most this many made at once.
const AT_ONCE: usize = 2;

fn registry() -> &'static Mutex<HashMap<String, PathBuf>> {
    static KEYS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    KEYS.get_or_init(Default::default)
}

/// The cached picture behind a key the panel drew (`thumb/<digest>`).
/// A picture the host made elsewhere (Settings' project poster) under `key`.
pub fn register(key: &str, path: PathBuf) {
    if let Ok(mut keys) = registry().lock() {
        keys.insert(key.to_owned(), path);
    }
}
pub fn resolve(key: &str) -> Option<PathBuf> {
    registry().lock().ok()?.get(key).cloned()
}

/// What a thumbnail is made from.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub asset: String,
    /// `video`, `image` or `audio` (an image sequence is a `video` whose
    /// file is its manifest).
    pub kind: String,
    pub path: PathBuf,
    /// Seconds of a video's poster frame.
    pub at: Option<f64>,
}

#[derive(Default)]
pub struct Thumbs {
    /// Asset → key of its made thumbnail.
    ready: HashMap<String, String>,
    /// Assets asked for, made or not (a failure is not retried this run).
    asked: HashSet<String>,
    queue: VecDeque<Item>,
    running: usize,
    /// A republish is due for thumbnails made since the last one.
    flush_due: bool,
    /// Bumped when a project opens: thumbnails finishing for the last one
    /// are dropped (asset ids are per project).
    generation: u64,
    /// The grid is shown somewhere: thumbnails are worth making.
    pub on: bool,
}

impl Thumbs {
    /// The key of `asset`'s thumbnail, once made.
    pub fn key(&self, asset: &str) -> Option<String> {
        self.ready.get(asset).cloned()
    }
    /// Queues the items not asked for yet; returns whether any were.
    pub fn want(&mut self, items: impl IntoIterator<Item = Item>) -> bool {
        let before = self.queue.len();
        for item in items {
            if self.asked.insert(item.asset.clone()) {
                self.queue.push_back(item);
            }
        }
        self.queue.len() > before
    }
    /// The next items to make (within [`AT_ONCE`]).
    pub fn start(&mut self) -> Vec<Item> {
        let mut out = vec![];
        while self.running < AT_ONCE
            && let Some(item) = self.queue.pop_front()
        {
            self.running += 1;
            out.push(item);
        }
        out
    }
    /// Development builds' review route: what this is doing.
    #[cfg(debug_assertions)]
    pub fn report(&self) -> String {
        format!(
            "on={} asked={} ready={} queued={} running={} generation={}",
            self.on,
            self.asked.len(),
            self.ready.len(),
            self.queue.len(),
            self.running,
            self.generation
        )
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Another project opened: forget this one's assets.
    pub fn reset(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            on: self.on,
            ..Self::default()
        };
    }
    /// A made (or failed) thumbnail, asked for in `generation`.
    pub fn done(&mut self, generation: u64, asset: String, made: Option<(String, PathBuf)>) {
        if generation != self.generation {
            return;
        }
        self.running = self.running.saturating_sub(1);
        if let Some((key, path)) = made {
            if let Ok(mut keys) = registry().lock() {
                keys.insert(key.clone(), path);
            }
            self.ready.insert(asset, key);
        }
    }
}

/// Makes `item`'s thumbnail under `dir` (or finds it made): its key and file.
pub fn make(dir: &Path, item: &Item, ffmpeg: Option<&Path>) -> Option<(String, PathBuf)> {
    let meta = std::fs::metadata(&item.path).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let mut hash = blake3::Hasher::new_derive_key(RECIPE);
    hash.update(item.path.as_os_str().as_encoded_bytes());
    hash.update(&meta.len().to_le_bytes());
    hash.update(&modified.to_le_bytes());
    hash.update(item.kind.as_bytes());
    let digest = hash.finalize().to_hex()[..32].to_owned();
    let file = dir.join(format!("{digest}.png"));
    let key = format!("thumb/{digest}");
    if file.is_file() {
        return Some((key, file));
    }
    std::fs::create_dir_all(dir).ok()?;
    let staged = dir.join(format!(".{digest}.{}.png", std::process::id()));
    let rgba = match item.kind.as_str() {
        "audio" => waveform(&item.path)?,
        _ => match pm_media::image::sequence::Manifest::read(&item.path) {
            Ok(Some(sequence)) => {
                let first = (sequence.first..=sequence.last)
                    .map(|n| sequence.file(n))
                    .find(|f| f.is_file())?;
                stacked(&still(&first, ffmpeg)?)
            }
            _ if item.kind == "video" => frame(&item.path, item.at.unwrap_or(0.), ffmpeg)?,
            _ => still(&item.path, ffmpeg)?,
        },
    };
    write_png(&staged, &rgba)?;
    std::fs::rename(&staged, &file).ok()?;
    Some((key, file))
}

/// RGBA8 at [`WIDTH`]×[`HEIGHT`].
type Picture = Vec<u8>;

/// A picture filling the frame, cropped about its centre: v1.2.0's tile
/// (`object-fit: cover`), so every card is a full 16:9 picture.
fn fitted(width: usize, height: usize, rgba: &[u8]) -> Picture {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let scale = (w as f64 / width as f64).max(h as f64 / height as f64);
    let (ox, oy) = (
        (width as f64 * scale - w as f64) / 2.,
        (height as f64 * scale - h as f64) / 2.,
    );
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let sy = ((y as f64 + oy + 0.5) / scale) as usize;
        for x in 0..w {
            let sx = ((x as f64 + ox + 0.5) / scale) as usize;
            let s = (sy.min(height - 1) * width + sx.min(width - 1)) * 4;
            out[(y * w + x) * 4..][..4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    out
}

/// A still through ImageIO's thumbnail reader (every format the app reads
/// there), else FFmpeg (DPX).
fn still(path: &Path, ffmpeg: Option<&Path>) -> Option<Picture> {
    let pool = pm_media::pool::FramePool::new(0);
    match pm_media::image::imageio::read_preview(path, &pool, WIDTH) {
        Ok(still) => {
            let (w, h) = (still.frame.format.width, still.frame.format.height);
            let unit = pm_media::image::imageio::rgba_unit(&still.frame).ok()?;
            let rgba: Vec<u8> = unit
                .iter()
                .map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
                .collect();
            Some(fitted(w, h, &rgba))
        }
        Err(_) => frame(path, 0., ffmpeg),
    }
}

/// A frame through FFmpeg, scaled to fit.
fn frame(path: &Path, at: f64, ffmpeg: Option<&Path>) -> Option<Picture> {
    let out = std::process::Command::new(ffmpeg?)
        .args(["-v", "error", "-ss", &format!("{at:.3}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!(
            "scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease"
        ))
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    // The scaled size from the byte count and the aspect FFmpeg kept.
    let pixels = out.stdout.len() / 4;
    let probe = pm_media::probe::probe(path).ok()?;
    let video = probe.probe.video.as_ref()?;
    let aspect = video.width as f64 / video.height.max(1) as f64;
    let w = ((pixels as f64 * aspect).sqrt().round() as usize).max(1);
    let h = pixels / w;
    (w * h == pixels).then(|| fitted(w, h, &out.stdout))
}

/// The first frame with two edges of the frames behind it (a sequence).
fn stacked(picture: &Picture) -> Picture {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut out = vec![0u8; w * h * 4];
    // Behind: two plates, each offset up and right by 4 px.
    for (offset, shade) in [(8usize, 70u8), (4, 110)] {
        for y in 0..h - offset {
            for x in offset..w {
                let d = (y * w + x) * 4;
                out[d..d + 4].copy_from_slice(&[shade, shade, shade, 255]);
            }
        }
    }
    let (fw, fh) = (w - 8, h - 8);
    let shrunk = fitted_into(picture, fw, fh);
    for y in 0..fh {
        for x in 0..fw {
            let s = (y * fw + x) * 4;
            if shrunk[s + 3] == 0 {
                continue;
            }
            let d = ((y + 8) * w + x) * 4;
            out[d..d + 4].copy_from_slice(&shrunk[s..s + 4]);
        }
    }
    out
}

/// [`WIDTH`]×[`HEIGHT`] RGBA scaled to `w`×`h`.
fn fitted_into(picture: &Picture, w: usize, h: usize) -> Picture {
    let (pw, ph) = (WIDTH as usize, HEIGHT as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let s = ((y * ph / h) * pw + x * pw / w) * 4;
            out[(y * w + x) * 4..][..4].copy_from_slice(&picture[s..s + 4]);
        }
    }
    out
}

/// An audio file's waveform as v1.2.0's tile draws it: a wall of bars
/// rising from the foot of the green ground, each the mean of the envelope
/// under it (a maximum over many peaks reads as a solid block for music),
/// leaving the top clear for the duration badge.
fn waveform(path: &Path) -> Option<Picture> {
    let (envelope, _) = pm_audio::waveform::clip_envelope(
        path,
        &[None],
        pm_media::clip::ClipTiming::Presentation,
        100.,
    )
    .ok()?;
    if envelope.is_empty() {
        return None;
    }
    let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
    let ground = tokens.colors.labels.green;
    let bar = tokens.colors.success;
    let rgb = |c: u32| [(c >> 16) as u8, (c >> 8) as u8, c as u8];
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let [gr, gg, gb] = rgb(ground);
    let mut out: Vec<u8> = [gr, gg, gb, 255].repeat(w * h);
    let [br, bg, bb] = rgb(bar);
    let blend = |g: u8, b: u8| ((g as f32) * 0.1 + (b as f32) * 0.9).round() as u8;
    let ink = [blend(gr, br), blend(gg, bg), blend(gb, bb), 255];
    // v1.2.0: 120 bars over the tile's width, 26 pt clear at the top of a
    // 16:9 tile (≈ 23 %) and 3 pt at the foot; each bar 3–98 % of that band.
    const BARS: usize = 120;
    let (top, foot) = (
        (h as f32 * 0.23).round() as usize,
        (h as f32 * 0.03).round() as usize,
    );
    let band = (h - top - foot) as f32;
    // Levels are absolute (0–1), as v1.2.0's peaks: a quiet file stays low.
    for bar in 0..BARS {
        let from = bar * envelope.len() / BARS;
        let to = ((bar + 1) * envelope.len() / BARS)
            .max(from + 1)
            .min(envelope.len());
        let mean = envelope[from..to].iter().sum::<f32>() / (to - from) as f32;
        let height = ((mean.clamp(0.03, 0.98) * band).round() as usize).max(1);
        let (x0, x1) = (
            bar * w / BARS,
            ((bar + 1) * w / BARS).max(bar * w / BARS + 1),
        );
        for y in h - foot - height..h - foot {
            for x in x0..x1.min(w) {
                out[(y * w + x) * 4..][..4].copy_from_slice(&ink);
            }
        }
    }
    Some(out)
}

fn write_png(path: &Path, rgba: &[u8]) -> Option<()> {
    let file = std::fs::File::create(path).ok()?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A still, a sequence and an audio file get a thumbnail at thumbnail
    /// size under the cache; a second ask finds the same file; the panel's
    /// key resolves to it only through the registry.
    #[test]
    fn media_get_a_cached_thumbnail_of_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("thumbs");
        let png_file = |path: &Path, w: u32, h: u32, rgb: [u8; 3]| {
            let file = std::fs::File::create(path).unwrap();
            let mut e = png::Encoder::new(std::io::BufWriter::new(file), w, h);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&rgb.repeat((w * h) as usize))
                .unwrap();
        };
        let still = dir.path().join("plate.png");
        png_file(&still, 64, 64, [200, 40, 40]);
        let item = Item {
            asset: "a".into(),
            kind: "image".into(),
            path: still.clone(),
            at: None,
        };
        let (key, file) = make(&cache, &item, None).unwrap();
        // Opaque: no part of the file's path.
        assert!(key.starts_with("thumb/") && !key.contains("plate"));
        let decoded =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&file).unwrap()))
                .read_info()
                .unwrap();
        assert_eq!(
            (decoded.info().width, decoded.info().height),
            (WIDTH, HEIGHT)
        );
        assert_eq!(
            make(&cache, &item, None).unwrap(),
            (key.clone(), file.clone())
        );
        let mut thumbs = Thumbs::default();
        assert!(thumbs.want([item.clone()]));
        assert!(!thumbs.want([item.clone()]), "asked once");
        assert_eq!(thumbs.start().len(), 1);
        thumbs.done(
            thumbs.generation(),
            "a".into(),
            Some((key.clone(), file.clone())),
        );
        assert_eq!(thumbs.key("a"), Some(key.clone()));
        assert_eq!(resolve(&key), Some(file.clone()));
        // Another project opened: what finishes for the last one is dropped.
        thumbs.reset();
        assert!(
            thumbs.want([item.clone()]),
            "asked again in the new project"
        );
        assert_eq!(thumbs.start().len(), 1);
        thumbs.done(
            thumbs.generation() - 1,
            "a".into(),
            Some((key.clone(), file.clone())),
        );
        assert_eq!(thumbs.key("a"), None);
        // Audio: a waveform.
        let wav = dir.path().join("tone.wav");
        let samples: Vec<u8> = (0..8000)
            .flat_map(|i| (((i as f32 * 0.1).sin() * 12000.) as i16).to_le_bytes())
            .collect();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + samples.len() as u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend([1, 0, 1, 0]);
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend([2, 0, 16, 0]);
        bytes.extend(b"data");
        bytes.extend((samples.len() as u32).to_le_bytes());
        bytes.extend(samples);
        std::fs::write(&wav, bytes).unwrap();
        let audio = Item {
            asset: "b".into(),
            kind: "audio".into(),
            path: wav,
            at: None,
        };
        assert!(make(&cache, &audio, None).is_some(), "a waveform");
        // A file that is not media gets none.
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, b"x").unwrap();
        let none = Item {
            asset: "c".into(),
            kind: "image".into(),
            path: notes,
            at: None,
        };
        assert!(make(&cache, &none, None).is_none());
    }
}

/// What `record` (an asset of the document) is made into: its kind as
/// imports write it (`kind`) or as older projects have it (`type`), and a
/// video's poster time from its length (`dur`): v1.2.0's, a tenth in, at
/// most half a second.
fn item_of(id: &str, record: &pm_model::doc::Node, path: PathBuf) -> Option<Item> {
    let text = |key: &str| {
        record
            .get(key)
            .and_then(|v| v.to_value().as_str().map(str::to_owned))
    };
    let kind = text("kind").or_else(|| text("type"))?;
    let duration = ["dur", "duration"]
        .iter()
        .find_map(|key| record.get(key).and_then(|d| d.to_value().as_f64()))
        .filter(|d| d.is_finite() && *d > 0.);
    matches!(kind.as_str(), "video" | "image" | "audio").then(|| Item {
        asset: id.to_owned(),
        at: (kind == "video").then(|| duration.map_or(0., |d| (d * 0.1).min(0.5))),
        kind,
        path,
    })
}

#[cfg(test)]
mod item_tests {
    use super::*;
    fn node(json: &str) -> pm_model::doc::Node {
        pm_model::doc::Node::from_value(pm_model::json::parse(json).unwrap(), 1)
    }
    /// Assets of both shapes get thumbnails: imported (`kind`, `dur`) and
    /// from older projects (`type`); compositions and models do not.
    #[test]
    fn assets_of_either_shape_get_a_thumbnail() {
        let p = PathBuf::from("/m/a.mov");
        let imported = item_of("a", &node(r#"{"kind":"video","dur":3}"#), p.clone()).unwrap();
        assert_eq!(imported.kind, "video");
        assert!((imported.at.unwrap() - 0.3).abs() < 1e-9);
        let older = item_of("a", &node(r#"{"type":"video","dur":20}"#), p.clone()).unwrap();
        assert_eq!(older.at, Some(0.5));
        let still = item_of("b", &node(r#"{"type":"image"}"#), p.clone()).unwrap();
        assert_eq!((still.kind.as_str(), still.at), ("image", None));
        assert!(item_of("c", &node(r#"{"type":"model"}"#), p).is_none());
    }
}

impl crate::session::Controller {
    /// The grid is shown (or the person chose it): make the open project's
    /// thumbnails from here on.
    pub(crate) fn show_thumbnails(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.thumbs.on {
            self.thumbs.on = true;
            self.want_thumbnails(cx);
        }
    }
    /// Queues the open project's media not asked for yet and starts making
    /// them, a few at a time, on the background executor. Each one made
    /// republishes the panels, so its card draws it.
    pub(crate) fn want_thumbnails(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.thumbs.on {
            return;
        }
        let Some(published) = self.published.clone() else {
            return;
        };
        let Some(pm_model::doc::NodeKind::Object(assets)) = published
            .snapshot
            .root()
            .and_then(|r| r.get("assets"))
            .map(|a| a.kind())
        else {
            return;
        };
        let items: Vec<Item> = assets
            .iter()
            .filter_map(|(id, record)| {
                let id = id.as_str()?;
                item_of(id, record, self.media_base.originals.get(id)?.clone())
            })
            .collect();
        self.thumbs.want(items);
        self.start_thumbnails(cx);
    }
    /// Thumbnails made reach the panel together, at most every quarter
    /// second: a bin of hundreds republished once per thumbnail made every
    /// frame of a scroll heavy while they were made.
    fn flush_thumbnails(&mut self, cx: &mut gpui::Context<Self>) {
        if std::mem::replace(&mut self.thumbs.flush_due, true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let _ = this.update(cx, |c, cx| {
                c.thumbs.flush_due = false;
                c.update_builtins(cx);
            });
        })
        .detach();
    }
    fn start_thumbnails(&mut self, cx: &mut gpui::Context<Self>) {
        let dir = crate::media::cache_root(&self.profile).join("media-thumbnails");
        let generation = self.thumbs.generation();
        for item in self.thumbs.start() {
            let dir = dir.clone();
            let make = cx.background_executor().spawn(async move {
                let ffmpeg = crate::pro_media::ffmpeg().map(|(ffmpeg, _)| ffmpeg);
                let made = make(&dir, &item, ffmpeg.as_deref());
                (item.asset, made)
            });
            cx.spawn(async move |this, cx| {
                let (asset, made) = make.await;
                let _ = this.update(cx, |c, cx| {
                    let drew = made.is_some();
                    c.thumbs.done(generation, asset, made);
                    c.start_thumbnails(cx);
                    if drew {
                        c.flush_thumbnails(cx);
                    }
                });
            })
            .detach();
        }
    }
}
