//! Home's poster frames: one still of each project, kept under the profile
//! (`caches/thumbnails/<project>.png` with a `.stamp`, the time it was
//! taken, and the time in the composition it shows). A poster is written
//! after a save, and when Home shows the project this window has open with
//! none or an older one. It is rendered off the main thread by an engine job
//! of the open project, with the Media Offline card where media is missing.
//! A project that was never saved or shown on Home from this app has none:
//! there is no way yet to draw a project that is not open.
use pm_compat_runner::{FrameRenderer, FrameRequest};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Width of a stored poster frame; its height follows the composition.
const WIDTH: u32 = 640;
/// Where a poster is looked for when the viewer kept no picture with
/// something in it, as shares of the composition's length, from the start
/// on: the first frame that shows something is kept (PM-285).
const PLACES: [f64; 7] = [0., 1. / 12., 1. / 6., 1. / 3., 1. / 2., 2. / 3., 5. / 6.];
/// Channel values a frame must span before it counts as showing something.
const CONTENT_SPAN: u8 = 24;

pub(crate) fn files(dir: &Path, id: &str) -> (PathBuf, PathBuf) {
    let stem: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    (
        dir.join(format!("{stem}.png")),
        dir.join(format!("{stem}.stamp")),
    )
}
/// The last frame `id` showed while it was open, when one is stored and
/// shows something, as its poster (written beside the others, stamped
/// `stamp`): Home shows what the person last saw rather than drawing it
/// again. A flat one (the viewer's ground while the project loaded) is
/// never taken. Blocking.
pub(crate) fn adopt_last_frame(profile: &Path, dir: &Path, id: &str, stamp: i64) -> Option<Vec<u8>> {
    let (last, last_stamp) = last_frame_files(profile, id);
    let png = std::fs::read(&last).ok().filter(|png| has_content(png))?;
    let time: f64 = std::fs::read_to_string(&last_stamp)
        .ok()
        .and_then(|s| s.lines().nth(1).and_then(|t| t.trim().parse().ok()))
        .unwrap_or(0.);
    let (file, stamp_file) = files(dir, id);
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&file, &png).ok()?;
    std::fs::write(&stamp_file, format!("{stamp}\n{time}\n{}\n{LAYERS}", identity())).ok()?;
    let _ = std::fs::remove_file(missing_file(dir, id));
    Some(png)
}
/// A poster's fourth stamp line: whether the project has a layer that can
/// show (`layers`) or none at all (`blank`). Only `blank` lets Home's card
/// say Blank: a project with any visible layer never does, whatever its
/// picture looks like.
const LAYERS: &str = "layers";
const BLANK: &str = "blank";
/// Whether `published` has any layer that can show: switched on and not
/// sound, a null, a camera, a light or an adjustment, in any composition.
pub(crate) fn shows_layers(published: &pm_kernel::Published) -> bool {
    let Some(root) = published.snapshot.root() else {
        return false;
    };
    let shows = |layer: &pm_model::Node| {
        let on = !matches!(layer.get("on").map(pm_model::Node::kind), Some(pm_model::NodeKind::Bool(false)));
        let kind = match layer.get("type").map(pm_model::Node::kind) {
            Some(pm_model::NodeKind::String(s)) => s.as_str(),
            _ => None,
        };
        on && !matches!(kind, Some("audio" | "null" | "camera" | "light" | "adjustment"))
    };
    let comps: Vec<&pm_model::Node> = match root.get("comps").map(pm_model::Node::kind) {
        Some(pm_model::NodeKind::Object(comps)) => comps.iter().map(|(_, comp)| comp).collect(),
        _ => vec![],
    };
    std::iter::once(root)
        .chain(comps)
        .any(|comp| pm_kernel::model::layers_of(comp).any(shows))
}
/// Whether the poster stamped in `stamp_file` is of a project with no layer
/// that can show; `None` when the stamp does not say.
pub(crate) fn blank(stamp_file: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(stamp_file).ok()?;
    match text.lines().nth(3).map(str::trim) {
        Some(BLANK) => Some(true),
        Some(LAYERS) => Some(false),
        _ => None,
    }
}
/// Marks a poster that left offline media out (Home's card says so).
pub(crate) fn missing_file(dir: &Path, id: &str) -> PathBuf {
    files(dir, id).0.with_extension("missing")
}
/// A trashed project's poster is removed; Put Back draws it again when needed.
pub(crate) fn forget(dir: &Path, id: &str) {
    let (image, stamp) = files(dir, id);
    let _ = std::fs::remove_file(missing_file(dir, id));
    let _ = std::fs::remove_file(image);
    let _ = std::fs::remove_file(stamp);
}
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
/// The open composition's size and length.
pub(crate) fn frame_of(published: &pm_kernel::Published) -> ((u32, u32), Option<f64>) {
    let comp = published.snapshot.root().map(|root| {
        let open = published.snapshot.open_comp();
        pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root)
    });
    let number = |key: &str| comp.and_then(|c| c.get(key)).and_then(crate::node_number);
    (
        (
            number("w").map_or(1920, |w| w.max(2.) as u32),
            number("h").map_or(1080, |h| h.max(2.) as u32),
        ),
        number("dur"),
    )
}
/// The times to try for a composition `duration` seconds long, in order:
/// where the last poster was found first (`kept`), so a project that is
/// saved again costs one frame, not a search.
fn times(duration: Option<f64>, kept: Option<f64>) -> Vec<f64> {
    let Some(d) = duration.filter(|d| d.is_finite() && *d > 0.) else {
        return vec![0., 1.];
    };
    let kept = kept.filter(|at| at.is_finite() && *at >= 0. && *at < d);
    kept.into_iter()
        .chain(PLACES.iter().map(|place| d * place))
        .collect()
}
/// The stamp of a poster drawn by another build of the app: older than any
/// change to a project, so the poster is shown until it can be drawn again.
pub(crate) const STALE: i64 = -1;
/// What drew a poster: the app's version and build. A poster from another
/// build may not be what this one's renderer draws (an effect fixed, a
/// colour pipeline changed), so it is not fresh whatever its date.
fn identity() -> String {
    let build = objc2_foundation::NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&objc2_foundation::NSString::from_str("CFBundleVersion"))
        .and_then(|v| v.downcast::<objc2_foundation::NSString>().ok())
        .map(|v| v.to_string())
        .unwrap_or_default();
    // The poster's own format: posters drawn before offline media was left
    // out (the Media Offline card) are drawn again.
    format!("{} ({build}) {FORMAT}", crate::updates::current_version())
}
/// The poster format this build draws: offline media left out, the
/// viewer's last picture or the first frame with something in it, and
/// whether the project has a layer that can show (PM-285).
const FORMAT: &str = "posters 3";
/// Whether a stored poster is in this build's format: one in an older
/// format may show the Media Offline card, so it is never shown, only
/// drawn again.
pub(crate) fn current_format(stamp_file: &Path) -> bool {
    std::fs::read_to_string(stamp_file)
        .is_ok_and(|text| text.lines().nth(2).is_some_and(|by| by.trim().ends_with(FORMAT)))
}
/// A stored poster's stamp and the time in the composition it shows. The
/// stamp is [`STALE`] when another build drew it.
pub(crate) fn stamp(stamp_file: &Path) -> Option<(i64, Option<f64>)> {
    stamp_by(stamp_file, &identity())
}
fn stamp_by(stamp_file: &Path, identity: &str) -> Option<(i64, Option<f64>)> {
    let text = std::fs::read_to_string(stamp_file).ok()?;
    let mut lines = text.lines();
    let stamp: i64 = lines.next()?.trim().parse().ok()?;
    let at = lines.next().and_then(|at| at.trim().parse().ok());
    let same = lines.next().is_some_and(|by| by.trim() == identity);
    Some((if same { stamp } else { STALE }, at))
}
/// A stored picture decoded to straight RGBA with its size, to show from
/// the first frame it is laid out in. Blocking (a file read and a decode).
pub(crate) fn decode(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((info.width, info.height, rgba))
}
/// Whether a frame shows anything: a flat frame (all black, or one colour
/// from edge to edge) does not.
pub(crate) fn has_content(png: &[u8]) -> bool {
    let mut reader = match png::Decoder::new(std::io::Cursor::new(png)).read_info() {
        Ok(reader) => reader,
        Err(_) => return false,
    };
    let Some(size) = reader.output_buffer_size() else {
        return false;
    };
    let mut pixels = vec![0; size];
    let Ok(info) = reader.next_frame(&mut pixels) else {
        return false;
    };
    let channels = info.color_type.samples();
    // Colour channels only: alpha is the same everywhere in an opaque frame.
    let colour = channels.min(3);
    let (mut low, mut high) = ([u8::MAX; 3], [u8::MIN; 3]);
    for pixel in pixels[..info.buffer_size()].chunks_exact(channels) {
        for c in 0..colour {
            low[c] = low[c].min(pixel[c]);
            high[c] = high[c].max(pixel[c]);
        }
    }
    (0..colour).any(|c| high[c].saturating_sub(low[c]) >= CONTENT_SPAN)
}
/// Renders `id`'s poster and stores it with `stamp`. Blocking: call it off
/// the main thread. `published` is the project as it stood when the poster
/// was asked for; `None` draws the kernel's project now. Returns the PNG
/// that was stored.
pub(crate) fn write(
    renderer: &FrameRenderer,
    published: Option<Arc<pm_kernel::Published>>,
    dir: &Path,
    id: &str,
    stamp: i64,
    (w, h): (u32, u32),
    duration: Option<f64>,
) -> Option<Vec<u8>> {
    fn never() -> bool {
        false
    }
    // What the person last saw of it, when the viewer kept a picture with
    // something in it, before any frame is drawn.
    if let Some(profile) = dir.parent().and_then(Path::parent)
        && let Some(png) = adopt_last_frame(profile, dir, id, stamp)
    {
        return Some(png);
    }
    let shows = published.as_deref().map(shows_layers);
    let duration = duration.or_else(|| published.as_deref().and_then(|p| frame_of(p).1));
    let height = (WIDTH as f64 * h as f64 / w.max(1) as f64).round() as u32;
    let (file, stamp_file) = files(dir, id);
    let kept = self::stamp(&stamp_file).and_then(|(_, at)| at);
    let mut first = None;
    let mut chosen = None;
    for time in times(duration, kept) {
        let png = match renderer(&FrameRequest {
            time,
            width: WIDTH,
            height: height.clamp(90, 2 * WIDTH),
            motion_blur: false,
            stop: &never,
            published: published.clone(),
        }) {
            Ok(png) => png,
            Err(error) => {
                crate::diag::line(serde_json::json!({"poster": id, "time": time, "render": error.to_string()}));
                continue;
            }
        };
        if has_content(&png) {
            chosen = Some((time, png));
            break;
        }
        first.get_or_insert((time, png));
    }
    // A composition that is blank wherever it was looked at keeps its
    // blank frame: that is what it looks like.
    let (time, png) = chosen.or(first)?;
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&file, &png).ok()?;
    let what = match shows {
        Some(false) => BLANK,
        Some(true) => LAYERS,
        None => "",
    };
    std::fs::write(&stamp_file, format!("{stamp}\n{time}\n{}\n{what}", identity())).ok()?;
    let missing = missing_file(dir, id);
    if crate::mcp_services::poster_missed_media(id) {
        let _ = std::fs::write(&missing, "");
    } else {
        let _ = std::fs::remove_file(&missing);
    }
    Some(png)
}

/// Width of a stored last frame: it stands in the viewer, larger than a card.
const LAST_WIDTH: u32 = 1280;
/// The picture the viewer last showed for a project: copied from the
/// viewer when its picture holds still, after a save, at a close and
/// before a chosen quit, kept under the profile
/// (`caches/last-frame/<project>.png`, its `.stamp` naming the revision and
/// time it shows). A launch shows it in the viewer until the project's first
/// real frame (PM-384). Home's card keeps its own poster.
pub(crate) fn last_frame_files(profile: &Path, id: &str) -> (PathBuf, PathBuf) {
    files(&profile.join("caches").join("last-frame"), id)
}
/// What a last frame shows: the project's revision and the playhead's time,
/// by this build.
pub(crate) fn last_frame_mark(revision: u64, time: f64) -> String {
    format!("{revision}\n{time:.4}\n{}", identity())
}
/// The picture that stands in for `id` while it opens: the frame its viewer
/// last showed, else its Home card's poster; none when neither was ever
/// stored (PM-384).
pub(crate) fn opening_poster(profile: &Path, id: &str) -> Option<PathBuf> {
    [
        last_frame_files(profile, id).0,
        files(&profile.join("caches").join("thumbnails"), id).0,
    ]
    .into_iter()
    .find(|poster| poster.is_file())
}
/// How long the viewer's picture must hold still before it is kept as the
/// last frame: a kill or a signal quit keeps nothing, so the picture a
/// launch opens on is the last one that held this long.
pub(crate) const LAST_FRAME_STILL: std::time::Duration = std::time::Duration::from_secs(4);
/// The least time between two pictures kept that way.
pub(crate) const LAST_FRAME_GAP: std::time::Duration = std::time::Duration::from_secs(10);
/// What the still picture's watch does next.
#[derive(Debug, PartialEq)]
pub(crate) enum Still {
    /// Look again after this long.
    Wait(std::time::Duration),
    /// Keep it now.
    Store,
}
/// The still picture's watch, given how long the picture has held, how long
/// since the last one was kept (if one was), whether one is still being
/// written, and whether the picture is moving or still loading.
pub(crate) fn still(
    held: std::time::Duration,
    since_stored: Option<std::time::Duration>,
    writing: bool,
    busy: bool,
) -> Still {
    if held < LAST_FRAME_STILL {
        return Still::Wait(LAST_FRAME_STILL - held);
    }
    if writing || busy {
        return Still::Wait(LAST_FRAME_STILL);
    }
    match since_stored {
        Some(since) if since < LAST_FRAME_GAP => Still::Wait(LAST_FRAME_GAP - since),
        _ => Still::Store,
    }
}
/// The watch over the picture the viewer shows, for the last frame.
#[derive(Default)]
pub(crate) struct StillWatch {
    /// The picture shown (its revision, time and whether it settled) and
    /// since when.
    pub shown: Option<((u64, i64, bool), std::time::Instant)>,
    /// A watch is waiting for the picture to hold.
    pub watching: bool,
    /// A picture is being written.
    pub writing: bool,
    /// When the last one was kept.
    pub stored: Option<std::time::Instant>,
}
/// Whether the stored last frame already shows `mark`.
pub(crate) fn last_frame_current(profile: &Path, id: &str, mark: &str) -> bool {
    let (image, stamp) = last_frame_files(profile, id);
    image.is_file() && std::fs::read_to_string(stamp).is_ok_and(|s| s == mark)
}
/// Stores `picture`, what the viewer presented, as `id`'s last frame with
/// `mark`: the engine's held picture read and written, no render. Blocking:
/// call it off the main thread. Whether it was stored.
pub(crate) fn write_last_frame(
    picture: &pm_engine_service::Picture,
    profile: &Path,
    id: &str,
    mark: &str,
) -> bool {
    let Some((width, height, rgba)) = picture.srgb8(LAST_WIDTH) else {
        return false;
    };
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_compression(png::Compression::Fast);
        let Ok(mut writer) = encoder.write_header() else {
            return false;
        };
        if writer.write_image_data(&rgba).is_err() {
            return false;
        }
    }
    let (image, stamp) = last_frame_files(profile, id);
    let Some(dir) = image.parent() else {
        return false;
    };
    // Written beside and renamed over: a launch never reads half a picture.
    let partial = image.with_extension("png.partial");
    std::fs::create_dir_all(dir).is_ok()
        && std::fs::write(&partial, &png).is_ok()
        && std::fs::rename(&partial, &image).is_ok()
        && std::fs::write(&stamp, mark).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(paint: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let (w, h) = (16, 9);
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                pixels.extend(paint(x, y));
            }
        }
        writer.write_image_data(&pixels).unwrap();
        drop(writer);
        out
    }

    /// A last frame is current only for the revision and time it shows; a
    /// close or save with another one keeps it again (PM-384).
    #[test]
    fn a_last_frame_is_current_for_its_revision_and_time() {
        let profile = std::env::temp_dir().join(format!("pm-last-frame-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        let mark = last_frame_mark(7, 12.04);
        assert!(!last_frame_current(&profile, "file-1", &mark), "none stored yet");
        let (image, stamp) = last_frame_files(&profile, "file-1");
        std::fs::create_dir_all(image.parent().unwrap()).unwrap();
        std::fs::write(&image, b"png").unwrap();
        std::fs::write(&stamp, &mark).unwrap();
        assert!(image.ends_with("caches/last-frame/file_1.png"));
        assert!(last_frame_current(&profile, "file-1", &mark));
        assert!(!last_frame_current(&profile, "file-1", &last_frame_mark(8, 12.04)));
        assert!(!last_frame_current(&profile, "file-1", &last_frame_mark(7, 3.)));
        std::fs::remove_dir_all(&profile).unwrap();
    }

    /// An opening project stands in with the frame its viewer last showed,
    /// else its Home card's picture, never nothing when either exists.
    #[test]
    fn an_opening_project_shows_its_last_frame_else_its_home_poster() {
        let profile = tempfile::tempdir().unwrap();
        let profile = profile.path();
        assert_eq!(opening_poster(profile, "file-1"), None, "none ever stored");
        let (card, _) = files(&profile.join("caches/thumbnails"), "file-1");
        std::fs::create_dir_all(card.parent().unwrap()).unwrap();
        std::fs::write(&card, b"png").unwrap();
        assert_eq!(opening_poster(profile, "file-1"), Some(card.clone()));
        let (last, _) = last_frame_files(profile, "file-1");
        std::fs::create_dir_all(last.parent().unwrap()).unwrap();
        std::fs::write(&last, b"png").unwrap();
        assert_eq!(opening_poster(profile, "file-1"), Some(last));
        assert_eq!(opening_poster(profile, "file-2"), None, "another project's is not its");
    }

    /// A picture that held still is kept, so a launch after a kill or a
    /// signal quit (which store nothing) opens on it; a moving or loading
    /// picture waits, and writes stay one at a time and spaced.
    #[test]
    fn a_still_picture_is_kept_once_it_holds() {
        use std::time::Duration;
        let s = Duration::from_secs;
        assert_eq!(still(s(1), None, false, false), Still::Wait(s(3)));
        assert_eq!(still(s(4), None, false, false), Still::Store);
        assert_eq!(still(s(9), Some(s(60)), false, false), Still::Store);
        // Playing, settling or waiting on its effects: looked at again.
        assert_eq!(still(s(9), None, false, true), Still::Wait(LAST_FRAME_STILL));
        assert_eq!(still(s(9), None, true, false), Still::Wait(LAST_FRAME_STILL));
        assert_eq!(still(s(9), Some(s(4)), false, false), Still::Wait(s(6)));
    }

    #[test]
    fn a_flat_frame_shows_nothing_and_a_picture_does() {
        assert!(!has_content(&frame(|_, _| [0, 0, 0, 255])));
        assert!(!has_content(&frame(|_, _| [40, 90, 200, 255])));
        // Compression noise on a flat ground is still nothing.
        assert!(!has_content(&frame(|x, _| [
            10 + (x % 2) as u8 * 6,
            10,
            10,
            255
        ])));
        assert!(has_content(&frame(|x, _| [x as u8 * 12, 0, 0, 255])));
        assert!(!has_content(b"not a png"));
    }

    #[test]
    fn only_a_poster_in_this_format_is_shown() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            super::current_format(&path)
        };
        // Older posters (no format, or the first) may hold Media Offline.
        assert!(!at("a.stamp", "12\n1"));
        assert!(!at("b.stamp", "12\n1\n0.1.0 (1)"));
        // A poster from before the viewer's picture came first may be a
        // flat frame of a project with layers (PM-285): drawn again.
        assert!(!at("c.stamp", "12\n1\n0.1.0 (1) posters 2"));
        assert!(at("c3.stamp", "12\n1\n0.1.0 (1) posters 3\nlayers"));
        assert!(at("d.stamp", "12\n1\n0.2.0 (7) posters 3"), "another build's, same format");
        assert!(!super::current_format(&dir.path().join("none.stamp")));
    }
    #[test]
    fn a_poster_is_looked_for_from_the_start_on() {
        let at = times(Some(60.), None);
        assert_eq!(at, [0., 5., 10., 20., 30., 40., 50.]);
        // Its length unknown: the start, then a second in.
        assert_eq!(times(None, None), [0., 1.]);
        assert_eq!(times(Some(0.), Some(3.)), [0., 1.]);
        // Where the last poster was found comes first, while it still exists.
        assert_eq!(times(Some(60.), Some(25.))[..2], [25., 0.]);
        assert_eq!(times(Some(4.), Some(5.))[0], 0.);
    }

    fn published(layers: serde_json::Value) -> pm_kernel::Published {
        let project = serde_json::json!({"id": "P1", "name": "P", "w": 1920, "h": 1080, "fps": 24, "dur": 60,
            "bg": "#232323", "layers": layers, "comps": {}, "assets": {}, "markers": [], "params": {}});
        pm_kernel::detached_publication("P1", pm_model::json::parse(&project.to_string()).unwrap())
    }

    /// Blank is what the project holds, not what a frame looks like: a
    /// project with a layer that can show is never blank (PM-285).
    #[test]
    fn a_project_with_a_layer_that_shows_is_never_blank() {
        let layer = |kind: &str, on: bool| {
            serde_json::json!({"id": format!("L{kind}"), "name": kind, "type": kind, "on": on,
                "from": 0, "dur": 10, "parent": null})
        };
        assert!(!shows_layers(&published(serde_json::json!([]))), "no layers");
        assert!(!shows_layers(&published(serde_json::json!([layer("audio", true)]))), "sound only");
        assert!(!shows_layers(&published(serde_json::json!([layer("text", false)]))), "switched off");
        assert!(shows_layers(&published(serde_json::json!([layer("audio", true), layer("text", true)]))));
        assert!(shows_layers(&published(serde_json::json!([layer("video", true)]))), "even offline media shows");
    }

    /// The viewer's last picture is the poster when it shows something; a
    /// flat one (the ground while the project loaded) is passed over for
    /// the first frame with content, and the stamp says what the project
    /// holds.
    #[test]
    fn the_viewers_last_picture_comes_first_unless_it_is_flat() {
        let profile = tempfile::tempdir().unwrap();
        let dir = profile.path().join("caches/thumbnails");
        let picture = frame(|x, _| [x as u8 * 12, 0, 0, 255]);
        let drawn = Arc::new(std::sync::Mutex::new(0));
        let count = drawn.clone();
        let renderer: FrameRenderer = Arc::new(move |_: &FrameRequest| {
            *count.lock().unwrap() += 1;
            Ok(frame(|_, y| [0, y as u8 * 20, 0, 255]))
        });
        let (last, last_stamp) = last_frame_files(profile.path(), "P1");
        std::fs::create_dir_all(last.parent().unwrap()).unwrap();
        // Flat: drawn instead.
        std::fs::write(&last, frame(|_, _| [35, 35, 35, 255])).unwrap();
        std::fs::write(&last_stamp, last_frame_mark(3, 12.)).unwrap();
        let text = serde_json::json!([{"id": "L1", "name": "Twenty", "type": "text", "on": true,
            "from": 0, "dur": 10, "parent": null}]);
        let png = write(&renderer, Some(Arc::new(published(text.clone()))), &dir, "P1", 9, (1920, 1080), None);
        assert!(has_content(&png.unwrap()));
        assert_eq!(*drawn.lock().unwrap(), 1, "the start, from the project's own length");
        assert_eq!(blank(&files(&dir, "P1").1), Some(false));
        // A picture: taken as it is, nothing drawn.
        std::fs::write(&last, &picture).unwrap();
        let png = write(&renderer, Some(Arc::new(published(text))), &dir, "P1", 10, (1920, 1080), None);
        assert_eq!(png.unwrap(), picture);
        assert_eq!(*drawn.lock().unwrap(), 1);
        assert_eq!(self::stamp(&files(&dir, "P1").1), Some((10, Some(12.))));
        // An empty project's flat frame is its poster, and it is blank.
        std::fs::remove_file(&last).unwrap();
        let flat: FrameRenderer = Arc::new(|_| Ok(frame(|_, _| [35, 35, 35, 255])));
        assert!(write(&flat, Some(Arc::new(published(serde_json::json!([])))), &dir, "P2", 1, (1920, 1080), None).is_some());
        assert_eq!(blank(&files(&dir, "P2").1), Some(true));
    }

    #[test]
    fn the_first_frame_with_content_is_stored_and_blank_ones_are_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = asked.clone();
        // Blank until half way in.
        let renderer: FrameRenderer = Arc::new(move |request: &FrameRequest| {
            seen.lock().unwrap().push(request.time);
            Ok(if request.time >= 30. {
                frame(|x, _| [x as u8 * 12, 0, 0, 255])
            } else {
                frame(|_, _| [0, 0, 0, 255])
            })
        });
        let png = write(
            &renderer,
            None,
            dir.path(),
            "file-1/a",
            7,
            (1920, 1080),
            Some(60.),
        );
        assert!(has_content(&png.unwrap()));
        assert_eq!(
            *asked.lock().unwrap(),
            [0., 5., 10., 20., 30.],
            "it stops at the first picture"
        );
        let (file, stamp) = files(dir.path(), "file-1/a");
        assert!(has_content(&std::fs::read(file).unwrap()));
        assert_eq!(self::stamp(&stamp), Some((7, Some(30.))));
        // Saved again: one frame, where the last poster was found.
        asked.lock().unwrap().clear();
        write(
            &renderer,
            None,
            dir.path(),
            "file-1/a",
            8,
            (1920, 1080),
            Some(60.),
        );
        assert_eq!(*asked.lock().unwrap(), [30.]);
        assert_eq!(self::stamp(&stamp), Some((8, Some(30.))));
        // Another build drew it (or one that did not say): stale whatever
        // its date, and the time it was found at is kept for the redraw.
        assert_eq!(stamp_by(&stamp, "another build"), Some((STALE, Some(30.))));
        std::fs::write(&stamp, "8\n30").unwrap();
        assert_eq!(self::stamp(&stamp), Some((STALE, Some(30.))));
        // Blank everywhere: its blank frame is still its poster.
        let blank: FrameRenderer = Arc::new(|_| Ok(frame(|_, _| [0, 0, 0, 255])));
        assert!(write(&blank, None, dir.path(), "b", 1, (1920, 1080), Some(60.)).is_some());
        // Nothing renders: nothing is stored.
        let failing: FrameRenderer = Arc::new(|_| Err("offline".into()));
        assert!(write(&failing, None, dir.path(), "c", 1, (1920, 1080), Some(60.)).is_none());
        assert!(!files(dir.path(), "c").0.exists());
        forget(dir.path(), "b");
        assert!(!files(dir.path(), "b").0.exists());
        assert!(!files(dir.path(), "b").1.exists());
        assert!(files(dir.path(), "file-1/a").0.exists());
    }
}
