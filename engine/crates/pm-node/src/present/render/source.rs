//! Source-only presenter audio, status, and frame rendering.

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
        sink: Sink,
    ) -> Result<Self, String> {
        let decoder =
            pm_media::audio::AudioDecoder::open(&path, None).map_err(|e| e.to_string())?;
        let duration = decoder.info().duration().unwrap_or(86_400.0).max(0.001);
        let duration_ticks = Ticks::new((duration * TICKS_PER_SECOND as f64).round() as i64)
            .unwrap_or(Ticks::from_whole_seconds(86_400));
        let doc = super::super::source_audio::source_monitor_document(asset.as_str(), duration)?;
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
        if asset.is_none() {
            self.source_intent.remove(&key);
            if let Some(source) = self.source_audio.remove(&key) {
                self.source_status(key, source.asset, time, false, generation);
            }
            if let Some(source) = self.source_silent.remove(&key) {
                self.source_status(key, source.asset, time, false, generation);
            }
            return;
        }
        if let Some(asset) = asset.as_ref() {
            if playing {
                self.source_intent
                    .insert(key, (asset.clone(), time, generation));
            } else {
                self.source_intent.remove(&key);
            }
        }
        if self.source_audio.get(&key).is_some_and(|current| {
            Some(&current.asset) != asset.as_ref() || current.generation != generation || !playing
        }) {
            self.source_audio.remove(&key);
        }
        if self.source_silent.get(&key).is_some_and(|current| {
            Some(&current.asset) != asset.as_ref() || current.generation != generation || !playing
        }) {
            self.source_silent.remove(&key);
        }
        if let Some(asset) = asset {
            if playing
                && !self.source_audio.contains_key(&key)
                && !self.source_silent.contains_key(&key)
            {
                let path = self
                    .docs
                    .get(&key)
                    .and_then(|d| d.source_media(asset.as_str()))
                    .flatten();
                if path.is_none() {
                    if self
                        .docs
                        .get(&key)
                        .and_then(|d| d.source_media(asset.as_str()))
                        .is_none()
                    {
                        return;
                    }
                    self.source_intent.remove(&key);
                    self.source_status_error(
                        key,
                        asset,
                        time,
                        generation,
                        "The source media or browser-playable copy is unavailable".into(),
                    );
                    return;
                }
                if let Some(path) = path {
                    let record = self.docs.get(&key).and_then(|d| {
                        pm_eval::node::member(d.document().root(), "assets")
                            .and_then(|assets| pm_eval::node::member(Some(assets), asset.as_str()))
                    });
                    let video =
                        pm_eval::node::is_str(pm_eval::node::member(record, "kind"), "video");
                    let has_audio =
                        pm_eval::node::truthy(pm_eval::node::member(record, "hasAudio"));
                    let rendition =
                        pm_eval::node::truthy(pm_eval::node::member(record, "rendition"));
                    let original = self
                        .docs
                        .get(&key)
                        .and_then(|d| d.media().path(asset.as_str()));
                    if rendition && original.as_ref() == Some(&path) {
                        self.source_intent.remove(&key);
                        self.source_status_error(
                            key,
                            asset,
                            time,
                            generation,
                            "The browser-playable copy is not available yet".into(),
                        );
                        return;
                    }
                    let duration_seconds =
                        pm_eval::node::finite_number(pm_eval::node::member(record, "dur"))
                            .unwrap_or(86_400.0)
                            .clamp(0.001, 86_400.0);
                    let duration =
                        Ticks::new((duration_seconds * TICKS_PER_SECOND as f64).round() as i64)
                            .unwrap_or(Ticks::from_whole_seconds(86_400));
                    if video && !has_audio {
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
                    } else {
                        match SourceAudio::start(
                            key,
                            asset.clone(),
                            generation,
                            path,
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
                }
            }
            if let Some(source) = self.source_audio.get(&key) {
                source.playing.store(playing, Ordering::Relaxed);
                if playing {
                    source.link.send(AudioCtl::Play {
                        from: time.get(),
                        rate: pm_protocol::common::PlaybackRate { num: 1, den: 1 },
                    });
                } else {
                    source.link.send(AudioCtl::Pause);
                    source.link.send(AudioCtl::Seek(time.get()));
                }
            }
            if playing
                && time
                    >= self
                        .source_silent
                        .get(&key)
                        .map_or(Ticks::from_whole_seconds(86_400), |s| s.duration)
            {
                self.source_silent.remove(&key);
                self.source_intent.remove(&key);
                self.source_status(key, asset, time, false, generation);
            } else {
                self.source_status(key, asset, time, playing, generation);
            }
        }
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
        self.send(
            key.0,
            Envelope::new(Message::Event {
                body: Event::Audio(AudioMessage::SourceStatus {
                    asset,
                    time,
                    playing,
                    generation,
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
        let path = if color != SourceColor::Interpreted {
            doc.source_media(asset.as_str()).flatten()
        } else {
            original.clone()
        };
        let Some(path) = path else {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::MediaOffline,
                "The source media is offline or its browser rendition is unavailable.",
            );
            return;
        };
        let record = pm_eval::node::member(doc.document().root(), "assets")
            .and_then(|assets| pm_eval::node::member(Some(assets), asset.as_str()));
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
        if color != SourceColor::Interpreted
            && record
                .is_some_and(|r| pm_eval::node::truthy(pm_eval::node::member(Some(r), "rendition")))
            && original.as_ref() == Some(&path)
        {
            self.stats().snapshot_failures += 1;
            self.snapshot_error(
                &job,
                ErrorCode::MediaOffline,
                "The browser-playable copy is not available yet.",
            );
            return;
        }
        let interpretation = if color == SourceColor::Interpreted {
            record.and_then(pm_render::footage::interpretation)
        } else {
            None
        };
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
        let painted = if let Some(request) = &svg_request {
            composer.renderer().source_svg_rgba8(
                &request.key,
                job.request.output_size,
                fit,
                interpretation.as_ref(),
            )
        } else if color == SourceColor::Compositor {
            match composer
                .renderer()
                .source_video_planes(&path, job.request.time)
            {
                Ok((size, pixels, video)) => {
                    coded_video = Some((size, video));
                    Ok(Some(pixels))
                }
                Err(error) => Err(error),
            }
        } else {
            composer
                .renderer()
                .source_rgba8(
                    &path,
                    job.request.time,
                    job.request.output_size,
                    fit,
                    interpretation.as_ref(),
                )
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
        let Some(payload) = self.share_pixels(&job, pixels) else {
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
