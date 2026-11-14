//! `<video>` in a realm: a file's frames at a time, for `drawImage` (an
//! extension that reads a clip's frames, as rotoscoping does). AVFoundation
//! decodes them exactly at the asked time, colour-managed, upright, in sRGB.
use crate::{Error, Result};
use pm_recorder::ImageData;
use std::path::Path;

/// One opened video.
pub(crate) struct Video {
    #[cfg(target_os = "macos")]
    generator: objc2::rc::Retained<objc2_av_foundation::AVAssetImageGenerator>,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
}
// SAFETY: the generator is only used under the host's lock, one call at a
// time; AVFoundation objects may be used from any one thread at a time.
unsafe impl Send for Video {}

fn unreadable() -> Error {
    Error::Host("The video cannot be decoded.".into())
}

#[cfg(target_os = "macos")]
impl Video {
    pub fn open(path: &Path) -> Result<Self> {
        use objc2_av_foundation::{AVAssetImageGenerator, AVURLAsset};
        use objc2_core_media::kCMTimeZero;
        use objc2_foundation::{NSString, NSURL};
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: a file URL and no options; the asset is kept by the generator.
        let (generator, duration) = unsafe {
            let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
            #[allow(deprecated)]
            let duration = asset.duration().seconds();
            let generator = AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset);
            generator.setAppliesPreferredTrackTransform(true);
            // Exact frames: the frame at the time asked, never a nearby one.
            generator.setRequestedTimeToleranceBefore(kCMTimeZero);
            generator.setRequestedTimeToleranceAfter(kCMTimeZero);
            (generator, duration)
        };
        if !(duration.is_finite() && duration > 0.) {
            return Err(Error::Host(format!(
                "The video cannot be decoded (duration {duration})."
            )));
        }
        let mut video = Self {
            generator,
            duration,
            width: 0,
            height: 0,
        };
        let first = video.frame(0.)?;
        (video.width, video.height) = (first.width(), first.height());
        Ok(video)
    }
    pub fn frame(&self, seconds: f64) -> Result<ImageData> {
        use objc2_core_media::CMTime;
        let at = seconds.clamp(0., self.duration);
        // SAFETY: a valid CMTime and a null actual-time pointer.
        #[allow(deprecated)]
        let image = unsafe {
            self.generator.copyCGImageAtTime_actualTime_error(
                CMTime::with_seconds(at, 600_000),
                std::ptr::null_mut(),
            )
        }
        .map_err(|error| {
            Error::Host(format!(
                "The video's frame at {at} s cannot be decoded: {}",
                error.localizedDescription()
            ))
        })?;
        crate::image::rgba_of(&image).ok_or_else(unreadable)
    }
}

#[cfg(not(target_os = "macos"))]
impl Video {
    pub fn open(_: &Path) -> Result<Self> {
        Err(unreadable())
    }
    pub fn frame(&self, _: f64) -> Result<ImageData> {
        Err(unreadable())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn a_clip_opens_with_its_duration_size_and_frames() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../engine/fixtures/h264-aac.mp4");
        let video = super::Video::open(&path).unwrap();
        assert!((video.duration - 2.).abs() < 0.05, "{}", video.duration);
        assert_eq!((video.width, video.height), (160, 90));
        let red = video.frame(0.5).unwrap();
        let green = video.frame(1.5).unwrap();
        let at =
            |frame: &pm_recorder::ImageData| frame.bytes()[(45 * 160 + 80) * 4..][..4].to_vec();
        // Colour-managed from the clip's tagged colour: red (247, 36, 0), green (63, 251, 0).
        assert!(at(&red)[0] > 200 && at(&red)[1] < 80, "{:?}", at(&red));
        assert!(
            at(&green)[1] > 200 && at(&green)[0] < 100,
            "{:?}",
            at(&green)
        );
    }
}
