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
