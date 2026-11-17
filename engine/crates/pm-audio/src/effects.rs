//! Native stereo DSP for validated extension graphs. Construct on a control
//! thread; `process_frame` and `begin_block` never allocate, lock, or call out.
//! The mixer feeds already-retimed PCM after clip gain/fades, before summing.
pub use pm_fx_defs::audio::Definition;
use pm_fx_defs::audio::{Error, Filter, Resolved, Settings};

pub const MAX_BLOCK_FRAMES: usize = 65_536;

const DENORMAL: f64 = 1e-24;
fn clean(v: f64) -> f64 {
    if v.is_finite() && v.abs() >= DENORMAL {
        v
    } else {
        0.0
    }
}
fn db(v: f64) -> f64 {
    10.0f64.powf(v / 20.0)
}

#[derive(Clone, Copy, Debug)]
struct Smooth {
    value: f64,
    target: f64,
    step: f64,
    left: u32,
}
impl Smooth {
    fn new(value: f64) -> Self {
        Self {
            value,
            target: value,
            step: 0.0,
            left: 0,
        }
    }
    fn set(&mut self, target: f64, frames: u32) {
        self.target = target;
        self.left = frames;
        self.step = (target - self.value) / f64::from(frames);
    }
    fn next(&mut self) -> f64 {
        if self.left > 0 {
            self.left -= 1;
            self.value = if self.left == 0 {
                self.target
            } else {
                self.value + self.step
            };
        }
        self.value
    }
}

#[derive(Debug)]
struct Node {
    settings: Resolved,
    controls: [Smooth; 6],
    filter: [[f64; 2]; 2],
    reduction: f64,
    delay: Vec<[f64; 2]>,
    cursor: usize,
    failed: bool,
}

fn coefficients(mode: Filter, frequency: f64, q: f64, gain: f64, rate: u32) -> [f64; 6] {
    // RBJ audio EQ cookbook; shelving uses Q, not a separate slope control.
    // Clamp below Nyquist at low project rates, retaining a stable filter.
    let w = 2.0 * std::f64::consts::PI * frequency.min(f64::from(rate) * 0.49) / f64::from(rate);
    let c = w.cos();
    let s = w.sin();
    let alpha = s / (2.0 * q);
    let a = 10.0f64.powf(gain / 40.0);
    let beta = 2.0 * a.sqrt() * alpha;
    let (b0, b1, b2, a0, a1, a2) = match mode {
        Filter::LowPass => (
            (1.0 - c) / 2.0,
            1.0 - c,
            (1.0 - c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        ),
        Filter::HighPass => (
            (1.0 + c) / 2.0,
            -(1.0 + c),
            (1.0 + c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        ),
        Filter::Notch => (1.0, -2.0 * c, 1.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
        Filter::Peaking => (
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        ),
        Filter::LowShelf => (
            a * ((a + 1.0) - (a - 1.0) * c + beta),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - beta),
            (a + 1.0) + (a - 1.0) * c + beta,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - beta,
        ),
        Filter::HighShelf => (
            a * ((a + 1.0) + (a - 1.0) * c + beta),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - beta),
            (a + 1.0) - (a - 1.0) * c + beta,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - beta,
        ),
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0, 0.0]
}
fn controls(settings: Resolved, rate: u32) -> [f64; 6] {
    let r = f64::from(rate);
    match settings {
        Resolved::Gain { gain_db } => [db(gain_db), 0.0, 0.0, 0.0, 0.0, 0.0],
        Resolved::Biquad {
            mode,
            frequency,
            q,
            gain_db,
        } => coefficients(mode, frequency, q, gain_db, rate),
        Resolved::Compressor {
            threshold,
            ratio,
            attack,
            release,
            knee,
            makeup,
        } => [
            threshold,
            ratio,
            (-1.0 / (attack * r)).exp(),
            (-1.0 / (release * r)).exp(),
            knee,
            db(makeup),
        ],
        Resolved::Limiter { ceiling, release } => [
            db(ceiling),
            (-1.0 / (release * r)).exp(),
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        Resolved::Delay {
            time,
            feedback,
            mix,
            ..
        } => [(time * r).round().max(1.0), feedback, mix, 0.0, 0.0, 0.0],
        Resolved::Gate { enabled } => [if enabled { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0, 0.0, 0.0],
    }
}
impl Node {
    fn new(settings: Resolved, rate: u32) -> Self {
        let len = match settings {
            Resolved::Delay { max_time, .. } => (max_time * f64::from(rate)).ceil() as usize + 1,
            _ => 0,
        };
        Self {
            settings,
            controls: controls(settings, rate).map(Smooth::new),
            filter: [[0.0; 2]; 2],
            reduction: 0.0,
            delay: vec![[0.0; 2]; len],
            cursor: 0,
            failed: false,
        }
    }
    fn reset(&mut self) {
        self.filter = [[0.0; 2]; 2];
        self.reduction = 0.0;
        self.delay.fill([0.0; 2]);
        self.cursor = 0;
        self.failed = false;
    }
    fn process(&mut self, input: [f64; 2]) -> [f64; 2] {
        if self.failed {
            return input;
        }
        let c = self.controls.each_mut().map(Smooth::next);
        let mut out = input;
        match self.settings {
            Resolved::Gain { .. } | Resolved::Gate { .. } => {
                out.iter_mut().for_each(|v| *v *= c[0])
            }
            Resolved::Biquad { .. } => {
                for ch in 0..2 {
                    let state = &mut self.filter[ch];
                    out[ch] = c[0] * input[ch] + state[0];
                    state[0] = clean(c[1] * input[ch] - c[3] * out[ch] + state[1]);
                    state[1] = clean(c[2] * input[ch] - c[4] * out[ch]);
                }
            }
            Resolved::Compressor { .. } => {
                let level = 20.0 * input[0].abs().max(input[1].abs()).max(1e-12).log10();
                let over = level - c[0];
                let reduction = if c[4] > 0.0 && over.abs() < c[4] / 2.0 {
                    (1.0 - 1.0 / c[1]) * (over + c[4] / 2.0).powi(2) / (2.0 * c[4])
                } else {
                    (1.0 - 1.0 / c[1]) * over.max(0.0)
                };
                let a = if reduction > self.reduction {
                    c[2]
                } else {
                    c[3]
                };
                self.reduction = clean(a * self.reduction + (1.0 - a) * reduction);
                let gain = db(-self.reduction) * c[5];
                out.iter_mut().for_each(|v| *v *= gain);
            }
            Resolved::Limiter { .. } => {
                let peak = input[0].abs().max(input[1].abs());
                let reduction = if peak > c[0] { 1.0 - c[0] / peak } else { 0.0 };
                self.reduction = reduction.max(c[1] * self.reduction);
                out.iter_mut()
                    .for_each(|v| *v = (*v * (1.0 - self.reduction)).clamp(-c[0], c[0]));
            }
            Resolved::Delay { .. } => {
                // Fractional read interpolation makes a live time edit glide;
                // integer settings retain an exact sample-aligned impulse.
                let time = c[0].clamp(1.0, (self.delay.len() - 1) as f64);
                let frames = time.floor() as usize;
                let a = (self.cursor + self.delay.len() - frames) % self.delay.len();
                let b = (a + self.delay.len() - 1) % self.delay.len();
                let frac = time - frames as f64;
                for ch in 0..2 {
                    let wet = self.delay[a][ch] * (1.0 - frac) + self.delay[b][ch] * frac;
                    self.delay[self.cursor][ch] = clean(input[ch] + wet * c[1]);
                    out[ch] = input[ch] * (1.0 - c[2]) + wet * c[2];
                }
                self.cursor = (self.cursor + 1) % self.delay.len();
            }
        }
        if out
            .iter()
            .any(|v| !v.is_finite() || v.abs() > f64::from(f32::MAX))
        {
            self.reset();
            self.failed = true;
            input
        } else {
            out.map(clean)
        }
    }
}

#[derive(Debug)]
pub struct Chain {
    identity: String,
    nodes: Vec<Node>,
    rate: u32,
    next: Option<i64>,
    last: [f64; 2],
    seek_from: [f64; 2],
    seek_left: u32,
    block_first: Option<i64>,
    block_output: Vec<[f32; 2]>,
    recorded: usize,
    replay: usize,
    replaying: bool,
}
impl Chain {
    pub fn new(settings: &Settings, rate: u32) -> Result<Self, Error> {
        if !(8_000..=192_000).contains(&rate) {
            return Err(Error("sampleRate: expected 8000..192000".into()));
        }
        Ok(Self {
            identity: String::new(),
            nodes: settings
                .nodes()
                .iter()
                .map(|s| Node::new(*s, rate))
                .collect(),
            rate,
            next: None,
            last: [0.0; 2],
            seek_from: [0.0; 2],
            seek_left: 0,
            block_first: None,
            block_output: vec![[0.0; 2]; MAX_BLOCK_FRAMES],
            recorded: 0,
            replay: 0,
            replaying: false,
        })
    }
    /// Copy compatible history into already allocated state at a graph handoff.
    /// The outgoing glide keeps its own history; no state is shared or locked.
    pub(crate) fn inherit(&mut self, old: &Self) {
        if self.identity != old.identity
            || self.rate != old.rate
            || self.nodes.len() != old.nodes.len()
            || self.nodes.iter().zip(&old.nodes).any(|(a, b)| {
                std::mem::discriminant(&a.settings) != std::mem::discriminant(&b.settings)
                    || a.delay.len() != b.delay.len()
            })
        {
            return;
        }
        for (node, previous) in self.nodes.iter_mut().zip(&old.nodes) {
            for (control, before) in node.controls.iter_mut().zip(&previous.controls) {
                let target = control.target;
                *control = *before;
                if target != before.target {
                    control.set(target, self.rate / 100);
                }
            }
            node.filter = previous.filter;
            node.reduction = previous.reduction;
            node.delay.copy_from_slice(&previous.delay);
            node.cursor = previous.cursor;
        }
        self.next = old.next;
        self.last = old.last;
        self.seek_from = old.seek_from;
        self.seek_left = old.seek_left;
    }
    /// Change settings without replacing history. Called by the owner between
    /// blocks. A different topology or delay capacity needs a new off-thread chain.
    pub fn retarget(&mut self, settings: &Settings) -> Result<(), Error> {
        if self.nodes.len() != settings.nodes().len() || self.nodes.iter().zip(settings.nodes()).any(|(a,b)| std::mem::discriminant(&a.settings) != std::mem::discriminant(b) || matches!((a.settings,*b), (Resolved::Delay { max_time:a,.. },Resolved::Delay { max_time:b,.. }) if a != b)) { return Err(Error("graph: topology changed; rebuild on control thread".into())); }
        self.block_first = None;
        for (node, settings) in self.nodes.iter_mut().zip(settings.nodes()) {
            node.settings = *settings;
            for (s, target) in node.controls.iter_mut().zip(controls(*settings, self.rate)) {
                s.set(target, self.rate / 100);
            }
        }
        Ok(())
    }
    /// Call once at the beginning of a contiguous block. Seek/loop discards all
    /// histories (including delay tails), with a 5 ms output bridge from the
    /// last sample heard. The first sample at a seek is exactly that sample.
    pub fn begin_block(&mut self, first: i64) {
        // A producer retries the same range while a different voice decodes.
        // Reuse this voice's committed output without advancing its history.
        self.replaying = self.block_first == Some(first);
        self.replay = 0;
        if self.replaying {
            return;
        }
        self.block_first = Some(first);
        self.recorded = 0;
        if self.next.is_some_and(|next| next != first) {
            for node in &mut self.nodes {
                node.reset();
            }
            self.seek_from = self.last;
            self.seek_left = self.rate / 200;
        }
        self.next = Some(first);
    }
    pub fn process_frame(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.replaying && self.replay < self.recorded {
            let out = self.block_output[self.replay];
            self.replay += 1;
            return out;
        }
        self.replaying = false;
        let mut out = input.map(|v| clean(f64::from(v)));
        for node in &mut self.nodes {
            out = node.process(out);
        }
        if self.seek_left > 0 {
            let t = 1.0 - f64::from(self.seek_left) / f64::from(self.rate / 200);
            for (o, from) in out.iter_mut().zip(self.seek_from) {
                *o = from * (1.0 - t) + *o * t;
            }
            self.seek_left -= 1;
        }
        self.last = out;
        if let Some(next) = &mut self.next {
            *next = next.saturating_add(1);
        }
        let out = out.map(|v| clean(v) as f32);
        if self.recorded < self.block_output.len() {
            self.block_output[self.recorded] = out;
            self.recorded += 1;
        }
        out
    }
    /// An explicit seek is distinct from a producer retry of the same block.
    pub fn abandon_retry(&mut self) {
        self.block_first = None;
        self.replaying = false;
        // Force the next block to reset history even when it seeks exactly to
        // the previously rendered block's end.
        if self.next.is_some() {
            self.next = Some(i64::MIN);
        }
    }
}

/// Winning registrations from the extension contribution registry, captured at
/// one revision. Removing a provider removes its definition from this map.
pub type Registry = std::collections::BTreeMap<String, pm_fx_defs::audio::Validated>;

/// All clip state is allocated by the control caller before installation.
/// Ownership transfer into the producer is exclusive; processing needs no Arc,
/// lock or atomic reference count. Retired mixer voices own their old state.
pub(crate) struct PreparedVoice {
    pub id: String,
    pub chains: Vec<Chain>,
    pub matrix: Vec<Vec<f32>>,
}

pub struct Prepared {
    pub(crate) voices: Vec<PreparedVoice>,
    pub problems: Vec<(String, String, String)>,
}
impl Prepared {
    pub fn build(
        doc: &pm_model::Document,
        graph: &crate::MixGraph,
        registry: &Registry,
        rate: u32,
    ) -> crate::Result<Self> {
        use pm_eval::node;
        let comps = graph.resolve(doc)?;
        let mut voices = Vec::with_capacity(graph.voices.len());
        let mut problems = Vec::new();
        for voice in &graph.voices {
            let mut chains = Vec::new();
            if let Some(level) = voice.levels.last() {
                let layer = &comps[level.comp].layers()[level.layer];
                let effects = node::array(node::member(Some(layer), "audioFx")).unwrap_or_default();
                if effects.len() > 16 {
                    problems.push((
                        voice.name.clone(),
                        String::new(),
                        "Audio effects: audioFx count exceeds 16; extra effects bypassed".into(),
                    ));
                }
                for effect in effects.iter().take(16) {
                    if matches!(
                        node::member(Some(effect), "on").map(pm_model::Node::kind),
                        Some(pm_model::NodeKind::Bool(false))
                    ) {
                        continue;
                    }
                    let id = node::string(node::member(Some(effect), "type"))
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default();
                    let name = node::string(node::member(Some(effect), "name"))
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_else(|| id.clone());
                    let error = if let Some(def) = registry.get(&id) {
                        let params = node::member(Some(effect), "p").map(pm_model::Node::to_value);
                        let params = params
                            .as_ref()
                            .map(pm_model::json::to_canonical_string)
                            .unwrap_or_else(|| "{}".into());
                        match serde_json::from_str(&params)
                            .map_err(|e| Error(format!("params: {e}")))
                            .and_then(|values| def.resolve(&values))
                            .and_then(|settings| Chain::new(&settings, rate))
                        {
                            Ok(mut chain) => {
                                let instance = node::string(node::member(Some(effect), "id"))
                                    .map(|s| s.to_string_lossy())
                                    .unwrap_or_default();
                                chain.identity = format!("{id}:{instance}");
                                chains.push(chain);
                                None
                            }
                            Err(e) => {
                                Some(format!("Audio effect “{name}” ({id}) is bypassed: {e}"))
                            }
                        }
                    } else {
                        Some(format!(
                            "Audio effect “{name}” ({id}) is unavailable; bypassed"
                        ))
                    };
                    if let Some(error) = error {
                        problems.push((voice.name.clone(), id, error));
                    }
                }
            }
            let matrix = if chains.is_empty() {
                vec![]
            } else {
                crate::layout::matrix(&voice.source.info.speakers, &crate::layout::stereo())
            };
            voices.push(PreparedVoice {
                id: voice.id.clone(),
                chains,
                matrix,
            });
        }
        Ok(Self { voices, problems })
    }
}

/// Decode the audio part of a normal contribution publication. Owner metadata
/// is transport-only and never becomes part of an extension's DSP schema.
pub fn registrations(publication: &serde_json::Value) -> Result<Registry, Error> {
    let mut registry = Registry::new();
    for item in publication["audioEffects"].as_array().into_iter().flatten() {
        let mut item = item.clone();
        if let Some(object) = item.as_object_mut() {
            object.remove("owner");
        }
        let validated = pm_fx_defs::audio::parse(item)?;
        registry.insert(validated.definition().id.clone(), validated);
    }
    Ok(registry)
}

/// Native seeds sit below extension providers, as the visual registry's do.
pub fn builtin_registry() -> Registry {
    pm_fx_defs::audio::builtins()
        .iter()
        .map(|d| (d.definition().id.clone(), d.clone()))
        .collect()
}
