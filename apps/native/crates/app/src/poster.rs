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
/// A stored poster's stamp and the time in the composition it shows.
pub(crate) fn stamp(stamp_file: &Path) -> Option<(i64, Option<f64>)> {
    let text = std::fs::read_to_string(stamp_file).ok()?;
    let mut lines = text.lines();
    Some((
        lines.next()?.trim().parse().ok()?,
        lines.next().and_then(|at| at.trim().parse().ok()),
    ))
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
        let Ok(png) = renderer(&FrameRequest {
            time,
            width: WIDTH,
            height: height.clamp(90, 2 * WIDTH),
            motion_blur: false,
            stop: &never,
            published: published.clone(),
        }) else {
            continue;
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
    std::fs::write(&stamp_file, format!("{stamp}\n{time}")).ok()?;
    Some(png)
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
        // Blank everywhere: its blank frame is still its poster.
        let blank: FrameRenderer = Arc::new(|_| Ok(frame(|_, _| [0, 0, 0, 255])));
        assert!(write(&blank, None, dir.path(), "b", 1, (1920, 1080), Some(60.)).is_some());
        // Nothing renders: nothing is stored.
        let failing: FrameRenderer = Arc::new(|_| Err("offline".into()));
        assert!(write(&failing, None, dir.path(), "c", 1, (1920, 1080), Some(60.)).is_none());
        assert!(!files(dir.path(), "c").0.exists());
    }
}
