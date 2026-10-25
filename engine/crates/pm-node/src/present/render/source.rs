//! Source-only presenter audio, status, and frame rendering.

use super::super::source_audio::SourceMonitorTiming;
use super::*;

pub(super) struct SourceAudio {
    pub(super) asset: pm_protocol::common::AssetId,
    pub(super) generation: u32,
    pub(super) link: AudioLink,
    pub(super) playing: Arc<AtomicBool>,
    pub(super) duration: Ticks,
    pub(super) started: i64,
    pub(super) saw_playing: bool,
}

pub(super) struct SourceSilent {
    pub(super) asset: pm_protocol::common::AssetId,
    pub(super) generation: u32,
    pub(super) from: Ticks,
    pub(super) started: i64,
    pub(super) duration: Ticks,
    pub(super) last_sent: i64,
}

pub(super) fn silent_source_position(source: &SourceSilent, at: i64) -> (Ticks, bool) {
    let delta =
        ((at - source.started).max(0) as i128 * TICKS_PER_SECOND as i128 / 1_000_000) as i64;
    let ticks = source
        .from
        .get()
        .saturating_add(delta)
        .min(source.duration.get());
    let time = Ticks::new(ticks).unwrap_or(source.duration);
    (time, time < source.duration)
}

pub(super) fn audio_source_end(
    sample: &pm_protocol::clock::ClockSample,
    duration: Ticks,
    saw_playing: bool,
    at: pm_protocol::common::HostMicros,
) -> Option<Ticks> {
    let time = sample.media_time_at(at);
    if time >= duration {
        Some(duration)
    } else if saw_playing && sample.state == TransportState::Stopped {
        Some(time)
    } else {
        None
    }
}

impl SourceAudio {
    pub(super) fn start(
        key: Key,
        asset: pm_protocol::common::AssetId,
        generation: u32,
        path: PathBuf,
        timing: &SourceMonitorTiming,
        sink: Sink,
    ) -> Result<Self, String> {
        let duration_ticks = timing.duration_ticks()?;
        let doc = super::super::source_audio::source_monitor_document(asset.as_str(), timing)?;
        let mut media = BTreeMap::new();
        media.insert(asset.as_str().to_owned(), Some(path));
        let playing = Arc::new(AtomicBool::new(false));
        let heard = playing.clone();
        let status_asset = asset.clone();
        let levels: super::super::audio::LevelsSink = Box::new(move |levels| {
            let envelope = Envelope::new(Message::Event {
                body: Event::Audio(AudioMessage::SourceStatus {
                    asset: status_asset.clone(),
                    time: levels.media_time,
                    playing: heard.load(Ordering::Relaxed),
                    generation,
                    duration: Some(duration_ticks),
                    levels: levels.master,
                    error: None,
                }),
            })
            .with_session(key.1);
            if let Ok(bytes) = codec::encode(&envelope) {
                sink(vec![Action::Send { port: key.0, bytes }]);
            }
        });
        let link = AudioLink::start(
            doc,
            "source".to_owned(),
            super::super::document::MediaMap(Arc::new(media)),
            levels,
        )?;
        Ok(Self {
            asset,
            generation,
            link,
            playing,
            duration: duration_ticks,
            started: now(),
            saw_playing: false,
        })
    }
}

impl Thread {
    pub(super) fn handle_source_monitor(
        &mut self,
        key: Key,
        asset: Option<pm_protocol::common::AssetId>,
        time: Ticks,
        playing: bool,
        generation: u32,
    ) {
        let Some(asset) = asset else {
            self.source_intent.remove(&key);
            self.source_timing.remove(&key);
            if let Some(source) = self.source_audio.remove(&key) {
                self.source_status(key, source.asset, time, false, generation);
            }
            if let Some(source) = self.source_silent.remove(&key) {
                self.source_status(key, source.asset, time, false, generation);
            }
            return;
        };
        // Paused monitors also wait for the binding and refresh on relink.
        self.source_intent
            .insert(key, (asset.clone(), time, playing, generation));
        if self.source_audio.get(&key).is_some_and(|current| {
            current.asset != asset || current.generation != generation || !playing
        }) {
            self.source_audio.remove(&key);
        }
        if self.source_silent.get(&key).is_some_and(|current| {
            current.asset != asset || current.generation != generation || !playing
        }) {
            self.source_silent.remove(&key);
        }
        // Stills have no transport, but their paused monitor requests status.
        let image = self.docs.get(&key).is_some_and(|doc| {
            let record = pm_eval::node::member(doc.document().root(), "assets")
                .and_then(|assets| pm_eval::node::member(Some(assets), asset.as_str()));
            pm_eval::node::is_str(pm_eval::node::member(record, "kind"), "image")
        });
        if image && !playing {
            self.source_timing.remove(&key);
            self.source_status(key, asset, time, false, generation);
            return;
        }
        let path = match self
            .docs
            .get(&key)
            .and_then(|doc| doc.source_media(asset.as_str()))
        {
            None => return, // The host has not answered this source binding yet.
            Some(Some(path)) => path,
            Some(None) => {
                self.source_intent.remove(&key);
                if let Some(cache) = self.source_timing.get_mut(&key) {
                    cache.invalidate(asset.as_str());
                }
                self.source_status_error(
                    key,
                    asset,
                    time,
                    generation,
                    "The source media is unavailable".into(),
                );
                return;
            }
        };
        let timing = match self
            .source_timing
            .entry(key)
            .or_default()
            .open(asset.as_str(), &path)
        {
            Ok(timing) => timing,
            Err(error) => {
                self.source_intent.remove(&key);
                self.source_status_error(
                    key,
                    asset,
                    time,
                    generation,
                    format!("The source media could not start: {error}"),
                );
                return;
            }
        };
        let duration = timing.duration_ticks().expect("validated source duration");
        let time = time.min(duration);
        let playing = playing && time < duration;
        self.source_intent
            .insert(key, (asset.clone(), time, playing, generation));
        if !playing {
            self.source_audio.remove(&key);
            self.source_silent.remove(&key);
            self.source_status(key, asset, time, false, generation);
            return;
        }
        if timing.audio_start.is_none() && !self.source_silent.contains_key(&key) {
            self.source_silent.insert(
                key,
                SourceSilent {
                    asset: asset.clone(),
                    generation,
                    from: time,
                    started: now(),
                    duration,
                    last_sent: 0,
                },
            );
        } else if timing.audio_start.is_some() && !self.source_audio.contains_key(&key) {
            match SourceAudio::start(
                key,
                asset.clone(),
                generation,
                path,
                &timing,
                self.sink.clone(),
            ) {
                Ok(source) => {
                    self.source_audio.insert(key, source);
                }
                Err(error) => {
                    self.source_intent.remove(&key);
                    self.source_status_error(
                        key,
                        asset,
                        time,
                        generation,
                        format!("The source audio could not start: {error}"),
                    );
                    return;
                }
            }
        }
        if let Some(source) = self.source_audio.get(&key) {
            source.playing.store(true, Ordering::Relaxed);
            source.link.send(AudioCtl::Play {
                from: time.get(),
                rate: pm_protocol::common::PlaybackRate { num: 1, den: 1 },
                // The monitor follows this audio's own clock (its status
                // carries the heard time), so it starts once it is ready.
                at: 0,
            });
        }
        self.source_status(key, asset, time, true, generation);
    }

    pub(super) fn source_status(
        &self,
        key: Key,
        asset: pm_protocol::common::AssetId,
        time: Ticks,
        playing: bool,
        generation: u32,
    ) {
        self.source_status_detail(key, asset, time, playing, generation, None);
    }

    pub(super) fn source_status_error(
        &self,
        key: Key,
        asset: pm_protocol::common::AssetId,
        time: Ticks,
        generation: u32,
        error: String,
    ) {
        self.source_status_detail(key, asset, time, false, generation, Some(error));
    }

    pub(super) fn source_status_detail(
        &self,
        key: Key,
        asset: pm_protocol::common::AssetId,
        time: Ticks,
        playing: bool,
        generation: u32,
        error: Option<String>,
    ) {
        let duration = self
            .source_timing
            .get(&key)
            .and_then(|cache| cache.get(asset.as_str()))
            .and_then(|timing| timing.duration_ticks().ok());
        self.send(
            key.0,
            Envelope::new(Message::Event {
                body: Event::Audio(AudioMessage::SourceStatus {
                    asset,
                    time,
                    playing,
                    generation,
                    duration,
                    levels: pm_protocol::clock::ChannelLevels {
                        peak: vec![0.0, 0.0],
                        rms: vec![0.0, 0.0],
                    },
                    error,
                }),
            })
            .with_session(key.1),
        );
    }

    pub(super) fn work_source_monitor(&mut self) {
        let at = now();
        let host = pm_protocol::common::HostMicros::new(at.max(0) as u64)
            .unwrap_or(pm_protocol::common::HostMicros::MAX);
        let mut ended = Vec::new();
        for (&key, source) in &mut self.source_audio {
            if source.link.failed() {
                let error = source
                    .link
                    .status()
                    .error
                    .unwrap_or_else(|| "The audio output is unavailable".into());
                ended.push((
                    key,
                    source.asset.clone(),
                    source.generation,
                    source.duration,
                    Some(format!("The source audio stopped: {error}")),
                ));
                continue;
            }
            if let Some(sample) = source.link.clock().and_then(|clock| clock.latest()) {
                if sample.state != TransportState::Stopped {
                    source.saw_playing = true;
                }
                if let Some(ended_at) =
                    audio_source_end(&sample, source.duration, source.saw_playing, host)
                {
                    ended.push((key, source.asset.clone(), source.generation, ended_at, None));
                }
            } else if at - source.started > AUDIO_START_TIMEOUT_US {
                ended.push((
                    key,
                    source.asset.clone(),
                    source.generation,
                    Ticks::ZERO,
                    Some("The source audio output did not start".into()),
                ));
            }
        }
        for (key, asset, generation, time, error) in ended {
            if let Some(source) = self.source_audio.remove(&key) {
                source.playing.store(false, Ordering::Relaxed);
            }
            self.source_intent.remove(&key);
            match error {
                Some(error) => self.source_status_error(key, asset, time, generation, error),
                None => self.source_status(key, asset, time, false, generation),
            }
        }
        let mut silent_done = Vec::new();
        let mut silent_status = Vec::new();
        for (&key, source) in &mut self.source_silent {
            if at - source.last_sent < 33_000 {
                continue;
            }
            source.last_sent = at;
            let (time, playing) = silent_source_position(source, at);
            silent_status.push((key, source.asset.clone(), time, playing, source.generation));
            if !playing {
                silent_done.push(key);
            }
        }
        for (key, asset, time, playing, generation) in silent_status {
            self.source_status(key, asset, time, playing, generation);
        }
        for key in silent_done {
            self.source_silent.remove(&key);
            self.source_intent.remove(&key);
        }
    }

    /// Draws a source independently of the composition, using the viewer
    /// renderer's decoder, source pass, and cache.
    pub(super) fn run_source_snapshot(
        &mut self,
        job: super::super::snapshot::SnapshotJob,
        started: i64,
    ) {
        use pm_protocol::snapshot::{SnapshotOutcome, SnapshotResult, SnapshotTarget, SourceColor};
        let SnapshotTarget::Source { asset, fit, color } = &job.request.target else {
            return;
        };
        let (asset, fit, color) = (asset.clone(), *fit, *color);
        let Some(doc) = self.docs.get(&job.key) else {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::UnknownSession,
                "The source session has no document.",
            );
            return;
        };
        let revision = Revision::new(doc.revision()).unwrap_or(Revision::ZERO);
        let original = doc.media().path(asset.as_str());
        let record = pm_eval::node::member(doc.document().root(), "assets")
            .and_then(|assets| pm_eval::node::member(Some(assets), asset.as_str()));
        // Interpret Footage says how composition reads the original. The
        // monitor shows that reading of that file, drawn by the engine.
        let footage = record.and_then(pm_render::footage::interpretation);
        let reads_original = color == SourceColor::Interpreted
            || (color == SourceColor::Compositor && footage.is_some());
        let path = if reads_original {
            original.clone()
        } else {
            doc.source_media(asset.as_str()).flatten()
        };
        let Some(path) = path else {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::MediaOffline,
                "The source media is offline.",
            );
            return;
        };
        if color == SourceColor::Compositor
            && !pm_eval::node::is_str(pm_eval::node::member(record, "kind"), "video")
        {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::InvalidMessage,
                "Compositor source colour requires a video asset.",
            );
            return;
        }
        let interpretation = if color == SourceColor::Interpreted {
            footage.clone()
        } else {
            None
        };
        let sequence = record.is_some_and(pm_render::footage::is_sequence);
        let time = pm_render::footage::source_time(record, job.request.time,
            if color == SourceColor::Compositor { footage.as_ref() } else { interpretation.as_ref() });
        let svg_request = match pm_render::Renderer::source_is_svg(&path) {
            Ok(true) if color != SourceColor::Compositor => {
                let intrinsic = (
                    pm_eval::node::finite_number(pm_eval::node::member(record, "w")).unwrap_or(0.0),
                    pm_eval::node::finite_number(pm_eval::node::member(record, "h")).unwrap_or(0.0),
                );
                match pm_render::Renderer::source_svg_request(
                    &path,
                    asset.as_str(),
                    job.request.output_size,
                    fit,
                    intrinsic,
                ) {
                    Ok(request) => Some(request),
                    Err(error) => {
                        self.stats().snapshot_failures += 1;
                        self.snapshot_error(&job, ErrorCode::Internal, &error);
                        return;
                    }
                }
            }
            Ok(_) => None,
            Err(error) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(&job, ErrorCode::MediaOffline, &error);
                return;
            }
        };
        let Some(gpu) = self.gpu.as_mut() else { return };
        let composer = gpu.composer_with(&self.config, self.frame_cache, &mut self.prepared);
        let mut coded_video = None;
        // Coded planes are packed straight into their payload (a shared
        // segment for any real frame): one copy from the decoder's buffer.
        let mut written = None;
        let painted = if let Some(request) = &svg_request {
            composer.renderer().source_svg_rgba8(
                &request.key,
                job.request.output_size,
                fit,
                interpretation.as_ref(),
            )
        } else if color == SourceColor::Compositor {
            let planes = match footage {
                Some(_) => Ok(None),
                // Coded planes up to four times the shown pixels; beyond that
                // (4K in a 720p monitor) the engine resamples first.
                None => composer.renderer().source_video_planes_with(
                    &path,
                    time,
                    sequence,
                    4 * u64::from(job.request.output_size.width)
                        * u64::from(job.request.output_size.height),
                    |length, pack| {
                        Ok((
                            length,
                            super::write_payload(length, |out| {
                                pack(out).map_err(std::io::Error::other)
                            }),
                        ))
                    },
                ),
            };
            match planes {
                Ok(Some((size, payload, video))) => {
                    coded_video = Some((size, video));
                    written = Some(payload);
                    Ok(Some(Vec::new())) // already in `written`
                }
                // Interpret Footage, or a reading Chromium cannot apply
                // exactly (wide gamut, HDR, other curves, or geometry such as
                // rotation): the engine draws the source as composition reads
                // it, and the presenter shows these sRGB pixels as an RGBA frame.
                Ok(None) => composer
                    .renderer()
                    .source_rgba8_composed(
                        &path,
                        time,
                        job.request.output_size,
                        fit,
                        footage.as_ref().unwrap_or(&Default::default()),
                        sequence,
                    )
                    .map(|pixels| {
                        let pixels = super::super::snapshot::encode_rgba8_unorm(
                            &pixels,
                            pm_protocol::snapshot::SnapshotPixels::Straight,
                        );
                        coded_video = Some((
                            job.request.output_size,
                            pm_protocol::snapshot::SourceVideoFrame::srgb_rgba(),
                        ));
                        Some(pixels)
                    }),
                Err(error) => Err(error),
            }
        } else {
            composer
                .renderer()
                .source_rgba8(
                    &path,
                    time,
                    job.request.output_size,
                    fit,
                    interpretation.as_ref(),
                 sequence)
                .map(Some)
        };
        if let (Some(request), Ok(None)) = (&svg_request, &painted) {
            let key = &request.key;
            if self.svg.has_failed(key) {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    &job,
                    ErrorCode::Internal,
                    "The window could not draw this source SVG's Chromium raster.",
                );
                return;
            }
            if job.request.uploaded.contains(&key.upload_key()) {
                self.svg.expect(job.key, key, now());
                self.snapshots.requeue(job);
            } else {
                self.stats().snapshot_needs += 1;
                let mut envelope = Envelope::new(Message::Response {
                    id: job.id,
                    body: Response::Snapshot(Box::new(SnapshotOutcome::NeedsRenderer {
                        revision,
                        fallback: Vec::new(),
                        compat: Vec::new(),
                        svg: vec![request.clone()],
                    })),
                });
                envelope.revision = Some(revision);
                self.send(job.host, envelope);
            }
            return;
        }
        let pixels = match painted {
            Ok(Some(pixels)) => {
                if coded_video.is_some() {
                    pixels
                } else if svg_request.is_some() {
                    super::super::snapshot::encode_straight_rgba8_unorm(&pixels, job.request.pixels)
                } else {
                    super::super::snapshot::encode_rgba8_unorm(&pixels, job.request.pixels)
                }
            }
            Ok(None) => unreachable!("handled missing source SVG above"),
            Err(error) => {
                self.stats().snapshot_failures += 1;
                self.snapshot_error(
                    &job,
                    ErrorCode::Internal,
                    &format!("The source frame could not be drawn: {error}"),
                );
                return;
            }
        };
        let payload = match written {
            Some((length, payload)) => self.track_payload(&job, length, payload),
            None => self.share_pixels(&job, pixels),
        };
        let Some(payload) = payload else {
            return;
        };
        let (size, video) = match coded_video {
            Some((size, video)) => (size, Some(video)),
            None => (job.request.output_size, None),
        };
        let render_us = (now() - started).max(0);
        self.snapshots.observe(render_us as f64);
        {
            let mut stats = self.stats();
            stats.snapshots += 1;
            stats.snapshot_us_max = stats.snapshot_us_max.max(render_us);
        }
        let mut envelope = Envelope::new(Message::Response {
            id: job.id,
            body: Response::Snapshot(Box::new(SnapshotOutcome::Rendered(SnapshotResult {
                revision,
                view_seq: job.request.view_seq,
                width: size.width,
                height: size.height,
                pixels: job.request.pixels,
                payload,
                video,
                origin: [0.0, 0.0],
                layer_box: None,
                raster: None,
                quality: FrameQuality {
                    resolution: PreviewResolution::Full,
                    representation: Representation::Original,
                    draft_3d: false,
                    cached: false,
                    stand_in: false,
                },
                waited_us: (started - job.received).clamp(0, i64::from(u32::MAX)) as u32,
                render_us: render_us.clamp(0, i64::from(u32::MAX)) as u32,
            }))),
        });
        envelope.revision = Some(revision);
        self.send(job.host, envelope);
    }
}

#[cfg(test)]
mod source_audio_tests {
    use super::*;
    use pm_protocol::clock::ClockSample;
    use pm_protocol::common::{HostMicros, PlaybackRate, Seq};

    #[test]
    fn source_audio_clock_runs_through_short_audio_and_holds_media_end() {
        let duration = Ticks::from_whole_seconds(10);
        let sample = ClockSample {
            seq: Seq::new(1).unwrap(),
            epoch: 1,
            state: TransportState::Playing,
            host_time: HostMicros::new(1_000_000).unwrap(),
            // The soundtrack has ended, but the document still has video.
            media_time: Ticks::from_whole_seconds(4),
            rate: PlaybackRate { num: 1, den: 1 },
            output_latency_us: 0,
            sample_rate: 48_000,
            loop_range: None,
        };
        assert_eq!(
            audio_source_end(&sample, duration, true, sample.host_time),
            None
        );
        assert_eq!(
            audio_source_end(&sample, duration, true, HostMicros::new(6_999_999).unwrap()),
            None
        );
        assert_eq!(
            audio_source_end(&sample, duration, true, HostMicros::new(7_000_000).unwrap()),
            Some(duration)
        );
        let stopped = ClockSample {
            state: TransportState::Stopped,
            media_time: duration,
            ..sample
        };
        assert_eq!(
            audio_source_end(&stopped, duration, true, stopped.host_time),
            Some(duration)
        );
    }
}
