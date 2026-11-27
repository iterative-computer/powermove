//! Reading an export back through AVFoundation for verification: decoded frames at
//! chosen indices (VideoToolbox decode to the encoder-input format), and the video
//! format description's extensions.

#![allow(deprecated)]

use std::path::Path;

use objc2::rc::Retained;
use objc2_av_foundation::*;
use objc2_core_foundation::{CFDictionary, CFRetained};
use objc2_core_media::{CMFormatDescription, CMSampleBuffer, CMTime, CMTimeRange};
use objc2_foundation::{NSDictionary, NSString, NSURL};

use super::cf;
use super::pixbuf::{self, PixelBuffer};
use crate::Error;
use crate::convert::EncoderInput;
use crate::settings::InputFormat;
use crate::timescale::VideoTiming;

fn video_track(path: &Path) -> Result<(Retained<AVURLAsset>, Retained<AVAssetTrack>), Error> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
    let media = unsafe { AVMediaTypeVideo }.expect("AVMediaTypeVideo");
    let tracks = unsafe { asset.tracksWithMediaType(media) };
    let track = tracks
        .firstObject()
        .ok_or_else(|| Error::Verify("AVFoundation finds no video track".into()))?;
    Ok((asset, track))
}

/// Key → description of the first video format description's extensions.
pub fn video_extensions(path: &Path) -> Result<Vec<(String, String)>, Error> {
    let (_asset, track) = video_track(path)?;
    let fds = unsafe { track.formatDescriptions() };
    let first = fds
        .firstObject()
        .ok_or_else(|| Error::Verify("no video format description".into()))?;
    let fd: &CMFormatDescription =
        unsafe { &*(Retained::as_ptr(&first) as *const CMFormatDescription) };
    let Some(ext) = (unsafe { fd.extensions() }) else {
        return Ok(vec![]);
    };
    Ok(pairs(&ext))
}

fn pairs(d: &CFRetained<CFDictionary>) -> Vec<(String, String)> {
    let nd: &NSDictionary<NSString, objc2::runtime::AnyObject> = unsafe {
        &*(&**d as *const CFDictionary as *const NSDictionary<NSString, objc2::runtime::AnyObject>)
    };
    let mut out: Vec<(String, String)> = nd
        .allKeys()
        .iter()
        .filter_map(|k| {
            nd.objectForKey(&k)
                .map(|v| (k.to_string(), cf::describe(&v).replace('\n', " ")))
        })
        .collect();
    out.sort();
    out
}

/// Decodes frame `index` to `format`.
pub fn decode_frame(
    path: &Path,
    index: u64,
    timing: VideoTiming,
    format: InputFormat,
) -> Result<EncoderInput, Error> {
    let (asset, track) = video_track(path)?;
    let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(&asset) }
        .map_err(|e| Error::Verify(format!("AVAssetReader: {}", e.localizedDescription())))?;
    let settings = pixbuf::output_attributes(format.code());
    let output = cf::objc_try("AVAssetReaderTrackOutput", || unsafe {
        let output = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
            &track,
            Some(&settings),
        );
        output.setAlwaysCopiesSampleData(false);
        reader.addOutput(&output);
        reader.setTimeRange(CMTimeRange {
            start: CMTime::new(timing.pts(index), timing.timescale),
            duration: CMTime::new(timing.frame_duration, timing.timescale),
        });
        output
    })
    .map_err(|e| Error::Verify(e.to_string()))?;
    if !unsafe { reader.startReading() } {
        return Err(Error::Verify(format!("cannot read frame {index}")));
    }
    let want = timing.pts(index) as i128;
    while let Some(sb) = unsafe { output.copyNextSampleBuffer() } {
        let sb: &CMSampleBuffer = &sb;
        let pts = unsafe { sb.presentation_time_stamp() };
        // Compare as rationals: the file's timescale is the one we set, but be exact anyway.
        if (pts.value as i128) * timing.timescale as i128 != want * pts.timescale as i128 {
            continue;
        }
        let Some(image) = (unsafe { sb.image_buffer() }) else {
            continue;
        };
        let pb = unsafe { PixelBuffer::retain(std::ptr::NonNull::from(&*image)) };
        unsafe { reader.cancelReading() };
        return pb.read(format);
    }
    Err(Error::Verify(format!(
        "frame {index} is missing from the file"
    )))
}

/// Readers that decode one file at once, each over its own run of frames.
const READERS: u64 = 4;

/// Decodes every frame of the file to `format` and hands each decoded
/// buffer to `each` with its index, counting it in `checked`. Up to
/// [`READERS`] readers work at once, each over its own run of frames in
/// presentation order; a frame that is not at its exact time, or a frame
/// beyond `frames`, fails.
pub fn decode_frames(
    path: &Path,
    timing: VideoTiming,
    format: InputFormat,
    frames: u64,
    each: &(dyn Fn(u64, &PixelBuffer) -> Result<(), Error> + Sync),
    checked: &std::sync::atomic::AtomicU64,
) -> Result<(), Error> {
    let readers = (frames / 16).clamp(1, READERS);
    decode_frames_with(path, timing, format, frames, readers, each, checked)
}

/// [`decode_frames`] with `readers` readers.
pub fn decode_frames_with(
    path: &Path,
    timing: VideoTiming,
    format: InputFormat,
    frames: u64,
    readers: u64,
    each: &(dyn Fn(u64, &PixelBuffer) -> Result<(), Error> + Sync),
    checked: &std::sync::atomic::AtomicU64,
) -> Result<(), Error> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let readers = readers.clamp(1, frames.max(1));
    let failed = AtomicBool::new(false);
    let results: Vec<Result<(), Error>> = std::thread::scope(|scope| {
        let runs: Vec<_> = (0..readers)
            .map(|r| {
                let (start, end) = (frames * r / readers, frames * (r + 1) / readers);
                let last = r + 1 == readers;
                let failed = &failed;
                scope.spawn(move || {
                    let result = decode_run(
                        path,
                        timing,
                        format,
                        frames,
                        start,
                        (!last).then_some(end),
                        &|i, pb| {
                            if failed.load(Ordering::Acquire) {
                                return Err(Error::Cancelled);
                            }
                            each(i, pb)?;
                            checked.fetch_add(1, Ordering::AcqRel);
                            Ok(())
                        },
                    );
                    if result.is_err() {
                        failed.store(true, Ordering::Release);
                    }
                    result
                })
            })
            .collect();
        runs.into_iter()
            .map(|run| {
                run.join()
                    .unwrap_or_else(|_| Err(Error::Verify("a reader panicked".into())))
            })
            .collect()
    });
    // The first real failure, before the stops it caused in other readers.
    let mut cancelled = None;
    for result in results {
        match result {
            Err(Error::Cancelled) => cancelled = Some(Error::Cancelled),
            Err(e) => return Err(e),
            Ok(()) => {}
        }
    }
    cancelled.map_or(Ok(()), Err)
}

/// Decodes frames `start` up to `end` (to the end of the file when `None`).
fn decode_run(
    path: &Path,
    timing: VideoTiming,
    format: InputFormat,
    frames: u64,
    start: u64,
    end: Option<u64>,
    each: &dyn Fn(u64, &PixelBuffer) -> Result<(), Error>,
) -> Result<(), Error> {
    let (asset, track) = video_track(path)?;
    let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(&asset) }
        .map_err(|e| Error::Verify(format!("AVAssetReader: {}", e.localizedDescription())))?;
    let settings = pixbuf::output_attributes(format.code());
    let output = cf::objc_try("AVAssetReaderTrackOutput", || unsafe {
        let output = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
            &track,
            Some(&settings),
        );
        output.setAlwaysCopiesSampleData(false);
        reader.addOutput(&output);
        if start > 0 || end.is_some() {
            let from = CMTime::new(timing.pts(start), timing.timescale);
            reader.setTimeRange(CMTimeRange {
                start: from,
                duration: match end {
                    Some(end) => CMTime::new(timing.pts(end) - timing.pts(start), timing.timescale),
                    None => objc2_core_media::kCMTimePositiveInfinity,
                },
            });
        }
        output
    })
    .map_err(|e| Error::Verify(e.to_string()))?;
    if !unsafe { reader.startReading() } {
        return Err(Error::Verify("cannot read the file's frames".into()));
    }
    let first = timing.pts(start) as i128;
    let mut index = start;
    while let Some(sb) = unsafe { output.copyNextSampleBuffer() } {
        let sb: &CMSampleBuffer = &sb;
        let Some(image) = (unsafe { sb.image_buffer() }) else {
            continue;
        };
        let pts = unsafe { sb.presentation_time_stamp() };
        let (value, scale) = (pts.value as i128, pts.timescale as i128);
        // A reader over a run may hand over the frame before it.
        if value * (timing.timescale as i128) < first * scale {
            continue;
        }
        if end.is_some_and(|end| index >= end) {
            break;
        }
        let want = timing.pts(index) as i128;
        if index >= frames || value * timing.timescale as i128 != want * scale {
            unsafe { reader.cancelReading() };
            return Err(Error::Verify(format!(
                "decoded frame {index} of {frames} is at {value}/{scale}, expected {want}/{}",
                timing.timescale
            )));
        }
        let pb = unsafe { PixelBuffer::retain(std::ptr::NonNull::from(&*image)) };
        if let Err(e) = each(index, &pb) {
            unsafe { reader.cancelReading() };
            return Err(e);
        }
        index += 1;
    }
    unsafe { reader.cancelReading() };
    Ok(())
}
