//! AVFoundation I/O: AVAssetWriter (encoding and pass-through), AVAssetReader
//! (compressed samples), AVAssetExportSession.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::*;
use objc2_core_foundation::CFRetained;
use objc2_core_media::{CMFormatDescription, CMSampleBuffer, CMTime};
use objc2_foundation::{NSDictionary, NSString, NSURL};

use crate::cf::{self, Result, err};
use crate::pixbuf::PixelBuffer;

pub fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

pub fn mov() -> &'static AVFileType {
    unsafe { AVFileTypeQuickTimeMovie.unwrap() }
}

pub fn mp4() -> &'static AVFileType {
    unsafe { AVFileTypeMPEG4.unwrap() }
}

pub fn video() -> &'static AVMediaType {
    unsafe { AVMediaTypeVideo.unwrap() }
}

pub fn time(value: i64, timescale: i32) -> CMTime {
    unsafe { CMTime::new(value, timescale) }
}

pub struct VideoWriter {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    adaptor: Option<Retained<AVAssetWriterInputPixelBufferAdaptor>>,
    started: bool,
}

impl VideoWriter {
    fn new_inner(
        path: &Path,
        file_type: &AVFileType,
        settings: Option<&NSDictionary<NSString, AnyObject>>,
        hint: Option<&CMFormatDescription>,
        adaptor: bool,
        timescale: i32,
    ) -> Result<Self> {
        remove(path);
        let url = file_url(path);
        let writer = unsafe { AVAssetWriter::assetWriterWithURL_fileType_error(&url, file_type) }
            .map_err(|e| cf::Error(format!("AVAssetWriter: {}", e.localizedDescription())))?;
        let input = unsafe {
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings_sourceFormatHint(video(), settings, hint)
        };
        unsafe { input.setExpectsMediaDataInRealTime(false) };
        // Section 17: the media and movie timescales follow the rate (24000 for 23.976);
        // AVAssetWriter otherwise defaults to 600, which cannot represent 1001/24000.
        // A timescale ≤ 0 keeps AVAssetWriter's defaults (measured in exp_tags).
        if timescale > 0 {
            unsafe {
                input.setMediaTimeScale(timescale);
                writer.setMovieTimeScale(timescale);
            }
        }
        if !unsafe { writer.canAddInput(&input) } {
            return err("AVAssetWriter cannot add input");
        }
        unsafe { writer.addInput(&input) };
        let adaptor = if adaptor {
            Some(unsafe {
                AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(&input, None)
            })
        } else {
            None
        };
        Ok(Self { writer, input, adaptor, started: false })
    }

    /// Encoding writer: AVAssetWriter compresses appended pixel buffers with `settings`.
    pub fn encoding(
        path: &Path,
        file_type: &AVFileType,
        settings: &NSDictionary<NSString, AnyObject>,
        timescale: i32,
    ) -> Result<Self> {
        Self::new_inner(path, file_type, Some(settings), None, true, timescale)
    }

    /// Pass-through writer for samples compressed by our own VTCompressionSession.
    pub fn passthrough(
        path: &Path,
        file_type: &AVFileType,
        hint: &CMFormatDescription,
        timescale: i32,
    ) -> Result<Self> {
        Self::new_inner(path, file_type, None, Some(hint), false, timescale)
    }

    fn start(&mut self, t: CMTime) -> Result<()> {
        if !self.started {
            if !unsafe { self.writer.startWriting() } {
                return err(format!("startWriting: {}", self.error_text()));
            }
            unsafe { self.writer.startSessionAtSourceTime(t) };
            self.started = true;
        }
        Ok(())
    }

    fn wait_ready(&self) -> Result<()> {
        let t0 = Instant::now();
        while !unsafe { self.input.isReadyForMoreMediaData() } {
            if unsafe { self.writer.status() } == AVAssetWriterStatus::Failed {
                return err(format!("writer failed: {}", self.error_text()));
            }
            if t0.elapsed() > Duration::from_secs(60) {
                return err("writer input never became ready");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    pub fn append_pixel_buffer(&mut self, pb: &PixelBuffer, pts: CMTime) -> Result<()> {
        self.start(pts)?;
        self.wait_ready()?;
        let ok = unsafe { self.adaptor.as_ref().unwrap().appendPixelBuffer_withPresentationTime(pb.as_cf(), pts) };
        if !ok {
            return err(format!("appendPixelBuffer: {}", self.error_text()));
        }
        Ok(())
    }

    pub fn append_sample(&mut self, sb: &CMSampleBuffer) -> Result<()> {
        self.start(unsafe { sb.presentation_time_stamp() })?;
        self.wait_ready()?;
        if !unsafe { self.input.appendSampleBuffer(sb) } {
            return err(format!("appendSampleBuffer: {}", self.error_text()));
        }
        Ok(())
    }

    fn error_text(&self) -> String {
        unsafe { self.writer.error() }
            .map(|e| format!("{} ({})", e.localizedDescription(), cf::describe(&e)))
            .unwrap_or_else(|| "no error".into())
    }

    pub fn finish(self) -> Result<()> {
        unsafe { self.input.markAsFinished() };
        let (tx, rx) = mpsc::channel::<()>();
        let block = RcBlock::new(move || {
            let _ = tx.send(());
        });
        unsafe { self.writer.finishWritingWithCompletionHandler(&block) };
        rx.recv_timeout(Duration::from_secs(300)).map_err(|_| cf::Error("finishWriting timed out".into()))?;
        if unsafe { self.writer.status() } != AVAssetWriterStatus::Completed {
            return err(format!("finishWriting: {}", self.error_text()));
        }
        Ok(())
    }
}

/// The first video track of a file.
pub fn video_track(path: &Path) -> Result<(Retained<AVURLAsset>, Retained<AVAssetTrack>)> {
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&file_url(path), None) };
    #[allow(deprecated)]
    let tracks = unsafe { asset.tracksWithMediaType(video()) };
    let track = tracks.firstObject().ok_or_else(|| cf::Error(format!("no video track in {}", path.display())))?;
    Ok((asset, track))
}

/// Read every compressed sample of the first video track, in decode order.
pub fn read_compressed(path: &Path) -> Result<Vec<CFRetained<CMSampleBuffer>>> {
    let (asset, track) = video_track(path)?;
    let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(&asset) }
        .map_err(|e| cf::Error(format!("AVAssetReader: {}", e.localizedDescription())))?;
    let out = unsafe { AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, None) };
    unsafe { out.setAlwaysCopiesSampleData(false) };
    unsafe { reader.addOutput(&out) };
    if !unsafe { reader.startReading() } {
        return err(format!("startReading {}", path.display()));
    }
    let mut v = vec![];
    while let Some(sb) = unsafe { out.copyNextSampleBuffer() } {
        if unsafe { sb.total_sample_size() } == 0 {
            continue;
        }
        let raw = Retained::into_raw(sb) as *mut CMSampleBuffer;
        v.push(unsafe { CFRetained::from_raw(std::ptr::NonNull::new_unchecked(raw)) });
    }
    if unsafe { reader.status() } == AVAssetReaderStatus::Failed {
        return err(format!("reader failed for {}", path.display()));
    }
    Ok(v)
}

/// AVAssetExportSession with a preset; blocks until done.
pub fn export_session(src: &Path, dst: &Path, preset: &NSString, file_type: &AVFileType) -> Result<()> {
    remove(dst);
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&file_url(src), None) };
    let session =
        unsafe { AVAssetExportSession::initWithAsset_presetName(AVAssetExportSession::alloc(), &asset, preset) }
            .ok_or_else(|| cf::Error(format!("AVAssetExportSession init failed for preset {preset}")))?;
    unsafe {
        session.setOutputURL(Some(&file_url(dst)));
        session.setOutputFileType(Some(file_type));
    }
    let (tx, rx) = mpsc::channel::<()>();
    let block = RcBlock::new(move || {
        let _ = tx.send(());
    });
    #[allow(deprecated)]
    unsafe {
        session.exportAsynchronouslyWithCompletionHandler(&block)
    };
    rx.recv_timeout(Duration::from_secs(600)).map_err(|_| cf::Error("export timed out".into()))?;
    let status = unsafe { session.status() };
    if status != AVAssetExportSessionStatus::Completed {
        let e = unsafe { session.error() }.map(|e| cf::describe(&e)).unwrap_or_default();
        return err(format!("export {preset} failed: status {status:?} {e}"));
    }
    Ok(())
}

/// Facts about the first video track of a file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrackInfo {
    pub codec: String,
    pub estimated_data_rate_bps: f64,
    pub width: f64,
    pub height: f64,
    /// Format description extensions (color tags, depth, field info, and so on).
    pub extensions: Vec<(String, String)>,
}

#[allow(deprecated)]
pub fn track_info(path: &Path) -> Result<TrackInfo> {
    let (_asset, track) = video_track(path)?;
    let fds = unsafe { track.formatDescriptions() };
    let first = fds.firstObject().ok_or_else(|| cf::Error("no format description".into()))?;
    let fd: &CMFormatDescription = unsafe { &*(Retained::as_ptr(&first) as *const CMFormatDescription) };
    let codec = cf::fourcc_str(unsafe { fd.media_sub_type() });
    let extensions = unsafe { fd.extensions() }.map(|d| crate::pixbuf::dict_pairs(&d)).unwrap_or_default();
    let size = unsafe { track.naturalSize() };
    Ok(TrackInfo {
        codec,
        estimated_data_rate_bps: unsafe { track.estimatedDataRate() } as f64,
        width: size.width,
        height: size.height,
        extensions,
    })
}
