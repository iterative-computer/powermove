//! Asks the font registry, ahead and off the render thread, for every face
//! the open composition's text lays out with.
//!
//! A text layer whose faces the registry has not answered yet costs the
//! first frame that lays it out a round trip to the main thread's font
//! service (`pm_text::host`), and the service's first answer for a family
//! can be long: a variable source (`fontAxis.*`) is found by scanning the
//! installed fonts, and its file is read and registered. In `launch.pmv`
//! that is `Geist Width` (`Text edit · Powermove → Text`), about 0.55 s,
//! first met at 13.733 s while playing. The registry keeps every answer it
//! gives (`CoreTextRegistry`: families, variable sources, files, fallback
//! manifests), so the same questions asked earlier leave that frame a
//! cached answer.
//!
//! [`FontWarmer`] asks them when a frame's scene is new (document edition,
//! composition, font generation). Its thread walks the composition tree
//! (nested compositions too, once each, whatever a layer's span or
//! visibility) and lays out every distinct text content the text layers can
//! show (each font property's static value and keyframe values, and the
//! range styles over them) with the renderer's own level 1 layout
//! (`CompatMeasure` over a [`BrokerResolver`] on the renderer's broker,
//! as `pm_render::text::host::EngineFonts`). The layout asks exactly the
//! questions a frame's layout asks; the answers are the registry's either
//! way, so pictures and exports do not change.
//!
//! Asking ahead is advice. An ask that fails or is not taken up in time is
//! dropped, never reported, and the frame that needs the face asks as it
//! always did. A newer scene of the same composition (an edit) cancels the
//! walk between contents; the thread is never joined (it may be waiting on
//! an ask), so stopping never waits.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use pm_eval::node::{array, member};
use pm_eval::text_measure::CompatMeasure;
use pm_model::{Node, NodeKind};
use pm_text::compat::TextContent;
use pm_text::host::{BrokerResolver, FontBroker, FontHealth};

/// Most distinct text contents warmed for one composition tree.
const MAX_CONTENTS: usize = 4096;
/// Most variants of one layer or range style (the product of its keyed font
/// properties' values); past it, each keyed property's values are taken
/// one at a time.
const MAX_LAYER_VARIANTS: usize = 64;
/// Characters of one content laid out ahead (the frame lays out the rest).
const MAX_TEXT_CHARS: usize = 4096;
/// Contents remembered as asked for; past it the warm fonts start over.
const MAX_WARMED: usize = 16_384;
/// Scenes remembered as handed over ([`FontWarmer::request`]).
const REMEMBERED_SCENES: usize = 4;
/// How long a warm ask waits for the font service to take it up. Short:
/// a service busy for longer is left alone, and the frame asks as before.
const WARM_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a text layer's `d` key chooses its faces (`canvas_font`,
/// `variable_family`) or the characters that pick fallbacks. The others
/// (alignment, leading, tracking, box) move glyphs, never pick a face.
fn font_key(key: &str) -> bool {
    matches!(key, "text" | "font" | "weight" | "size" | "italic") || key.starts_with("fontAxis.")
}

/// One text content to lay out ahead: what a text layer's resolved content
/// reads as (`pm_eval::text_geom::text_content`), font keys only.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WarmText {
    pub(crate) content: TextContent,
    /// The earliest a layer showing it starts, in seconds of the open
    /// composition (nested offsets summed; time remapping ignored): the
    /// order contents are warmed in, nothing else.
    pub(crate) start: f64,
}

/// The distinct text contents of `comp`'s tree, earliest first.
pub(crate) fn text_contents(doc: &pm_model::Document, comp: &str) -> Vec<WarmText> {
    let Ok(resolved) = pm_eval::Comp::resolve(doc, comp) else {
        return Vec::new();
    };
    let comps = resolved.root().get("comps");
    // A precomp's composition as the plan finds it (`comps[id]`), or, failing
    // that, however the document resolves the id (`draws_visual_media`).
    let layers_of = |id: &str| {
        comps
            .and_then(|comps| comps.get(id))
            .and_then(|nested| array(member(Some(nested), "layers")))
            .or_else(|| pm_eval::Comp::resolve(doc, id).ok().map(|c| c.layers()))
    };
    let first = resolved.layers();
    // Layer arrays already queued: a precomp cycle is walked once.
    let mut seen: Vec<*const Node> = vec![first.as_ptr()];
    let mut stack: Vec<(&[Node], f64)> = vec![(first, 0.0)];
    let mut out: Vec<WarmText> = Vec::new();
    let mut known: HashMap<String, usize> = HashMap::new();
    let mut variants = Vec::new();
    'walk: while let Some((layers, offset)) = stack.pop() {
        for layer in layers {
            let from = pm_eval::node::to_number(member(Some(layer), "from"));
            let start = offset + if from.is_finite() { from } else { 0.0 };
            let ty =
                pm_eval::node::string(member(Some(layer), "type")).map(|t| t.to_string_lossy());
            match ty.as_deref() {
                Some("text") => {
                    variants.clear();
                    if let Some(d) = member(Some(layer), "d") {
                        layer_contents(d, &mut variants);
                    }
                    for content in variants.drain(..) {
                        let key = format!("{content:?}");
                        match known.get(&key) {
                            Some(&i) => out[i].start = out[i].start.min(start),
                            None => {
                                known.insert(key, out.len());
                                out.push(WarmText { content, start });
                                if out.len() >= MAX_CONTENTS {
                                    break 'walk;
                                }
                            }
                        }
                    }
                }
                Some("precomp") => {
                    if let Some(nested) = member(member(Some(layer), "d"), "comp")
                        .and_then(|id| pm_eval::node::string(Some(id)))
                        .and_then(|id| layers_of(id.to_string_lossy().as_str()))
                        && !seen.contains(&nested.as_ptr())
                    {
                        seen.push(nested.as_ptr());
                        stack.push((nested, start));
                    }
                }
                _ => {}
            }
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// The contents a text layer's `d` can resolve to: its font keys at their
/// static values and keyframe values, then each range style's over them.
/// Expression-driven values are not known ahead: their static value is
/// taken, and a frame meeting another asks for it then.
fn layer_contents(d: &Node, out: &mut Vec<TextContent>) {
    let NodeKind::Object(map) = d.kind() else {
        return;
    };
    let mut fixed = serde_json::Map::new();
    let mut keyed: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
    for (k, v) in map.iter() {
        let key = k.to_string_lossy();
        if font_key(&key) {
            set(&mut fixed, &mut keyed, key, candidates(v));
        }
    }
    variants(&fixed, &keyed, out);
    // `posed_glyphs`: a range style's `p` entries replace the layer's own
    // for the glyphs in its range.
    for style in array(member(Some(d), "styles")).unwrap_or(&[]) {
        let Some(NodeKind::Object(p)) = member(Some(style), "p").map(Node::kind) else {
            continue;
        };
        let (mut fixed, mut keyed) = (fixed.clone(), keyed.clone());
        let mut styled = false;
        for (k, v) in p.iter() {
            let key = k.to_string_lossy();
            if font_key(&key) && key != "text" {
                styled = true;
                set(&mut fixed, &mut keyed, key, candidates(v));
            }
        }
        if styled {
            variants(&fixed, &keyed, out);
        }
    }
}

/// Puts `key`'s values among the fixed (one value) or keyed (several) ones,
/// replacing what it had.
fn set(
    fixed: &mut serde_json::Map<String, serde_json::Value>,
    keyed: &mut Vec<(String, Vec<serde_json::Value>)>,
    key: String,
    mut values: Vec<serde_json::Value>,
) {
    fixed.remove(&key);
    keyed.retain(|(k, _)| *k != key);
    match values.len() {
        0 => {}
        1 => {
            fixed.insert(key, values.remove(0));
        }
        _ => keyed.push((key, values)),
    }
}

/// The values a stored property can take: an animated one's (`{v, kf}`)
/// static value and every keyframe's, distinct; a plain one's value.
fn candidates(v: &Node) -> Vec<serde_json::Value> {
    let Some(keyframes) = array(member(Some(v), "kf")) else {
        let value = node_json(v);
        return if value.is_null() {
            Vec::new()
        } else {
            vec![value]
        };
    };
    let mut out: Vec<serde_json::Value> = Vec::new();
    let values = member(Some(v), "v")
        .into_iter()
        .chain(keyframes.iter().filter_map(|k| member(Some(k), "v")));
    for value in values.map(node_json) {
        if !value.is_null() && !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

/// Every combination of the keyed values over the fixed ones (bounded).
fn variants(
    fixed: &serde_json::Map<String, serde_json::Value>,
    keyed: &[(String, Vec<serde_json::Value>)],
    out: &mut Vec<TextContent>,
) {
    let total = keyed
        .iter()
        .try_fold(1usize, |n, (_, values)| n.checked_mul(values.len()));
    if total.is_some_and(|n| n <= MAX_LAYER_VARIANTS) {
        let mut at = vec![0usize; keyed.len()];
        loop {
            let mut object = fixed.clone();
            for ((key, values), i) in keyed.iter().zip(&at) {
                object.insert(key.clone(), values[*i].clone());
            }
            out.push(content(object));
            // The next combination, odometer order.
            let mut j = 0;
            loop {
                if j == at.len() {
                    return;
                }
                at[j] += 1;
                if at[j] < keyed[j].1.len() {
                    break;
                }
                at[j] = 0;
                j += 1;
            }
        }
    }
    // Too many together: each keyed property's values on their own, the
    // others at their first.
    let mut base = fixed.clone();
    for (key, values) in keyed {
        base.insert(key.clone(), values[0].clone());
    }
    out.push(content(base.clone()));
    for (key, values) in keyed {
        for value in &values[1..] {
            let mut object = base.clone();
            object.insert(key.clone(), value.clone());
            out.push(content(object));
        }
    }
}

/// The content as the layout reads it (`TextContent::from_json`, as the
/// renderer's `text_content`), its text bounded.
fn content(object: serde_json::Map<String, serde_json::Value>) -> TextContent {
    let mut content = TextContent::from_json(&serde_json::Value::Object(object));
    if let Some((cut, _)) = content.text.char_indices().nth(MAX_TEXT_CHARS) {
        content.text.truncate(cut);
    }
    content
}

/// A node as JSON (as `pm_render::text`'s).
fn node_json(node: &Node) -> serde_json::Value {
    match node.kind() {
        NodeKind::Null => serde_json::Value::Null,
        NodeKind::Bool(b) => (*b).into(),
        NodeKind::Number(n) => {
            serde_json::Number::from_f64(*n).map_or(serde_json::Value::Null, Into::into)
        }
        NodeKind::String(s) => s.to_string_lossy().into(),
        NodeKind::Array(items) => items.iter().map(node_json).collect(),
        NodeKind::Object(map) => map
            .iter()
            .map(|(k, v)| (k.to_string_lossy(), node_json(v)))
            .collect(),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A scene to warm.
struct Job {
    doc: Arc<pm_model::Document>,
    comp: String,
    broker: FontBroker,
    generation: u64,
    /// [`FontWarmer::reset`]s before it (a replaced broker).
    fonts: u64,
}

#[derive(Default)]
struct Slot {
    /// Scenes not taken yet, oldest first: one per composition (a newer
    /// one of the same composition replaces it), a few at most.
    jobs: VecDeque<Job>,
    stop: bool,
    /// Jobs finished, cancelled or given up (tests).
    completed: u64,
}

#[derive(Default)]
struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
}

impl Shared {
    /// Whether a job warming `comp` should stop: a newer scene of the same
    /// composition waits (an edit), or the warmer is gone.
    fn cancelled(&self, comp: &str) -> bool {
        let slot = lock(&self.slot);
        slot.stop || slot.jobs.iter().any(|j| j.comp == comp)
    }
}

/// The warm thread's fonts: one replay over the broker, kept while its
/// generation and broker are, and what it has laid out.
struct Warm {
    generation: u64,
    fonts: u64,
    health: FontHealth,
    measure: CompatMeasure<BrokerResolver>,
    /// Contents laid out (`{content:?}`).
    warmed: HashSet<String>,
    /// Characters laid out per face description (`{content:?}` without its
    /// text).
    chars: HashMap<String, HashSet<char>>,
}

impl Warm {
    fn new(job: &Job, timeout: Duration) -> Self {
        let health = FontHealth::default();
        Warm {
            generation: job.generation,
            fonts: job.fonts,
            measure: CompatMeasure::new(
                BrokerResolver::new(job.broker.clone(), health.clone()).with_timeout(timeout),
            ),
            health,
            warmed: HashSet::new(),
            chars: HashMap::new(),
        }
    }
}

/// Lays `job`'s contents out until done, cancelled, or the service stops
/// answering (then the fonts are made again for the next job).
fn warm(shared: &Shared, state: &mut Option<Warm>, job: Job, timeout: Duration) {
    // Fresh fonts for another generation or broker, and past the bound (the
    // measurer's caches go with them).
    if state.as_ref().is_none_or(|w| {
        w.generation != job.generation || w.fonts != job.fonts || w.warmed.len() > MAX_WARMED
    }) {
        *state = Some(Warm::new(&job, timeout));
    }
    let Some(w) = state.as_mut() else {
        return;
    };
    let contents = text_contents(&job.doc, &job.comp);
    // The document is not kept while asking.
    let Job { doc, comp, .. } = job;
    drop(doc);
    for WarmText { content, .. } in contents {
        if shared.cancelled(&comp) {
            return;
        }
        let key = format!("{content:?}");
        if w.warmed.contains(&key) {
            continue;
        }
        let face_key = format!(
            "{:?}",
            TextContent {
                text: String::new(),
                ..content.clone()
            }
        );
        // A text of simple characters all laid out already in these faces
        // asks nothing new: its fallbacks are per character. Others
        // (combining marks, symbols, emoji and their selectors, scripts
        // whose fallback depends on neighbours) are laid out themselves.
        let simple = content.text.chars().all(|c| (c as u32) < 0x300);
        let covered = simple
            && w.chars
                .get(&face_key)
                .is_some_and(|known| content.text.chars().all(|c| known.contains(&c)));
        if !covered {
            // The questions `line_runs` asks are the frame's: its
            // measurer's family cascade, standard and variable faces, and
            // each character's fallback (the frame's per-line `text_runs`
            // ask the same of the same faces).
            drop(w.measure.compat().line_runs(&content));
            if w.health.take_failure().is_some() {
                // Not answered: the next job starts over with fresh fonts.
                *state = None;
                return;
            }
        }
        w.chars
            .entry(face_key)
            .or_default()
            .extend(content.text.chars());
        w.warmed.insert(key);
    }
}

fn run(shared: Arc<Shared>, timeout: Duration) {
    let mut state: Option<Warm> = None;
    loop {
        let job = {
            let mut slot = lock(&shared.slot);
            loop {
                if slot.stop {
                    return;
                }
                if let Some(job) = slot.jobs.pop_front() {
                    break job;
                }
                slot = shared.wake.wait(slot).unwrap_or_else(|e| e.into_inner());
            }
        };
        // A layout that panics loses this job only (and its fonts).
        let warmed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            warm(&shared, &mut state, job, timeout);
        }));
        if warmed.is_err() {
            state = None;
        }
        lock(&shared.slot).completed += 1;
    }
}

/// Asks the font registry ahead for the faces of the scenes a renderer draws
/// (module docs). One per [`crate::render::Engine`]; its thread starts with
/// the first request.
pub(crate) struct FontWarmer {
    shared: Arc<Shared>,
    started: bool,
    /// The scenes last handed over, newest first: document edition,
    /// composition, font generation. A few, so viewers taking turns on the
    /// shared worker do not hand theirs over again at every turn.
    requested: VecDeque<(u64, String, u64)>,
    /// [`Self::reset`]s so far.
    fonts: u64,
    timeout: Duration,
}

impl Default for FontWarmer {
    fn default() -> Self {
        FontWarmer {
            shared: Arc::default(),
            started: false,
            requested: VecDeque::new(),
            fonts: 0,
            timeout: WARM_TIMEOUT,
        }
    }
}

impl FontWarmer {
    #[cfg(test)]
    pub(crate) fn with_timeout(timeout: Duration) -> Self {
        // `FontWarmer` implements `Drop`: no struct update from a default.
        let mut warmer = Self::default();
        warmer.timeout = timeout;
        warmer
    }

    /// Hands `doc`'s `comp` over to be warmed with `fonts`, unless it is one
    /// of the scenes handed over lately (the same edition, composition and
    /// font generation). Cheap when it is: called before every frame. True
    /// when it was handed over.
    pub(crate) fn request(
        &mut self,
        doc: &Arc<pm_model::Document>,
        comp: &str,
        fonts: &pm_render::text::host::RenderFonts,
        edition: u64,
    ) -> bool {
        let generation = fonts.generation();
        if self
            .requested
            .iter()
            .any(|(e, c, g)| *e == edition && c == comp && *g == generation)
        {
            return false;
        }
        self.requested.truncate(REMEMBERED_SCENES - 1);
        self.requested
            .push_front((edition, comp.to_owned(), generation));
        if !self.started {
            let shared = self.shared.clone();
            let timeout = self.timeout;
            let spawned = std::thread::Builder::new()
                .name("pm-font-warm".into())
                .spawn(move || run(shared, timeout));
            if spawned.is_err() {
                // No thread, no warming: frames ask as before.
                return false;
            }
            self.started = true;
        }
        let mut slot = lock(&self.shared.slot);
        slot.jobs.retain(|j| j.comp != comp);
        if slot.jobs.len() >= REMEMBERED_SCENES {
            slot.jobs.pop_front();
        }
        slot.jobs.push_back(Job {
            doc: doc.clone(),
            comp: comp.to_owned(),
            broker: fonts.broker.clone(),
            generation,
            fonts: self.fonts,
        });
        drop(slot);
        self.shared.wake.notify_one();
        true
    }

    /// The renderer's fonts were replaced (`Engine::set_fonts`): the next
    /// request warms with the new broker from scratch.
    pub(crate) fn reset(&mut self) {
        self.requested.clear();
        self.fonts += 1;
    }

    /// Jobs the thread has finished, cancelled or given up.
    #[cfg(test)]
    pub(crate) fn completed(&self) -> u64 {
        lock(&self.shared.slot).completed
    }
}

impl Drop for FontWarmer {
    fn drop(&mut self) {
        let mut slot = lock(&self.shared.slot);
        slot.stop = true;
        // No document is kept by a warmer that is gone.
        slot.jobs.clear();
        drop(slot);
        self.shared.wake.notify_all();
        // Not joined: the thread may be waiting on an ask (at most the warm
        // timeout to be taken up); it ends after it.
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pm_text::answers::{Answers, Query, answer_batch};
    use pm_text::font::{FaceFacts, FaceId, FaceInfo, FontBytes, FontDesc, FontResolver};
    use pm_text::host::FontSource;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    const ROOT: &str = "Proot";

    fn text(id: &str, from: f64, d: Value) -> Value {
        json!({"id": id, "type": "text", "name": id, "from": from, "dur": 1, "d": d})
    }
    fn keyed(v: Value, kf: &[Value]) -> Value {
        let kf: Vec<Value> = kf
            .iter()
            .enumerate()
            .map(|(i, v)| json!({"t": i, "v": v, "ei": [1, 1], "eo": [0, 0], "hold": true}))
            .collect();
        json!({"expr": null, "kf": kf, "v": v})
    }
    fn precomp(id: &str, from: f64, comp: &str) -> Value {
        json!({"id": id, "type": "precomp", "name": id, "from": from, "dur": 4, "d": {"comp": comp}})
    }
    fn comp(id: &str, layers: Value) -> Value {
        json!({"id": id, "name": id, "w": 100, "h": 100, "fps": 30, "dur": 4, "layers": layers})
    }
    fn project(layers: Value, comps: &[(&str, Value)]) -> Value {
        let comps: serde_json::Map<String, Value> = comps
            .iter()
            .map(|(id, c)| ((*id).to_owned(), c.clone()))
            .collect();
        json!({
            "id": ROOT, "name": "p", "w": 640, "h": 360, "fps": 30, "dur": 4,
            "bg": "#000000", "revision": 1, "work": [0, 4], "markers": [],
            "assets": {}, "comps": comps, "layers": layers,
        })
    }
    fn doc(project: &Value) -> Arc<pm_model::Document> {
        let parsed = pm_model::json::parse(&project.to_string()).unwrap();
        Arc::new(pm_model::Document::new(Some(parsed), 1))
    }
    /// `launch.pmv`'s `Word` layer: `Geist Width` on its `wdth` axis, the
    /// text typed in by hold keyframes, its box animated.
    fn word(from: f64) -> Value {
        text(
            "Lword",
            from,
            json!({
                "align": "center", "animators": [], "color": "#000000",
                "boxHeight": keyed(json!(0), &[]), "boxWidth": keyed(json!(0), &[]),
                "font": "Geist Width", "fontAxis.wdth": 89, "italic": false,
                "leading": 1.2208, "size": 147, "tracking": 0, "weight": 420,
                "text": keyed(json!("Powermove"), &[json!(""), json!("T"), json!("Te"), json!("Text")]),
            }),
        )
    }
    fn summary(contents: &[WarmText]) -> Vec<(Option<String>, Option<f64>, String, f64)> {
        let mut out: Vec<_> = contents
            .iter()
            .map(|w| {
                (
                    w.content.font.clone(),
                    w.content.weight,
                    w.content.text.clone(),
                    w.start,
                )
            })
            .collect();
        out.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        out
    }

    #[test]
    fn every_text_content_of_the_tree_once_whatever_its_span() {
        let sf = |id: &str, from: f64, t: &str| {
            text(
                id,
                from,
                json!({"font": "SF Pro", "weight": 400, "size": 20, "text": t}),
            )
        };
        let mut member = sf("Lmember", 0.0, "Same");
        member["group"] = json!("Lgroup");
        let styled = text(
            "Lstyled",
            1.0,
            json!({
                "font": "SF Pro", "weight": 400, "size": 20, "text": "Ab",
                "styles": [{"start": 0, "end": 1, "p": {"font": "Styled", "color": "#f00"}}],
            }),
        );
        let p = project(
            json!([
                sf("Lsame", 0.0, "Same"),
                {"id": "Lgroup", "type": "group", "name": "g", "from": 0, "dur": 4, "d": {}},
                member,
                // Long after the composition's end: still warmed.
                word(100.0),
                styled,
                {"id": "Lshape", "type": "shape", "name": "s", "from": 0, "dur": 4, "d": {"font": "Not text"}},
                precomp("Pnested", 2.0, "Cnested"),
            ]),
            &[
                (
                    "Cnested",
                    comp(
                        "Cnested",
                        json!([
                            text(
                                "Lnested",
                                0.5,
                                json!({"font": "Nested", "weight": keyed(json!(300), &[json!(300), json!(700)]), "size": 30, "text": "N"})
                            ),
                            precomp("Pcycle", 0.0, "Ccycle"),
                            precomp("Proot", 0.0, ROOT),
                        ]),
                    ),
                ),
                (
                    "Ccycle",
                    comp("Ccycle", json!([precomp("Pback", 0.0, "Cnested")])),
                ),
                // Not in the open composition's tree.
                (
                    "Cother",
                    comp("Cother", json!([sf("Lother", 0.0, "Other")])),
                ),
            ],
        );
        let contents = text_contents(&doc(&p), ROOT);
        let s = |f: &str| Some(f.to_owned());
        let mut expected = vec![
            (s("SF Pro"), Some(400.), "Same".to_owned(), 0.),
            (s("SF Pro"), Some(400.), "Ab".to_owned(), 1.),
            (s("Styled"), Some(400.), "Ab".to_owned(), 1.),
            (s("Nested"), Some(300.), "N".to_owned(), 2.5),
            (s("Nested"), Some(700.), "N".to_owned(), 2.5),
        ];
        for t in ["Powermove", "", "T", "Te", "Text"] {
            expected.push((s("Geist Width"), Some(420.), t.to_owned(), 100.));
        }
        expected.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        assert_eq!(summary(&contents), expected);
        // Earliest first; the axis is read as the layout reads it, and keys
        // that choose no face are left out.
        assert!(contents.windows(2).all(|w| w[0].start <= w[1].start));
        let geist = contents
            .iter()
            .find(|w| w.content.font.as_deref() == Some("Geist Width"))
            .unwrap();
        assert_eq!(geist.content.axes, vec![("wdth".to_owned(), 89.0)]);
        assert_eq!(geist.content.size, Some(147.0));
        assert_eq!(geist.content.align, None);
        assert_eq!(geist.content.box_width, None);
        // A composition that does not resolve has nothing.
        assert!(text_contents(&doc(&p), "Cmissing").is_empty());
    }

    #[test]
    fn too_many_keyed_combinations_take_each_property_alone() {
        let values = |n: usize| -> Vec<Value> { (0..n).map(|i| json!(100 + i)).collect() };
        let d = json!({
            "font": "F", "text": "x",
            "weight": keyed(json!(100), &values(10)),
            "size": keyed(json!(100), &values(10)),
        });
        let p = project(json!([text("Lt", 0.0, d)]), &[]);
        let contents = text_contents(&doc(&p), ROOT);
        // 10 × 10 is past the bound: the base and nine more of each.
        assert_eq!(contents.len(), 1 + 9 + 9);
    }

    /// One real face (the `Geist` variable fixture) answering `Geist Width`
    /// and `Geist`, the standard font and the last resort.
    struct OneFace(FaceInfo);

    impl FontResolver for OneFace {
        fn face(&self, _: FaceId) -> &FaceInfo {
            &self.0
        }
        fn family(&mut self, family: &str, _: &FontDesc) -> Option<FaceId> {
            matches!(family, "Geist Width" | "Geist").then_some(0)
        }
        fn standard(&mut self, _: &FontDesc) -> Option<FaceId> {
            Some(0)
        }
        fn fallback_for_char(
            &mut self,
            _: &FontDesc,
            _: u32,
            _: FaceId,
            _: bool,
        ) -> Option<FaceId> {
            None
        }
        fn last_resort(&mut self, _: &FontDesc) -> Option<FaceId> {
            Some(0)
        }
        fn variable_face(&mut self, family: &str) -> Option<FaceId> {
            (family == "Geist Width").then_some(0)
        }
    }

    /// A registry that keeps its answers, as the CoreText one does, and
    /// records each question asked for the first time (a cold answer).
    struct Registry {
        faces: OneFace,
        bytes: FontBytes,
        seen: HashSet<Query>,
        cold: Vec<Query>,
    }

    impl FontSource for Registry {
        fn generation(&self) -> u64 {
            0
        }
        fn answer(&mut self, queries: &[Query]) -> Result<Answers, String> {
            for q in queries {
                if self.seen.insert(q.clone()) {
                    self.cold.push(q.clone());
                }
            }
            Ok(answer_batch(&mut self.faces, queries))
        }
        fn blob(&self, key: &str) -> Option<FontBytes> {
            (key == "geist").then(|| self.bytes.clone())
        }
    }

    fn registry() -> Arc<Mutex<Registry>> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../engine/crates/pm-text/tests/fixtures/Geist-Variable.ttf"
        );
        let bytes = FontBytes::new(std::fs::read(path).expect("the Geist fixture"));
        let facts = FaceFacts::of(bytes.as_slice(), 0);
        let face = FaceInfo {
            data: bytes.clone(),
            index: 0,
            blob: "geist".into(),
            coords: Vec::new(),
            synthetic_bold: false,
            synthetic_italic: false,
            custom: false,
            web: None,
            synthesized_hyphens: [None, None],
            postscript_name: "Geist-Regular".into(),
            typeface_key: "geist".into(),
            fallback_key: None,
            facts,
        };
        Arc::new(Mutex::new(Registry {
            faces: OneFace(face),
            bytes,
            seen: HashSet::new(),
            cold: Vec::new(),
        }))
    }

    /// The main thread's pump: answers the broker's asks from `registry`.
    fn serve(
        broker: &FontBroker,
        registry: &Arc<Mutex<Registry>>,
    ) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
        broker.set_wake(Arc::new(|| {}));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let (broker, registry, stop) = (broker.clone(), registry.clone(), stop.clone());
            move || {
                while !stop.load(Ordering::Acquire) {
                    let asks = broker.take_asks();
                    if !asks.is_empty() {
                        pm_text::host::serve(&mut *lock(&registry), asks);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
        (stop, thread)
    }

    pub(crate) fn with_test_fonts<T>(
        test: impl FnOnce(pm_render::text::host::RenderFonts) -> T,
    ) -> T {
        struct Pump(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);
        impl Drop for Pump {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
                self.1.take().unwrap().join().unwrap();
            }
        }

        use pm_render::text::host::RenderFonts;
        let broker = FontBroker::new();
        let (stop, thread) = serve(&broker, &registry());
        let _pump = Pump(stop, Some(thread));
        test(RenderFonts::new(broker))
    }

    fn wait_for(warmer: &FontWarmer, completed: u64) {
        let started = Instant::now();
        while warmer.completed() < completed {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "the warm job never finished"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The `Word` layer as the frame's layout reads its resolved content at
    /// 14 s: every key, not only those that choose faces.
    fn resolved_word() -> TextContent {
        TextContent::from_json(&json!({
            "align": "center", "animators": [], "color": "#000000",
            "boxHeight": 0, "boxWidth": 0, "font": "Geist Width", "fontAxis.wdth": 89,
            "italic": false, "leading": 1.2208, "size": 147, "tracking": 0,
            "weight": 420, "text": "Powermove",
        }))
    }

    #[test]
    fn a_warmed_composition_leaves_its_frames_only_cached_answers() {
        let registry = registry();
        let broker = FontBroker::new();
        let (stop, pump) = serve(&broker, &registry);
        let fonts = pm_render::text::host::RenderFonts::new(broker.clone());
        let mut warmer = FontWarmer::default();
        // The word starts far after the frame drawn first.
        let first = doc(&project(json!([word(13.733)]), &[]));
        assert!(warmer.request(&first, ROOT, &fonts, first.edition()));
        wait_for(&warmer, 1);
        {
            let registry = lock(&registry);
            let desc = pm_text::compat::canvas::canvas_font(&resolved_word(), None).desc;
            for q in [
                Query::variable("Geist Width"),
                Query::family("Geist Width", &desc),
                Query::standard(&desc),
            ] {
                assert!(registry.cold.contains(&q), "{q:?} was not asked ahead");
            }
        }
        let cold = lock(&registry).cold.len();
        let (asked, _, failed) = broker.counts();
        assert_eq!(failed, 0);

        // The render thread's own fonts (`EngineFonts`: a fresh replay over
        // the same broker) lay the word out as a frame does: each of their
        // questions was asked ahead, so the registry answers from its cache.
        let mut frame =
            CompatMeasure::new(BrokerResolver::new(broker.clone(), FontHealth::default()));
        let d = resolved_word();
        let lines = frame.compat().line_runs(&d);
        assert!(
            !lines.is_empty() && !lines[0].runs.is_empty(),
            "the word was laid out"
        );
        for line in &lines {
            frame
                .compat()
                .text_runs(&d, &line.text, d.tracking.unwrap_or(0.0));
        }
        assert!(
            broker.counts().0 > asked,
            "the frame's replay asked the registry"
        );
        assert_eq!(
            lock(&registry).cold.len(),
            cold,
            "no question of the frame was cold"
        );

        // The same scene again asks nothing.
        assert!(!warmer.request(&first, ROOT, &fonts, first.edition()));
        stop.store(true, Ordering::Release);
        pump.join().unwrap();
    }

    #[test]
    fn a_new_document_or_font_generation_warms_again_and_only_what_is_new() {
        let registry = registry();
        let broker = FontBroker::new();
        let (stop, pump) = serve(&broker, &registry);
        let fonts = pm_render::text::host::RenderFonts::new(broker.clone());
        let mut warmer = FontWarmer::default();
        let first = doc(&project(json!([word(0.0)]), &[]));
        assert!(warmer.request(&first, ROOT, &fonts, first.edition()));
        wait_for(&warmer, 1);
        assert!(
            !warmer.request(&first, ROOT, &fonts, first.edition()),
            "the same scene is not warmed twice"
        );

        // An edit adds a bold Geist layer: only its questions are new.
        let bold = text(
            "Lbold",
            2.0,
            json!({"font": "Geist", "weight": 700, "size": 40, "text": "Bold"}),
        );
        let second = doc(&project(json!([word(0.0), bold]), &[]));
        assert!(warmer.request(&second, ROOT, &fonts, second.edition()));
        wait_for(&warmer, 2);
        let desc = pm_text::compat::canvas::canvas_font(
            &TextContent::from_json(&json!({"font": "Geist", "weight": 700, "size": 40})),
            None,
        )
        .desc;
        assert!(
            lock(&registry)
                .cold
                .contains(&Query::family("Geist", &desc))
        );

        // Another edition of the same content is handed over, and asks
        // nothing: every content was laid out already.
        let asked = broker.counts().0;
        let third = Arc::new((*second).clone());
        assert!(warmer.request(&third, ROOT, &fonts, third.edition()));
        wait_for(&warmer, 3);
        assert_eq!(broker.counts().0, asked);

        // Fonts changed (a font installed): the replay is made again, so
        // it asks again, and the registry's answers stand.
        let cold = lock(&registry).cold.len();
        broker.set_generation(broker.generation() + 1);
        assert!(warmer.request(&third, ROOT, &fonts, third.edition()));
        wait_for(&warmer, 4);
        assert!(broker.counts().0 > asked);
        assert_eq!(lock(&registry).cold.len(), cold);

        // Replaced fonts (`Engine::set_fonts`) warm again as well.
        warmer.reset();
        assert!(warmer.request(&third, ROOT, &fonts, third.edition()));
        wait_for(&warmer, 5);
        assert_eq!(broker.counts().2, 0, "no ask failed");
        stop.store(true, Ordering::Release);
        pump.join().unwrap();
    }

    #[test]
    fn a_service_that_never_answers_neither_blocks_nor_keeps_the_document() {
        let broker = FontBroker::new();
        // Asks are queued and never taken up.
        broker.set_wake(Arc::new(|| {}));
        let fonts = pm_render::text::host::RenderFonts::new(broker.clone());
        let mut warmer = FontWarmer::with_timeout(Duration::from_millis(100));
        let first = doc(&project(json!([word(0.0)]), &[]));
        assert!(warmer.request(&first, ROOT, &fonts, first.edition()));
        wait_for(&warmer, 1);
        assert_eq!(
            broker.counts().2,
            1,
            "the ask failed once, then the job gave up"
        );
        assert_eq!(Arc::strong_count(&first), 1, "the warmer keeps no document");
        // A scene waiting behind a stuck ask is let go with the warmer.
        let second = doc(&project(json!([word(1.0)]), &[]));
        assert!(warmer.request(&second, ROOT, &fonts, second.edition()));
        let dropped = Instant::now();
        drop(warmer);
        assert!(
            dropped.elapsed() < Duration::from_millis(50),
            "dropping never waits"
        );
        let started = Instant::now();
        while Arc::strong_count(&second) > 1 {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the document was kept"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
