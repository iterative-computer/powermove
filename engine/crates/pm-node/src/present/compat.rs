//! Compatibility pixels on the presenter's render thread (the Phase 3c
//! render transport; plan section 20, the `services` `raster` override and
//! the compatibility WebGL2 context).
//!
//! A renderer uploads the pixels it drew for one layer at one frame (a
//! `raster` override's result, an extension's offscreen WebGL2
//! compatibility context) in row bands ([`CompatMessage::Pixels`]); the
//! store assembles each raster, keeps it under its [`CompatKey`] (the
//! producer's cache key: it changes whenever anything that decides the
//! pixels does), and bounds the memory the rasters take
//! ([`MAX_BYTES`], least recently used first). `Forget` drops a layer's
//! rasters at once.
//!
//! The renderer draws a layer from these through [`CompatStore::get`]. The
//! job engine gets the same pixels for export frames through host services
//! (`compatFrame`), never from this store.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_protocol::common::{CompositionId, LayerId, PixelRect, Revision, Ticks};
use pm_protocol::snapshot::{CompatFormat, CompatKey, CompatMessage, CompatNeed, CompatSource};

use super::render::Key;

/// A malformed need must reach the window as a named failure, rather
/// than leave its last frame silently on screen. Validate before encoding;
/// the replacement report itself contains no untrusted scale/crop values.
pub fn needs_event(
    session: pm_protocol::common::SessionId,
    revision: Revision,
    needs: Vec<CompatNeed>,
) -> pm_protocol::envelope::Envelope {
    use pm_protocol::envelope::{Envelope, Event, Message};
    use pm_protocol::error::{EngineError, ErrorCode, ErrorScope, FailureReport};
    use pm_protocol::validate::Validate;
    let message = CompatMessage::Needs { revision, needs };
    let body = match message.validate() {
        Ok(()) => Event::Compat(message),
        Err(why) => Event::Failure(FailureReport {
            revision,
            error: EngineError::new(
                ErrorCode::InvalidMessage,
                ErrorScope::Session { session },
                format!("The compatibility raster request could not be sent: {why}."),
            ),
        }),
    };
    let mut envelope = Envelope::new(Message::Event { body }).with_session(session);
    envelope.revision = Some(revision);
    envelope
}

/// Most bytes of assembled and partial rasters one engine keeps.
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Most rasters being assembled at once (bands still missing).
pub const MAX_PARTIAL: usize = 32;

/// One assembled raster.
#[derive(Debug, Clone, PartialEq)]
pub struct CompatRaster {
    pub width: u32,
    pub height: u32,
    pub format: CompatFormat,
    pub origin: [f64; 2],
    pub scale: f64,
    pub revision: Revision,
    /// The quad in layer pixels and the part of the raster it samples.
    pub size: [f64; 2],
    pub uv: [f64; 4],
    /// Rows top-down, tightly packed.
    pub data: Arc<Vec<u8>>,
}

#[derive(Debug)]
struct Partial {
    raster: CompatRaster,
    data: Vec<u8>,
    rows: Vec<bool>,
    missing: u32,
}

/// One producer answer: a layer at a frame, drawn at a density.
type AnswerId = (
    Key,
    CompositionId,
    LayerId,
    Ticks,
    CompatSource,
    u64,
    Option<PixelRect>,
);

/// What a producer answered, at which document revision.
#[derive(Debug, Clone)]
struct Answer {
    revision: Revision,
    /// The raster drawn; `None`: the layer passed through.
    raster: Option<CompatKey>,
}

/// How long an ask waits before it is sent again (a renderer that missed
/// it: a reload, a closed panel).
pub const ASK_AGAIN: Duration = Duration::from_secs(1);

/// What a message did (counters for the engine's status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// A band was stored; the raster still misses rows.
    Band,
    /// The raster is complete and usable.
    Complete,
    /// A band that did not match the raster it continues (another size or
    /// format under the same key): the partial raster starts over.
    Restarted,
    /// The producer took a layer back: the answers and rasters forgotten.
    Forgot(usize),
    /// The session's producers changed.
    Producers,
    /// A pass-through or a reused raster answered a need.
    Answered,
    /// An answer replaced a different one for the same need at the same
    /// revision: the producer's output changed without a document change
    /// (a paused redraw after its own state changed). `pixels`: a raster.
    Replaced { pixels: bool },
    /// A reuse named a raster the store evicted meanwhile: the need is
    /// asked again at once (not after [`ASK_AGAIN`]), for new pixels.
    Evicted,
    /// Nothing changed, or not a layer raster (the SVG raster messages,
    /// `super::svg`).
    Ignored,
}

impl Applied {
    /// Whether the message redraws a view. Answers only when a frame
    /// waits for them (`waiting`: a playback frame kept in its slot, or the
    /// paused frame): answers for other frames (the lookahead, a preroll)
    /// would present an already complete paused frame again, a
    /// `frame:rendered` today's compositor never sends. A changed producer
    /// list, or answers the producer took back (an invalidation), always:
    /// what is shown may change.
    pub fn redraws(self, waiting: bool) -> bool {
        match self {
            Applied::Producers | Applied::Replaced { .. } => true,
            Applied::Forgot(n) => n > 0,
            Applied::Complete | Applied::Answered | Applied::Evicted => waiting,
            Applied::Band | Applied::Restarted | Applied::Ignored => false,
        }
    }
}

/// A session's view of the store for one render (`pm_render::compat`):
/// answers at `revision` or newer.
pub struct CompatView {
    pub store: Arc<Mutex<CompatStore>>,
    pub key: Key,
    pub revision: Revision,
    /// Records every need the render consulted (a viewer frame: the
    /// presenter's preroll checks the next frames' answers by them).
    pub consulted: Option<Arc<Mutex<Vec<CompatNeed>>>>,
}

impl pm_render::compat::CompatRasters for CompatView {
    fn producing(&self, source: CompatSource) -> bool {
        self.store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .producing(self.key, source)
    }
    fn answer(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        source: CompatSource,
        crop: Option<PixelRect>,
    ) -> pm_render::compat::CompatAnswer {
        if let Some(consulted) = &self.consulted
            && self.producing(source)
        {
            consulted
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(CompatNeed {
                    composition: composition.into(),
                    layer: layer.into(),
                    time: pm_render::compat::ticks_of(time),
                    source,
                    scale,
                    crop,
                });
        }
        self.store.lock().unwrap_or_else(|e| e.into_inner()).answer(
            self.key,
            composition,
            layer,
            time,
            scale,
            source,
            crop,
            self.revision,
        )
    }
}

/// The rasters of every session on this thread.
#[derive(Debug)]
pub struct CompatStore {
    complete: HashMap<(Key, CompatKey), (CompatRaster, u64)>,
    partial: HashMap<(Key, CompatKey), Partial>,
    clock: u64,
    bytes: u64,
    /// Complete rasters evicted for room ([`Self::evicted`]).
    evicted: u64,
    limit: u64,
    /// The producers each session has (`CompatMessage::Producers`).
    producers: HashMap<Key, Vec<CompatSource>>,
    /// Each layer's latest answer.
    answers: HashMap<AnswerId, Answer>,
    /// Needs sent to a renderer and not answered yet, with when and at
    /// which revision.
    asked: HashMap<AnswerId, (Revision, Instant)>,
    /// The rasters as the renderer draws them, made once per raster.
    drawn: HashMap<(Key, CompatKey), Arc<pm_render::compat::CompatRaster>>,
}

fn answer_id(
    key: Key,
    composition: &CompositionId,
    layer: &LayerId,
    time: Ticks,
    source: CompatSource,
    scale: f64,
    crop: Option<PixelRect>,
) -> AnswerId {
    (
        key,
        composition.clone(),
        layer.clone(),
        time,
        source,
        scale.to_bits(),
        crop,
    )
}

impl Default for CompatStore {
    fn default() -> Self {
        Self::with_limit(MAX_BYTES)
    }
}

fn size(width: u32, height: u32, format: CompatFormat) -> u64 {
    u64::from(width) * u64::from(height) * format.bytes_per_pixel()
}

impl CompatStore {
    /// A store that keeps at most `limit` bytes.
    pub fn with_limit(limit: u64) -> Self {
        Self {
            complete: HashMap::new(),
            partial: HashMap::new(),
            clock: 0,
            bytes: 0,
            evicted: 0,
            limit,
            producers: HashMap::new(),
            answers: HashMap::new(),
            asked: HashMap::new(),
            drawn: HashMap::new(),
        }
    }

    /// Applies a renderer's message for session `key`. The message was
    /// validated by the protocol (band bounds and lengths).
    pub fn apply(&mut self, key: Key, message: CompatMessage) -> Applied {
        match message {
            CompatMessage::Forget { composition, layer } => {
                let answers = self.answers.len();
                self.answers
                    .retain(|a, _| !(a.0 == key && a.1 == composition && a.2 == layer));
                let answers = answers - self.answers.len();
                self.asked
                    .retain(|a, _| !(a.0 == key && a.1 == composition && a.2 == layer));
                // Pass-through answers count too: the layer may be drawn
                // differently now.
                Applied::Forgot(answers + self.forget_layer(key, &composition, &layer))
            }
            CompatMessage::Producers { sources } => {
                let before = self
                    .producers
                    .insert(key, sources.clone())
                    .unwrap_or_default();
                // The same list again (a window re-announcing it) changes
                // nothing that is shown.
                let changed = before != sources;
                let gone: Vec<CompatSource> = before
                    .into_iter()
                    .filter(|s| !sources.contains(s))
                    .collect();
                if sources.is_empty() {
                    self.producers.remove(&key);
                }
                if !gone.is_empty() {
                    self.answers
                        .retain(|a, _| !(a.0 == key && gone.contains(&a.4)));
                    self.asked
                        .retain(|a, _| !(a.0 == key && gone.contains(&a.4)));
                    let rasters: Vec<_> = self
                        .complete
                        .keys()
                        .filter(|k| k.0 == key && gone.contains(&k.1.source))
                        .cloned()
                        .collect();
                    for k in rasters {
                        let (raster, _) = self.complete.remove(&k).expect("present");
                        self.bytes -= size(raster.width, raster.height, raster.format);
                        self.drawn.remove(&k);
                    }
                }
                if changed {
                    Applied::Producers
                } else {
                    Applied::Ignored
                }
            }
            CompatMessage::Native(n) => {
                let id = answer_id(
                    key,
                    &n.composition,
                    &n.layer,
                    n.time,
                    n.source,
                    n.scale,
                    n.crop,
                );
                self.asked.remove(&id);
                let replaced = self.replaces(&id, n.revision, None);
                self.answers.insert(
                    id,
                    Answer {
                        revision: n.revision,
                        raster: None,
                    },
                );
                if replaced {
                    Applied::Replaced { pixels: false }
                } else {
                    Applied::Answered
                }
            }
            CompatMessage::Reuse {
                id,
                scale,
                revision,
                crop,
            } => {
                let answer = answer_id(
                    key,
                    &id.composition,
                    &id.layer,
                    id.time,
                    id.source,
                    scale,
                    crop,
                );
                if !self.complete.contains_key(&(key, id.clone())) {
                    // Evicted meanwhile: the frame asks again at once, and
                    // the producer, having answered `reuse` for this need
                    // and revision, sends new pixels.
                    self.asked.remove(&answer);
                    return Applied::Evicted;
                }
                self.asked.remove(&answer);
                let replaced = self.replaces(&answer, revision, Some(&id));
                self.answers.insert(
                    answer,
                    Answer {
                        revision,
                        raster: Some(id),
                    },
                );
                if replaced {
                    Applied::Replaced { pixels: false }
                } else {
                    Applied::Answered
                }
            }
            // Needs travel the other way.
            CompatMessage::Needs { .. } => Applied::Ignored,
            CompatMessage::SvgRequests { .. }
            | CompatMessage::SvgPixels(_)
            | CompatMessage::SvgFailed { .. } => Applied::Ignored,
            CompatMessage::Pixels(band) => {
                let id = (key, band.id.clone());
                let bytes = size(band.width, band.height, band.format);
                let mut restarted = false;
                if let Some(p) = self.partial.get(&id)
                    && (p.raster.width != band.width
                        || p.raster.height != band.height
                        || p.raster.format != band.format)
                {
                    let gone = self.partial.remove(&id).expect("present");
                    self.bytes -= size(gone.raster.width, gone.raster.height, gone.raster.format);
                    restarted = true;
                }
                if !self.partial.contains_key(&id) {
                    while self.partial.len() >= MAX_PARTIAL {
                        // The oldest unfinished raster is abandoned.
                        let oldest = self.partial.keys().next().cloned().expect("non-empty");
                        let gone = self.partial.remove(&oldest).expect("present");
                        self.bytes -=
                            size(gone.raster.width, gone.raster.height, gone.raster.format);
                    }
                    self.make_room(bytes);
                    self.bytes += bytes;
                    let quad = band.size.unwrap_or([
                        f64::from(band.width) / band.scale,
                        f64::from(band.height) / band.scale,
                    ]);
                    self.partial.insert(
                        id.clone(),
                        Partial {
                            raster: CompatRaster {
                                width: band.width,
                                height: band.height,
                                format: band.format,
                                origin: band.origin,
                                scale: band.scale,
                                revision: band.revision,
                                size: quad,
                                uv: band.uv.unwrap_or([0.0, 0.0, 1.0, 1.0]),
                                data: Arc::default(),
                            },
                            data: vec![0; bytes as usize],
                            rows: vec![false; band.height as usize],
                            missing: band.height,
                        },
                    );
                }
                let partial = self.partial.get_mut(&id).expect("inserted");
                let row_bytes = (u64::from(band.width) * band.format.bytes_per_pixel()) as usize;
                let start = band.row as usize * row_bytes;
                partial.data[start..start + band.data.0.len()].copy_from_slice(&band.data.0);
                for r in band.row..band.row + band.rows {
                    let seen = &mut partial.rows[r as usize];
                    if !*seen {
                        *seen = true;
                        partial.missing -= 1;
                    }
                }
                if partial.missing > 0 {
                    return if restarted {
                        Applied::Restarted
                    } else {
                        Applied::Band
                    };
                }
                let Partial {
                    mut raster, data, ..
                } = self.partial.remove(&id).expect("present");
                raster.data = Arc::new(data);
                self.clock += 1;
                // The raster is the producer's answer for its layer at that
                // frame and density.
                let answer = answer_id(
                    key,
                    &band.id.composition,
                    &band.id.layer,
                    band.id.time,
                    band.id.source,
                    band.scale,
                    band.crop,
                );
                self.asked.remove(&answer);
                let replaced = self.replaces(&answer, raster.revision, Some(&band.id));
                self.answers.insert(
                    answer,
                    Answer {
                        revision: raster.revision,
                        raster: Some(band.id.clone()),
                    },
                );
                self.drawn.remove(&id);
                if let Some((old, _)) = self.complete.insert(id, (raster, self.clock)) {
                    self.bytes -= size(old.width, old.height, old.format);
                }
                if replaced {
                    Applied::Replaced { pixels: true }
                } else {
                    Applied::Complete
                }
            }
        }
    }

    /// A complete raster, marked used.
    pub fn get(&mut self, key: Key, id: &CompatKey) -> Option<&CompatRaster> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.complete.get_mut(&(key, id.clone()))?;
        entry.1 = clock;
        Some(&entry.0)
    }

    /// Whether an answer at `revision` drawing `raster` (`None`: a
    /// pass-through) replaces a different answer for the same need at the
    /// same revision.
    fn replaces(&self, id: &AnswerId, revision: Revision, raster: Option<&CompatKey>) -> bool {
        self.answers
            .get(id)
            .is_some_and(|a| a.revision == revision && a.raster.as_ref() != raster)
    }

    /// Evicts least recently used complete rasters until `incoming` fits.
    fn make_room(&mut self, incoming: u64) {
        while self.bytes + incoming > self.limit && !self.complete.is_empty() {
            let oldest = self
                .complete
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
                .expect("non-empty");
            let (gone, _) = self.complete.remove(&oldest).expect("present");
            self.bytes -= size(gone.width, gone.height, gone.format);
            self.drawn.remove(&oldest);
            self.evicted += 1;
        }
    }

    /// Complete rasters evicted for room so far.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    fn forget_layer(&mut self, key: Key, composition: &CompositionId, layer: &LayerId) -> usize {
        let matches = |k: &(Key, CompatKey)| {
            k.0 == key && &k.1.composition == composition && &k.1.layer == layer
        };
        let complete: Vec<_> = self
            .complete
            .keys()
            .filter(|k| matches(k))
            .cloned()
            .collect();
        let partial: Vec<_> = self
            .partial
            .keys()
            .filter(|k| matches(k))
            .cloned()
            .collect();
        for k in &complete {
            let (gone, _) = self.complete.remove(k).expect("present");
            self.bytes -= size(gone.width, gone.height, gone.format);
            self.drawn.remove(k);
        }
        for k in &partial {
            let gone = self.partial.remove(k).expect("present");
            self.bytes -= size(gone.raster.width, gone.raster.height, gone.raster.format);
        }
        complete.len() + partial.len()
    }

    /// Drops everything of a closed session.
    pub fn forget_session(&mut self, key: Key) {
        let complete: Vec<_> = self
            .complete
            .keys()
            .filter(|k| k.0 == key)
            .cloned()
            .collect();
        for k in complete {
            let (gone, _) = self.complete.remove(&k).expect("present");
            self.bytes -= size(gone.width, gone.height, gone.format);
            self.drawn.remove(&k);
        }
        self.producers.remove(&key);
        self.answers.retain(|a, _| a.0 != key);
        self.asked.retain(|a, _| a.0 != key);
        let partial: Vec<_> = self
            .partial
            .keys()
            .filter(|k| k.0 == key)
            .cloned()
            .collect();
        for k in partial {
            let gone = self.partial.remove(&k).expect("present");
            self.bytes -= size(gone.raster.width, gone.raster.height, gone.raster.format);
        }
    }

    /// Whether session `key` has a producer for `source`.
    pub fn producing(&self, key: Key, source: CompatSource) -> bool {
        self.producers
            .get(&key)
            .is_some_and(|s| s.contains(&source))
    }

    /// Whether session `key` has any producer.
    pub fn has_producers(&self, key: Key) -> bool {
        self.producers.get(&key).is_some_and(|s| !s.is_empty())
    }

    /// The answer for a layer of session `key` at a frame of `revision`:
    /// an answer the producer gave at that revision or a newer one.
    #[allow(clippy::too_many_arguments)]
    pub fn answer(
        &mut self,
        key: Key,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        source: CompatSource,
        crop: Option<PixelRect>,
        revision: Revision,
    ) -> pm_render::compat::CompatAnswer {
        use pm_render::compat::CompatAnswer;
        if !self.producing(key, source) {
            return CompatAnswer::Native;
        }
        let id = answer_id(
            key,
            &composition.into(),
            &layer.into(),
            pm_render::compat::ticks_of(time),
            source,
            scale,
            crop,
        );
        let Some(answer) = self
            .answers
            .get(&id)
            .filter(|a| a.revision >= revision)
            .cloned()
        else {
            return CompatAnswer::Missing;
        };
        let Some(raster) = answer.raster else {
            return CompatAnswer::Native;
        };
        let k = (key, raster);
        if let Some(drawn) = self.drawn.get(&k) {
            return CompatAnswer::Raster(drawn.clone());
        }
        let Some(stored) = self.get(key, &k.1).cloned() else {
            return CompatAnswer::Missing;
        };
        let drawn = Arc::new(pm_render::compat::CompatRaster {
            width: stored.width,
            height: stored.height,
            format: stored.format,
            anchor: stored.origin,
            size: stored.size,
            uv: stored.uv,
            data: stored.data.clone(),
        });
        self.drawn.insert(k, drawn.clone());
        CompatAnswer::Raster(drawn)
    }

    /// Of a frame's needs, the ones to send now: not asked yet at
    /// `revision` or newer, or asked longer than [`ASK_AGAIN`] ago. They
    /// are recorded as asked.
    pub fn to_ask(
        &mut self,
        key: Key,
        revision: Revision,
        needs: Vec<CompatNeed>,
        now: Instant,
    ) -> Vec<CompatNeed> {
        needs
            .into_iter()
            .filter(|n| {
                let id = answer_id(
                    key,
                    &n.composition,
                    &n.layer,
                    n.time,
                    n.source,
                    n.scale,
                    n.crop,
                );
                match self.asked.get(&id) {
                    Some((r, at)) if *r >= revision && now.duration_since(*at) < ASK_AGAIN => false,
                    _ => {
                        self.asked.insert(id, (revision, now));
                        true
                    }
                }
            })
            .collect()
    }

    /// A need's name in a snapshot job's `waiting_for` (`compat:` and the
    /// answer's identity).
    pub fn tag(key: Key, need: &CompatNeed) -> String {
        format!(
            "compat:{}:{}:{}:{}:{}:{:?}:{:x}:{}",
            key.0.0,
            key.1.get(),
            need.composition.as_str(),
            need.layer.as_str(),
            need.time.get(),
            need.source,
            need.scale.to_bits(),
            need.crop.map_or_else(String::new, |c| format!(
                "{},{},{},{}",
                c.x, c.y, c.width, c.height
            ))
        )
    }

    /// Whether the need named `tag` is asked and not answered.
    pub fn outstanding(&self, tag: &str) -> bool {
        self.asked
            .keys()
            .any(|(key, composition, layer, time, source, scale, crop)| {
                let need = CompatNeed {
                    composition: composition.clone(),
                    layer: layer.clone(),
                    time: *time,
                    source: *source,
                    scale: f64::from_bits(*scale),
                    crop: *crop,
                };
                Self::tag(*key, &need) == tag
            })
    }

    /// Answers and outstanding asks (status).
    pub fn answer_counts(&self) -> (usize, usize) {
        (self.answers.len(), self.asked.len())
    }

    /// Complete rasters per session, and every byte held.
    pub fn usage(&self) -> (BTreeMap<String, usize>, u64) {
        let mut per = BTreeMap::new();
        for (key, _) in self.complete.keys() {
            *per.entry(format!("{}:{}", key.0.0, key.1.get()))
                .or_insert(0) += 1;
        }
        (per, self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{Bytes, SessionId, Ticks};
    use pm_protocol::host::PortId;
    use pm_protocol::snapshot::{CompatPixels, CompatSource};

    #[test]
    fn an_invalid_density_is_a_named_failure_instead_of_a_silent_missing_need() {
        use pm_protocol::{
            codec,
            envelope::{Event, Message},
        };
        for scale in [24.0, 32.0, 33.0, f64::NAN] {
            let event = needs_event(
                session(1).1,
                Revision::ZERO,
                vec![CompatNeed {
                    composition: "c".into(),
                    layer: "L".into(),
                    time: Ticks::ZERO,
                    source: CompatSource::RasterOverride,
                    scale,
                    crop: None,
                }],
            );
            let decoded = codec::decode(&codec::encode(&event).unwrap()).unwrap();
            match decoded.message {
                Message::Event {
                    body: Event::Compat(_),
                } => assert!(scale <= 32.0),
                Message::Event {
                    body: Event::Failure(report),
                } => {
                    assert!(scale > 32.0 || scale.is_nan());
                    assert_eq!(
                        report.error.code,
                        pm_protocol::error::ErrorCode::InvalidMessage
                    );
                    assert!(report.error.message.contains("compatibility raster"));
                    assert!(report.error.message.contains("scale"));
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    fn session(n: u64) -> Key {
        (PortId(7), SessionId::new(n).unwrap())
    }

    fn id(layer: &str, key: &str) -> CompatKey {
        CompatKey {
            composition: "c".into(),
            layer: layer.into(),
            time: Ticks::ZERO,
            source: CompatSource::RasterOverride,
            key: key.into(),
        }
    }

    fn band(id: CompatKey, w: u32, h: u32, row: u32, rows: u32, fill: u8) -> CompatMessage {
        CompatMessage::Pixels(CompatPixels {
            id,
            revision: Revision::new(3).unwrap(),
            width: w,
            height: h,
            format: CompatFormat::Rgba8Premultiplied,
            origin: [-2.0, -3.0],
            scale: 1.0,
            row,
            rows,
            data: Bytes(vec![fill; (w * rows * 4) as usize]),
            size: None,
            uv: None,
            crop: None,
        })
    }

    #[test]
    fn bands_assemble_in_any_order_into_one_raster() {
        let mut store = CompatStore::default();
        let k = id("L7", "a");
        assert_eq!(
            store.apply(session(1), band(k.clone(), 2, 4, 2, 2, 9)),
            Applied::Band
        );
        assert!(store.get(session(1), &k).is_none(), "not complete yet");
        assert_eq!(
            store.apply(session(1), band(k.clone(), 2, 4, 0, 2, 5)),
            Applied::Complete
        );
        let raster = store.get(session(1), &k).unwrap().clone();
        assert_eq!(
            (raster.width, raster.height, raster.origin),
            (2, 4, [-2.0, -3.0])
        );
        assert_eq!(&raster.data[..16], &[5; 16]);
        assert_eq!(&raster.data[16..], &[9; 16]);
        // Another session's raster under the same key is another raster.
        assert!(store.get(session(2), &k).is_none());
        assert_eq!(store.usage().1, 32);
    }

    #[test]
    fn a_band_of_another_size_restarts_the_raster() {
        let mut store = CompatStore::default();
        let k = id("L7", "a");
        store.apply(session(1), band(k.clone(), 2, 4, 0, 1, 1));
        assert_eq!(
            store.apply(session(1), band(k.clone(), 4, 2, 0, 1, 1)),
            Applied::Restarted
        );
        assert_eq!(
            store.apply(session(1), band(k.clone(), 4, 2, 1, 1, 1)),
            Applied::Complete
        );
        assert_eq!(store.usage().1, 32);
    }

    #[test]
    fn only_a_waiting_frame_is_redrawn_by_answers() {
        // Answers for other frames (the lookahead, a preroll) never present
        // a complete paused frame again.
        for applied in [Applied::Complete, Applied::Answered, Applied::Evicted] {
            assert!(applied.redraws(true), "{applied:?}");
            assert!(!applied.redraws(false), "{applied:?}");
        }
        for applied in [
            Applied::Band,
            Applied::Restarted,
            Applied::Ignored,
            Applied::Forgot(0),
        ] {
            assert!(!applied.redraws(true), "{applied:?}");
        }
        // What is shown may change.
        assert!(Applied::Producers.redraws(false));
        assert!(Applied::Forgot(1).redraws(false));
        // The same producer list again is not a change.
        let mut store = CompatStore::default();
        let producers = || CompatMessage::Producers {
            sources: vec![CompatSource::RasterOverride],
        };
        assert_eq!(store.apply(session(1), producers()), Applied::Producers);
        assert_eq!(store.apply(session(1), producers()), Applied::Ignored);
        assert_eq!(
            store.apply(session(1), CompatMessage::Producers { sources: vec![] }),
            Applied::Producers
        );
    }

    #[test]
    fn a_different_answer_at_the_same_revision_replaces_the_shown_one() {
        let mut store = CompatStore::default();
        let s = session(1);
        let k = id("L7", "a");
        assert_eq!(
            store.apply(s, band(k.clone(), 1, 1, 0, 1, 1)),
            Applied::Complete
        );
        // The same raster again (fresh after a reuse): no change.
        assert_eq!(
            store.apply(s, band(k.clone(), 1, 1, 0, 1, 1)),
            Applied::Complete
        );
        // Another key for the same need and revision: the output changed.
        let other = id("L7", "b");
        assert_eq!(
            store.apply(s, band(other.clone(), 1, 1, 0, 1, 1)),
            Applied::Replaced { pixels: true }
        );
        assert!(Applied::Replaced { pixels: true }.redraws(false));
        // A reuse of the first raster replaces it back.
        let reuse = CompatMessage::Reuse {
            id: k.clone(),
            scale: 1.0,
            revision: Revision::new(3).unwrap(),
            crop: None,
        };
        let replaced = store.apply(s, reuse.clone());
        assert_eq!(replaced, Applied::Replaced { pixels: false });
        assert_eq!(store.apply(s, reuse), Applied::Answered);
    }

    #[test]
    fn forget_drops_a_layer_and_a_closed_session_drops_everything() {
        let mut store = CompatStore::default();
        store.apply(session(1), band(id("L7", "a"), 1, 1, 0, 1, 1));
        store.apply(session(1), band(id("L7", "b"), 1, 1, 0, 1, 1));
        store.apply(session(1), band(id("L8", "a"), 1, 1, 0, 1, 1));
        store.apply(session(1), band(id("L8", "c"), 1, 2, 0, 1, 1));
        // L7's two rasters and the answer naming its latest one.
        assert_eq!(
            store.apply(
                session(1),
                CompatMessage::Forget {
                    composition: "c".into(),
                    layer: "L7".into()
                }
            ),
            Applied::Forgot(3)
        );
        assert_eq!(store.usage().0.values().sum::<usize>(), 1);
        store.forget_session(session(1));
        assert_eq!(store.usage(), (BTreeMap::new(), 0));
    }

    #[test]
    fn a_reuse_of_an_evicted_raster_is_asked_again_at_once() {
        // 1 KiB rasters in a 1 KiB store: the second evicts the first.
        let mut store = CompatStore::with_limit(1024);
        let need = CompatNeed {
            composition: "c".into(),
            layer: "L0".into(),
            time: Ticks::ZERO,
            source: CompatSource::RasterOverride,
            scale: 1.0,
            crop: None,
        };
        let now = Instant::now();
        let r3 = Revision::new(3).unwrap();
        assert_eq!(
            store.to_ask(session(1), r3, vec![need.clone()], now).len(),
            1
        );
        store.apply(session(1), band(id("L0", "k"), 16, 16, 0, 16, 0));
        store.apply(session(1), band(id("L1", "k"), 16, 16, 0, 16, 0));
        assert!(store.get(session(1), &id("L0", "k")).is_none(), "evicted");
        assert_eq!(store.evicted(), 1);
        // The frame at revision 4 asks; the producer says its key is unchanged.
        let r4 = Revision::new(4).unwrap();
        assert_eq!(
            store.to_ask(session(1), r4, vec![need.clone()], now).len(),
            1
        );
        let applied = store.apply(
            session(1),
            CompatMessage::Reuse {
                id: id("L0", "k"),
                scale: 1.0,
                revision: r4,
                crop: None,
            },
        );
        assert_eq!(applied, Applied::Evicted);
        assert!(applied.redraws(true), "the frame tries again");
        // Asked again now, not after ASK_AGAIN.
        assert_eq!(store.to_ask(session(1), r4, vec![need], now).len(), 1);
    }

    #[test]
    fn another_crop_window_is_another_answer() {
        use pm_render::compat::CompatAnswer;
        let mut store = CompatStore::default();
        let s = session(1);
        store.apply(
            s,
            CompatMessage::Producers {
                sources: vec![CompatSource::RasterOverride],
            },
        );
        let window = |x| {
            Some(PixelRect {
                x,
                y: 0,
                width: 256,
                height: 256,
            })
        };
        let r3 = Revision::new(3).unwrap();
        let native = |crop| {
            CompatMessage::Native(pm_protocol::snapshot::CompatNative {
                composition: "c".into(),
                layer: "L".into(),
                time: Ticks::ZERO,
                source: CompatSource::RasterOverride,
                scale: 4.0,
                revision: r3,
                crop,
            })
        };
        assert_eq!(store.apply(s, native(window(0))), Applied::Answered);
        let answer = |store: &mut CompatStore, crop| {
            store.answer(
                s,
                "c",
                "L",
                0.0,
                4.0,
                CompatSource::RasterOverride,
                crop,
                r3,
            )
        };
        assert!(matches!(
            answer(&mut store, window(0)),
            CompatAnswer::Native
        ));
        // The viewport moved to the next bucket: a new need.
        assert!(matches!(
            answer(&mut store, window(256)),
            CompatAnswer::Missing
        ));
        assert!(matches!(answer(&mut store, None), CompatAnswer::Missing));
        // Its tag names the window, so a snapshot waits for that one.
        let need = |crop| CompatNeed {
            composition: "c".into(),
            layer: "L".into(),
            time: Ticks::ZERO,
            source: CompatSource::RasterOverride,
            scale: 4.0,
            crop,
        };
        assert_ne!(
            CompatStore::tag(s, &need(window(0))),
            CompatStore::tag(s, &need(window(256)))
        );
        let asked = store.to_ask(s, r3, vec![need(window(256))], Instant::now());
        assert_eq!(asked.len(), 1);
        assert!(store.outstanding(&CompatStore::tag(s, &need(window(256)))));
        assert!(!store.outstanding(&CompatStore::tag(s, &need(window(512)))));
    }

    #[test]
    fn memory_is_bounded_least_recently_used_first() {
        // 1 KiB rasters (16 × 16 × 4) in a 4 KiB store.
        let mut store = CompatStore::with_limit(4096);
        let big = |name: &str| band(id(name, "k"), 16, 16, 0, 16, 0);
        for n in 0..4 {
            store.apply(session(1), big(&format!("L{n}")));
        }
        assert_eq!(store.usage().1, 4096);
        // L0 used recently: L1 goes first.
        assert!(store.get(session(1), &id("L0", "k")).is_some());
        store.apply(session(1), big("L4"));
        assert!(store.usage().1 <= 4096);
        assert!(store.get(session(1), &id("L1", "k")).is_none());
        assert!(store.get(session(1), &id("L0", "k")).is_some());
        assert!(store.get(session(1), &id("L4", "k")).is_some());
    }
}
