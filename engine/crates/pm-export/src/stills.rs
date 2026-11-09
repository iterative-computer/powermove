//! Import Image Sequence: numbered stills into one lossless VP9 WebM with alpha
//! (`yuva420p`), frame n at n / fps — what today's importer writes and the
//! renderer plays as an `imageSequence` asset.
//!
//! Frames are the files the person chose, in frame-number order; missing
//! numbers are skipped (the available images play consecutively). Every frame
//! must have the first frame's size. 4:2:0 needs even dimensions, so an odd
//! width or height gains one transparent column or row; the picture itself is
//! never scaled.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::Error;
use crate::convert::{ConvertParams, WorkingFrame, convert};
use crate::pipeline::Sink;
use crate::settings::{
    AlphaOutput, Codec, ColorTags, Container, Dither, ExportSpec, Overwrite, WorkingEncoding,
};
use crate::webm::WebmSink;

/// What was encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StillSequence {
    /// The video's size (the stills' size, rounded up to even).
    pub width: u32,
    pub height: u32,
    pub frames: u64,
}

/// Where the encode is, for the person's progress line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Checking { done: u64, total: u64 },
    Creating { done: u64, total: u64 },
}

fn name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn reader(path: &Path) -> Result<image::ImageReader<std::io::BufReader<std::fs::File>>, Error> {
    image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|_| Error::Io(format!("Could not read {}", name(path))))
}

/// Encodes `frames` (already in order) at `fps` into a new file at
/// `destination`. Checks every frame's size before encoding anything, as
/// today's importer does, and stops between frames when `cancel` is set.
pub fn encode_still_sequence(
    frames: &[PathBuf],
    fps: f64,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Stage),
) -> Result<StillSequence, Error> {
    if frames.len() < 2 {
        return Err(Error::InvalidSpec(
            "Choose at least 2 numbered image frames".into(),
        ));
    }
    let rate = pm_time::snap_float_rate(fps)
        .ok()
        .filter(|_| (1.0..=240.0).contains(&fps))
        .ok_or_else(|| Error::InvalidSpec("Frame rate must be between 1 and 240 fps".into()))?
        .rate;
    let total = frames.len() as u64;
    let mut size = (0, 0);
    progress(Stage::Checking { done: 0, total });
    for (i, path) in frames.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        let dimensions = reader(path)?
            .into_dimensions()
            .map_err(|_| Error::Io(format!("Could not read {}", name(path))))?;
        if i == 0 {
            size = dimensions;
        } else if dimensions != size {
            return Err(Error::InvalidFrame(format!(
                "Frame dimensions differ: {} · all frames must be {} × {}",
                name(path),
                size.0,
                size.1
            )));
        }
        progress(Stage::Checking {
            done: i as u64 + 1,
            total,
        });
    }
    let (w, h) = size;
    let (width, height) = (w + w % 2, h + h % 2);
    let spec = ExportSpec {
        width,
        height,
        rate,
        frames: total,
        // Lossless: the bit rate is unused.
        codec: Codec::Vp9 {
            bitrate_bps: 16_000_000,
        },
        container: Container::Webm,
        // Stills are sRGB-encoded pictures: the working values pass through.
        tags: ColorTags::Rec709QuickTime,
        alpha: AlphaOutput::Straight,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: destination.to_path_buf(),
        overwrite: Overwrite::Fail,
        mode: None,
    };
    let mut sink = WebmSink::new_lossless(&spec)?;
    let format = spec.input_format();
    let mut working = vec![0f32; width as usize * height as usize * 4];
    progress(Stage::Creating { done: 0, total });
    for (i, path) in frames.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        let still = reader(path)?
            .decode()
            .map_err(|_| Error::Io(format!("Could not read {}", name(path))))?
            .into_rgba8();
        if still.dimensions() != (w, h) {
            return Err(Error::InvalidFrame(format!(
                "Frame dimensions differ: {} · all frames must be {w} × {h}",
                name(path)
            )));
        }
        premultiply_into(still.as_raw(), w as usize, width as usize, &mut working);
        let frame = WorkingFrame {
            width,
            height,
            rgba: &working,
        };
        let params = ConvertParams {
            working: spec.working,
            alpha: spec.alpha,
            tags: spec.tags,
            dither: false,
            frame: i as u32,
        };
        sink.video_owned(i as u64, convert(&frame, format, &params)?)?;
        progress(Stage::Creating {
            done: i as u64 + 1,
            total,
        });
    }
    sink.finish_with_cancel(cancel)?;
    Ok(StillSequence {
        width,
        height,
        frames: total,
    })
}

/// Straight RGBA8 rows of `w` pixels into premultiplied unit RGBA rows of
/// `stride` pixels; the padding stays transparent.
fn premultiply_into(rgba: &[u8], w: usize, stride: usize, out: &mut [f32]) {
    out.fill(0.);
    for (y, row) in rgba.chunks_exact(w * 4).enumerate() {
        let dst = &mut out[y * stride * 4..][..w * 4];
        for (d, s) in dst.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
            let a = f32::from(s[3]) / 255.;
            for c in 0..3 {
                d[c] = f32::from(s[c]) / 255. * a;
            }
            d[3] = a;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch folder, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("pm-stills-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn png(dir: &Path, name: &str, w: u32, h: u32, value: u8) -> PathBuf {
        let path = dir.join(name);
        let image = image::RgbaImage::from_fn(w, h, |x, _| {
            image::Rgba([value, 255 - value, 40, if x == 0 { 0 } else { 200 }])
        });
        image.save(&path).unwrap();
        path
    }

    #[test]
    fn stills_become_one_lossless_webm_with_alpha_frame_for_frame() {
        let dir = Scratch::new("lossless");
        let frames: Vec<_> = (0..20)
            .map(|i| png(dir.path(), &format!("f_{i:04}.png"), 33, 18, (i * 12) as u8))
            .collect();
        let out = dir.path().join("sequence.webm");
        let mut stages = vec![];
        let result = encode_still_sequence(&frames, 12., &out, &AtomicBool::new(false), &mut |s| {
            stages.push(s)
        })
        .unwrap();
        // An odd width gains a transparent column.
        assert_eq!(
            result,
            StillSequence {
                width: 34,
                height: 18,
                frames: 20
            }
        );
        assert_eq!(
            stages.first(),
            Some(&Stage::Checking { done: 0, total: 20 })
        );
        assert_eq!(
            stages.last(),
            Some(&Stage::Creating {
                done: 20,
                total: 20
            })
        );
        let facts = pm_media::facts::media_facts(&out, None).unwrap();
        assert_eq!(facts.display_size, Some([34, 18]));
        let duration = facts.duration.unwrap();
        assert!((duration - 20. / 12.).abs() < 0.01, "{duration}");
    }

    #[test]
    fn a_frame_of_another_size_is_named_before_anything_is_written() {
        let dir = Scratch::new("sizes");
        let frames = vec![
            png(dir.path(), "a1.png", 8, 8, 0),
            png(dir.path(), "a2.png", 8, 8, 0),
            png(dir.path(), "a3.png", 10, 8, 0),
        ];
        let out = dir.path().join("sequence.webm");
        let error = encode_still_sequence(&frames, 24., &out, &AtomicBool::new(false), &mut |_| {})
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid frame: Frame dimensions differ: a3.png · all frames must be 8 × 8"
        );
        assert!(!out.exists());
    }

    #[test]
    fn cancel_and_bad_rates_write_nothing() {
        let dir = Scratch::new("cancel");
        let frames = vec![
            png(dir.path(), "a1.png", 8, 8, 0),
            png(dir.path(), "a2.png", 8, 8, 0),
        ];
        let out = dir.path().join("sequence.webm");
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            encode_still_sequence(&frames, 24., &out, &cancel, &mut |_| {}),
            Err(Error::Cancelled)
        ));
        for fps in [0.5, 241., f64::NAN] {
            assert!(
                encode_still_sequence(&frames, fps, &out, &AtomicBool::new(false), &mut |_| {})
                    .is_err()
            );
        }
        assert!(!out.exists());
    }
}
