//! The unity-gain document played by the source monitor.
//!
//! Keep the offline PCM parity probe on the same graph as device playback.

use std::path::{Path, PathBuf};

use pm_model::Document;
use pm_protocol::common::{TICKS_PER_SECOND, Ticks};

/// The original file's presentation timeline, independent of imported clip
/// metadata. AudioDecoder exposes program samples; `audio_start` places
/// sample zero back on the video's presentation timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceMonitorTiming {
    pub duration: f64,
    pub audio_start: Option<f64>,
}

/// One current source per monitor session, retained across pause and seek.
/// The owner clears it when the binding changes or the monitor closes.
#[derive(Default)]
pub(crate) struct SourceMonitorTimingCache {
    entry: Option<(String, PathBuf, SourceStamp, SourceMonitorTiming)>,
}

#[derive(PartialEq)]
struct SourceStamp {
    length: u64,
    modified: Option<std::time::SystemTime>,
    // Replacement, even with the same size and restored modification time.
    identity: (u64, u64, i64, i64),
}

impl SourceMonitorTimingCache {
    pub(crate) fn get(&self, asset: &str) -> Option<SourceMonitorTiming> {
        self.entry
            .as_ref()
            .filter(|(current, ..)| current == asset)
            .map(|(_, _, _, timing)| *timing)
    }

    pub(crate) fn open(&mut self, asset: &str, path: &Path) -> Result<SourceMonitorTiming, String> {
        self.open_with(asset, path, SourceMonitorTiming::open)
    }

    fn open_with(
        &mut self,
        asset: &str,
        path: &Path,
        read: impl FnOnce(&Path) -> Result<SourceMonitorTiming, String>,
    ) -> Result<SourceMonitorTiming, String> {
        use std::os::unix::fs::MetadataExt;
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.entry = None;
                return Err(error.to_string());
            }
        };
        let stamp = SourceStamp {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        };
        if let Some((current, file, version, timing)) = &self.entry
            && current == asset
            && file == path
            && *version == stamp
        {
            return Ok(*timing);
        }
        self.entry = None;
        let timing = read(path)?;
        self.entry = Some((asset.to_owned(), path.to_owned(), stamp, timing));
        Ok(timing)
    }

    pub(crate) fn invalidate(&mut self, asset: &str) {
        if self
            .entry
            .as_ref()
            .is_some_and(|(current, ..)| current == asset)
        {
            self.entry = None;
        }
    }
}

impl SourceMonitorTiming {
    pub fn open(path: &Path) -> Result<Self, String> {
        let probe = pm_media::probe::probe_timing(path).map_err(|e| e.to_string())?;
        let video = probe.video_stream.is_some();
        let mut duration = probe.duration_us.map(|d| {
            (d as f64
                + if video {
                    probe.start_us.unwrap_or(0) as f64
                } else {
                    0.0
                })
                / 1_000_000.0
        });
        let audio_start = if let Some(stream) = probe.audio_stream {
            let decoder = pm_media::audio::AudioDecoder::open(path, Some(stream))
                .map_err(|e| e.to_string())?;
            let info = decoder.info();
            let track = probe
                .streams
                .iter()
                .find(|s| s.index == stream)
                .ok_or("The source audio stream is missing from its probe")?;
            let start = if video {
                track.start_pts.unwrap_or(0) as f64 * track.timebase[0] as f64
                    / track.timebase[1] as f64
                    + info.extra_priming as f64 / info.sample_rate as f64
            } else {
                0.0
            };
            if let Some(length) = info.duration() {
                duration = Some(if video {
                    duration.unwrap_or(0.0).max(start + length)
                } else {
                    length
                });
            }
            Some(start)
        } else {
            if !video {
                return Err("The source has neither video nor audio".into());
            }
            None
        };
        if let Some(stream) = probe.video_stream {
            let track = probe
                .streams
                .iter()
                .find(|s| s.index == stream)
                .ok_or("The source video stream is missing from its probe")?;
            let metadata_end = track
                .duration_pts
                .map(|length| {
                    (track.start_pts.unwrap_or(0) as f64 + length as f64) * track.timebase[0] as f64
                        / track.timebase[1] as f64
                })
                .filter(|end| end.is_finite() && *end > 0.0);
            // A known audio/container duration does not establish the selected
            // video's endpoint. Index only when its stream timing is missing;
            // the monitor cache retains the result across seek and status.
            let end = match metadata_end {
                Some(end) => end,
                None => Self::indexed_video_end(path, stream)?,
            };
            duration = Some(duration.unwrap_or(0.0).max(end));
        }
        let timing = Self {
            duration: duration.ok_or("The source presentation endpoint is unavailable")?,
            audio_start,
        };
        timing.duration_ticks()?;
        Ok(timing)
    }

    fn indexed_video_end(path: &Path, stream: usize) -> Result<f64, String> {
        let input = pm_media::ff::Input::open(path).map_err(|e| e.to_string())?;
        let index = pm_media::index::build(path, &input, stream).map_err(|e| e.to_string())?;
        let frame = index
            .frames
            .last()
            .ok_or("The source video has no presented frames")?;
        let sample = &index.samples[frame.sample];
        if sample.duration <= 0 {
            return Err("The final source video frame has no duration".into());
        }
        let end = frame
            .pts
            .checked_add(sample.duration)
            .ok_or("The source video endpoint is outside the supported timeline")?;
        Ok(end as f64 * index.timebase.num() as f64 / index.timebase.den() as f64)
    }

    pub fn duration_ticks(&self) -> Result<Ticks, String> {
        let ticks = self.duration * TICKS_PER_SECOND as f64;
        if !ticks.is_finite() || ticks <= 0.0 || ticks >= i64::MAX as f64 {
            return Err("The source duration is outside the supported timeline".into());
        }
        Ticks::new(ticks.round() as i64).map_err(|e| e.to_string())
    }
}

/// Builds the source monitor's isolated, one-voice document. Its source path
/// is resolved separately by the caller. Only this isolated document's layer
/// placement changes; authored composition trims are never rewritten.
pub fn source_monitor_document(
    asset: &str,
    timing: &SourceMonitorTiming,
) -> Result<Document, String> {
    timing.duration_ticks()?;
    let start = timing.audio_start.ok_or("The source has no audio")?;
    if asset.is_empty() || !start.is_finite() {
        return Err("The source audio needs an asset and a finite start".into());
    }
    let duration = timing.duration;
    let document = serde_json::json!({
        "id": "source-monitor", "compId": "source", "name": "Source monitor",
        "w": 1, "h": 1, "fps": 30, "dur": duration,
        "layers": [{
            "id": "source-audio", "type": "audio", "name": "Source audio",
            "from": start, "dur": (duration - start).max(0.0), "on": true, "solo": false,
            "group": null, "parent": null, "p": {}, "fx": [], "masks": [],
            "d": {"asset": asset, "gain": 1, "trim": 0, "fadeIn": 0, "fadeOut": 0}
        }], "comps": {}, "assets": {}, "params": {}
    });
    let parsed = pm_model::json::parse(&document.to_string()).map_err(|e| e.to_string())?;
    Ok(Document::new(Some(parsed), 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_audio::{MixFormat, MixGraph, Mixer, SourceCache, offline};
    use pm_eval::Evaluator;
    use std::process::Command;
    use std::sync::Arc;

    const RATE: u32 = 48_000;

    fn ffmpeg() -> PathBuf {
        PathBuf::from(
            std::env::var_os("PM_FFMPEG_DIR").expect("PM_FFMPEG_DIR for source monitor fixtures"),
        )
        .join("bin/ffmpeg")
    }

    fn fixture(dir: &Path, video: f64, audio: Option<(f64, f64)>) -> std::path::PathBuf {
        let path = dir.join("source.mov");
        let mut command = Command::new(ffmpeg());
        command
            .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
            .arg(format!("color=red:size=64x64:rate=4:duration={video}"));
        if let Some((length, start)) = audio {
            command.args(["-itsoffset", &start.to_string(), "-f", "lavfi", "-i"])
                .arg(format!("aevalsrc=if(lt(t\\,0.5)\\,0.125\\,0.375)|if(lt(t\\,0.5)\\,-0.125\\,-0.375):s={RATE}:d={length}"));
        }
        let result = command
            .args([
                "-c:v",
                "h264_videotoolbox",
                "-pix_fmt",
                "yuv420p",
                "-threads",
                "1",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        path
    }

    fn missing_duration_fixture(dir: &Path, audio: Option<(f64, f64)>) -> PathBuf {
        let original = fixture(dir, 2.0, audio);
        let path = dir.join("duration-less.mkv");
        // Live Matroska omits Duration and per-track duration tags while
        // retaining complete frame timestamps and DefaultDuration in the file.
        let result = Command::new(ffmpeg())
            .args(["-v", "error", "-nostdin", "-y", "-i"])
            .arg(original)
            .args([
                "-map",
                "0",
                "-c",
                "copy",
                "-avoid_negative_ts",
                "disabled",
                "-live",
                "1",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let probe = pm_media::probe::probe_timing(&path).unwrap();
        assert_eq!(
            probe.duration_us, None,
            "fixture must lack a container duration"
        );
        let video = probe.video_stream.unwrap();
        assert_eq!(
            probe
                .streams
                .iter()
                .find(|s| s.index == video)
                .unwrap()
                .duration_pts,
            None,
            "fixture must lack a selected-video duration"
        );
        path
    }

    fn render(path: &Path, timing: &SourceMonitorTiming, from: f64, to: f64) -> Vec<f32> {
        let doc = source_monitor_document("source", timing).unwrap();
        let cache = SourceCache::new(8 << 20, 0);
        let resolver = |asset: &str| (asset == "source").then(|| path.to_path_buf());
        let graph =
            MixGraph::build(&doc, "source", &mut Evaluator::new(), &cache, &resolver).unwrap();
        assert!(graph.problems.is_empty(), "{:?}", graph.problems);
        assert_eq!(graph.voices.len(), 1);
        let mut mixer = Mixer::new(
            MixFormat::stereo(RATE),
            Arc::new(doc),
            Arc::new(graph),
            Evaluator::new(),
            cache,
        );
        let first = (from * f64::from(RATE)).round() as i64;
        let end = (to * f64::from(RATE)).round() as i64;
        let mut pcm = Vec::new();
        let summary = offline::render(&mut mixer, first, end, 1024, |_, samples| {
            pcm.extend_from_slice(samples);
            Ok(())
        })
        .unwrap();
        assert!(summary.errors.is_empty(), "{:?}", summary.errors);
        assert_eq!(pcm.len(), (end - first) as usize * 2);
        pcm
    }

    fn region(pcm: &[f32], from: f64, to: f64, left: f32) {
        let first = (from * f64::from(RATE)) as usize;
        let end = (to * f64::from(RATE)) as usize;
        for (index, frame) in pcm[first * 2..end * 2].chunks_exact(2).enumerate() {
            assert!(
                (frame[0] - left).abs() < 2e-5 && (frame[1] + left).abs() < 2e-5,
                "frame {}: {frame:?}, expected ±{left}",
                first + index
            );
        }
    }

    fn transport_ends_at_document_duration(timing: &SourceMonitorTiming, from: f64) {
        use pm_audio::transport::{Effect, Transport};
        use pm_protocol::clock::{TransportCommand, TransportState};
        use pm_protocol::common::PlaybackRate;
        let doc = source_monitor_document("source", timing).unwrap();
        let duration =
            pm_eval::node::finite_number(pm_eval::node::member(doc.root(), "dur")).unwrap();
        let end = (duration * f64::from(RATE)).round() as i64;
        let mut transport = Transport::new(RATE);
        transport.set_composition(
            None,
            Some((duration * TICKS_PER_SECOND as f64).round() as i64),
        );
        transport.set_position((from * f64::from(RATE)).round() as i64);
        transport
            .apply(&TransportCommand::Play {
                rate: PlaybackRate { num: 1, den: 1 },
            })
            .unwrap();
        while let Some(segment) = transport.next(1024) {
            assert!(segment.start + segment.frames as i64 <= end);
        }
        assert_eq!(transport.position(), end);
        assert_eq!(transport.take_stop(), Effect::Stopped);
        assert_eq!(transport.state(), TransportState::Stopped);
    }

    #[test]
    fn source_monitor_short_audio_keeps_video_and_silent_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), 2.0, Some((0.75, 0.0)));
        let timing = SourceMonitorTiming::open(&path).unwrap();
        assert_eq!(timing.duration, 2.0);
        assert_eq!(timing.audio_start, Some(0.0));
        let pcm = render(&path, &timing, 0.0, 2.0);
        region(&pcm, 0.05, 0.45, 0.125);
        region(&pcm, 0.55, 0.7, 0.375);
        region(&pcm, 0.8, 2.0, 0.0);
        let seek = render(&path, &timing, 1.25, 2.0);
        assert!(seek.iter().all(|s| *s == 0.0));
        transport_ends_at_document_duration(&timing, 0.0);
        transport_ends_at_document_duration(&timing, 1.25);
    }

    #[test]
    fn source_monitor_long_audio_survives_video_eof() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), 0.75, Some((2.0, 0.0)));
        let timing = SourceMonitorTiming::open(&path).unwrap();
        assert_eq!(timing.duration, 2.0);
        let pcm = render(&path, &timing, 0.0, 2.0);
        region(&pcm, 1.0, 1.95, 0.375);
        let seek = render(&path, &timing, 1.25, 1.75);
        region(&seek, 0.05, 0.45, 0.375);
        transport_ends_at_document_duration(&timing, 0.0);
    }

    #[test]
    fn source_monitor_delayed_pcm_preserves_offset_and_seek() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), 2.0, Some((1.0, 0.5)));
        let timing = SourceMonitorTiming::open(&path).unwrap();
        assert_eq!(timing.duration, 2.0);
        assert_eq!(timing.audio_start, Some(0.5));
        let pcm = render(&path, &timing, 0.0, 2.0);
        region(&pcm, 0.0, 0.45, 0.0);
        region(&pcm, 0.55, 0.95, 0.125);
        region(&pcm, 1.05, 1.45, 0.375);
        region(&pcm, 1.55, 2.0, 0.0);
        // This straddles the authored delay and the change within the PCM,
        // proving a seek uses source time rather than restarting the program.
        let seek = render(&path, &timing, 0.25, 1.75);
        assert_eq!(seek, pcm[RATE as usize / 2..RATE as usize * 7 / 2]);
        transport_ends_at_document_duration(&timing, 1.75);
    }

    #[test]
    fn source_monitor_audio_presence_comes_from_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), 2.0, None);
        let timing = SourceMonitorTiming::open(&path).unwrap();
        assert_eq!(timing.duration, 2.0);
        assert_eq!(timing.audio_start, None);
    }

    #[test]
    fn source_monitor_missing_duration_silent_video_uses_cached_frame_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = missing_duration_fixture(dir.path(), None);
        let reads = std::cell::Cell::new(0);
        let read = |path: &Path| {
            reads.set(reads.get() + 1);
            SourceMonitorTiming::open(path)
        };
        let mut cache = SourceMonitorTimingCache::default();
        let timing = cache.open_with("source", &path, &read).unwrap();
        assert_eq!(timing.duration, 2.0, "includes the final frame's duration");
        assert_eq!(timing.audio_start, None);
        assert_eq!(cache.open_with("source", &path, &read).unwrap(), timing);
        assert_eq!(cache.get("source"), Some(timing));
        assert_eq!(reads.get(), 1, "seek/status reuse the indexed endpoint");
        // The silent transport uses the same endpoint without an audio document.
        assert_eq!(
            timing.duration_ticks().unwrap(),
            Ticks::from_whole_seconds(2)
        );
    }

    #[test]
    fn source_monitor_missing_duration_video_outlasts_short_audio() {
        let dir = tempfile::tempdir().unwrap();
        let path = missing_duration_fixture(dir.path(), Some((0.75, 0.0)));
        let timing = SourceMonitorTiming::open(&path).unwrap();
        assert_eq!(timing.duration, 2.0);
        assert_eq!(timing.audio_start, Some(0.0));
        let pcm = render(&path, &timing, 0.0, 2.0);
        region(&pcm, 0.05, 0.45, 0.125);
        region(&pcm, 0.8, 2.0, 0.0);
        let seek = render(&path, &timing, 1.25, 2.0);
        assert!(seek.iter().all(|sample| *sample == 0.0));
        transport_ends_at_document_duration(&timing, 1.25);
    }

    #[test]
    fn source_monitor_timing_cache_reuses_only_unchanged_current_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.mov");
        std::fs::write(&path, b"first").unwrap();
        let reads = std::cell::Cell::new(0);
        let read = |_: &Path| {
            reads.set(reads.get() + 1);
            Ok(SourceMonitorTiming {
                duration: 2.0,
                audio_start: Some(0.5),
            })
        };
        let mut cache = SourceMonitorTimingCache::default();
        let first = cache.open_with("asset", &path, &read).unwrap();
        for _ in 0..8 {
            assert_eq!(cache.open_with("asset", &path, &read).unwrap(), first);
        }
        cache.invalidate("unrelated");
        cache.open_with("asset", &path, &read).unwrap();
        assert_eq!(
            reads.get(),
            1,
            "seeks and unrelated answers keep the warm facts"
        );
        cache.invalidate("asset");
        cache.open_with("asset", &path, &read).unwrap();
        assert_eq!(reads.get(), 2, "rebinding invalidates even the same path");
        std::fs::write(&path, b"replacement").unwrap();
        cache.open_with("asset", &path, &read).unwrap();
        assert_eq!(reads.get(), 3, "a changed original is reprobed");
        cache.open_with("another-asset", &path, &read).unwrap();
        cache.open_with("asset", &path, &read).unwrap();
        assert_eq!(reads.get(), 5, "only one current source is retained");
        std::fs::remove_file(&path).unwrap();
        assert!(cache.open_with("asset", &path, &read).is_err());
        assert!(
            cache.entry.is_none(),
            "an unavailable original drops its facts"
        );
    }
}
