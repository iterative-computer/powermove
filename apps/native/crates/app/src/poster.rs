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
/// Where a poster is looked for, as shares of the composition's length: a
/// third in first (past titles fading up, before the end card), then either
/// side of it. The first frame that shows something is kept.
const PLACES: [f64; 6] = [1. / 3., 1. / 2., 1. / 6., 2. / 3., 5. / 6., 1. / 12.];
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
/// A trashed project's poster is removed; Put Back draws it again when needed.
pub(crate) fn forget(dir: &Path, id: &str) {
    let (image, stamp) = files(dir, id);
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
        return vec![1.];
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
    format!("{} ({build})", crate::updates::current_version())
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
/// Whether a frame shows anything: a flat frame (all black, or one colour
/// from edge to edge) does not.
fn has_content(png: &[u8]) -> bool {
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
    std::fs::write(&stamp_file, format!("{stamp}\n{time}\n{}", identity())).ok()?;
    Some(png)
}

/// Width of a stored last frame: it stands in the viewer, larger than a card.
const LAST_WIDTH: u32 = 1280;
/// The picture the viewer last showed for a project: rendered at the
/// playhead after a save and at a close, kept under the profile
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
/// last frame: a kill or a signal quit stores nothing, so the picture a
/// launch opens on is the last one that held this long.
pub(crate) const LAST_FRAME_STILL: std::time::Duration = std::time::Duration::from_secs(4);
/// The least time between two renders kept that way: each is an engine job.
pub(crate) const LAST_FRAME_GAP: std::time::Duration = std::time::Duration::from_secs(30);
/// What the still picture's watch does next.
#[derive(Debug, PartialEq)]
pub(crate) enum Still {
    /// Look again after this long.
    Wait(std::time::Duration),
    /// Render and keep it now.
    Store,
}
/// The still picture's watch, given how long the picture has held, how long
/// since the last render was asked for (if one was), whether one is still
/// rendering, and whether the picture is moving or still loading.
pub(crate) fn still(
    held: std::time::Duration,
    since_stored: Option<std::time::Duration>,
    rendering: bool,
    busy: bool,
) -> Still {
    if held < LAST_FRAME_STILL {
        return Still::Wait(LAST_FRAME_STILL - held);
    }
    if rendering || busy {
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
    /// The picture shown (its revision and time) and since when.
    pub shown: Option<((u64, i64), std::time::Instant)>,
    /// A watch is waiting for the picture to hold.
    pub watching: bool,
    /// A render is in flight.
    pub rendering: bool,
    /// When the last render was asked for.
    pub stored: Option<std::time::Instant>,
}
/// Whether the stored last frame already shows `mark`.
pub(crate) fn last_frame_current(profile: &Path, id: &str, mark: &str) -> bool {
    let (image, stamp) = last_frame_files(profile, id);
    image.is_file() && std::fs::read_to_string(stamp).is_ok_and(|s| s == mark)
}
/// Renders `published` at `time` and stores it as `id`'s last frame with
/// `mark`. Blocking: call it off the main thread. Whether it was stored.
pub(crate) fn write_last_frame(
    renderer: &FrameRenderer,
    published: Arc<pm_kernel::Published>,
    profile: &Path,
    id: &str,
    time: f64,
    (w, h): (u32, u32),
    mark: &str,
) -> bool {
    fn never() -> bool {
        false
    }
    let width = LAST_WIDTH.min(w.max(2));
    let height = (width as f64 * h as f64 / w.max(1) as f64).round().max(2.) as u32;
    let Ok(png) = renderer(&FrameRequest {
        time,
        width,
        height,
        motion_blur: false,
        stop: &never,
        published: Some(published),
    }) else {
        return false;
    };
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
    /// close or save with another one renders it again (PM-384).
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
    /// picture waits, and renders stay one at a time and spaced.
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
        assert_eq!(still(s(9), Some(s(10)), false, false), Still::Wait(s(20)));
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
    fn a_poster_is_looked_for_a_third_in_first() {
        let at = times(Some(60.), None);
        assert_eq!(at[0], 20.);
        assert_eq!(at.len(), PLACES.len());
        assert!(at.iter().all(|t| *t > 0. && *t < 60.));
        assert_eq!(times(None, None), [1.]);
        assert_eq!(times(Some(0.), Some(3.)), [1.]);
        // Where the last poster was found comes first, while it still exists.
        assert_eq!(times(Some(60.), Some(5.))[..2], [5., 20.]);
        assert_eq!(times(Some(4.), Some(5.))[0], 4. / 3.);
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
            [20., 30.],
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
