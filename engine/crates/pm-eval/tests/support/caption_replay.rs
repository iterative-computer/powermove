//! The replay of `tests/vectors/caption-vectors.json.gz` (written by
//! `tests/vectors/generate-caption-vectors.ts`), shared by
//! `tests/caption_parity.rs` and the identity examples. Include it next to
//! `layer_replay.rs` (it uses that module's value encoding).
#![allow(dead_code)]

use std::collections::BTreeMap;

use pm_eval::captions::{self, Block, CaptionLayout, CaptionStyle, CaptionWord, CueRef, WordState};
use pm_eval::text_geom::{MeasuredLine, TextMeasure};
use pm_eval::{JsThrow, JsValue};
use pm_model::json::Value;
use pm_model::{JsString, Node};
use pm_text::compat::{GlyphLayout, TextContent};

use super::layer_replay::{V, dec, from_js, get, hexf};

/// The generator's `synth` canvas: `measureText(text).width` from the font
/// string's style, weight, size, and first family, plus letter spacing per
/// code point.
pub struct Synth;

impl TextMeasure for Synth {
    fn measure_lines(&mut self, _: &TextContent) -> Vec<MeasuredLine> {
        unreachable!("captions measure words")
    }
    fn plain_width(&mut self, _: &TextContent, _: &str) -> f64 {
        unreachable!("captions measure words")
    }
    fn glyph_layout(&mut self, _: &TextContent) -> GlyphLayout {
        unreachable!("captions measure words")
    }
    fn spaced_width(&mut self, d: &TextContent, text: &str) -> f64 {
        let family = d
            .font
            .clone()
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| "SF Pro Display".to_owned());
        let weight = d
            .weight
            .filter(|w| *w != 0.0 && !w.is_nan())
            .unwrap_or(500.0);
        let size = d.size.unwrap();
        let ls = d.tracking.filter(|t| !t.is_nan()).unwrap_or(0.0);
        let italic = if d.italic.unwrap_or(false) { 1.05 } else { 1.0 };
        let k = size / 16.0
            * italic
            * (1.0 + (weight - 400.0) / 4000.0)
            * (1.0 + family.encode_utf16().count() as f64 / 100.0);
        let mut w = 0.0;
        let mut n = 0.0;
        for c in text.chars() {
            w += f64::from((u32::from(c) % 13) + 4) * k;
            n += 1.0;
        }
        w + ls * n
    }
}

pub fn node(case: &Value, key: &str) -> Node {
    Node::from_value(get(case, key).cloned().unwrap_or(Value::Null), 1)
}

pub fn num(case: &Value, key: &str) -> f64 {
    match get(case, key) {
        Some(Value::Number(n)) => *n,
        other => panic!("{key}: {other:?}"),
    }
}

pub fn s(x: &JsString) -> V {
    V::S(x.to_string_lossy())
}

pub fn obj<const N: usize>(entries: [(&str, V); N]) -> V {
    V::O(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
    )
}

pub fn style_value(st: &CaptionStyle) -> V {
    obj([
        ("preset", s(&st.preset)),
        ("font", s(&st.font)),
        ("weight", V::N(st.weight)),
        ("italic", V::B(st.italic)),
        ("size", V::N(st.size)),
        ("tracking", V::N(st.tracking)),
        ("leading", V::N(st.leading)),
        ("fill", s(&st.fill)),
        ("stroke", V::N(st.stroke)),
        ("strokeColor", s(&st.stroke_color)),
        ("box", V::B(st.boxed)),
        ("boxColor", s(&st.box_color)),
        ("boxOpacity", V::N(st.box_opacity)),
        ("boxPadding", V::N(st.box_padding)),
        ("boxRadius", V::N(st.box_radius)),
        ("placement", V::S(st.placement.name().into())),
        ("align", V::S(st.align.name().into())),
        ("offsetX", V::N(st.offset_x)),
        ("offsetY", V::N(st.offset_y)),
        ("safeMargin", V::N(st.safe_margin)),
        ("maxWidth", V::N(st.max_width)),
        ("maxLines", V::N(st.max_lines)),
        ("textCase", V::S(st.text_case.name().into())),
        ("highlight", V::B(st.highlight)),
        ("highlightColor", s(&st.highlight_color)),
        ("highlightScale", V::N(st.highlight_scale)),
        (
            "wordAnimation",
            V::S(st.word_animation.name().unwrap_or("?").into()),
        ),
    ])
}

pub fn words_value(words: &[CaptionWord]) -> V {
    V::A(
        words
            .iter()
            .map(|w| {
                let mut o = BTreeMap::from([("text".to_owned(), s(&w.text))]);
                for (k, v) in [("start", &w.start), ("end", &w.end)] {
                    if !matches!(v, JsValue::Undefined) {
                        o.insert(k.to_owned(), from_js(v));
                    }
                }
                V::O(o)
            })
            .collect(),
    )
}

pub fn layout_value(l: &CaptionLayout) -> V {
    obj([
        ("size", V::N(l.size)),
        ("lineHeight", V::N(l.line_height)),
        (
            "lines",
            V::A(
                l.lines
                    .iter()
                    .map(|line| {
                        obj([
                            (
                                "words",
                                V::A(
                                    line.words
                                        .iter()
                                        .map(|w| {
                                            obj([
                                                ("index", V::N(w.index as f64)),
                                                ("text", s(&w.text)),
                                                ("x", V::N(w.x)),
                                                ("width", V::N(w.width)),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                            ("width", V::N(line.width)),
                            ("y", V::N(line.y)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("width", V::N(l.width)),
        ("height", V::N(l.height)),
        ("words", words_value(&l.words)),
    ])
}

pub fn block_value(b: &Block) -> V {
    obj([
        ("x0", V::N(b.x0)),
        ("y0", V::N(b.y0)),
        ("x1", V::N(b.x1)),
        ("y1", V::N(b.y1)),
    ])
}

pub fn states_value(states: &[WordState]) -> V {
    V::A(
        states
            .iter()
            .map(|st| {
                obj([
                    ("alpha", V::N(st.alpha)),
                    ("scale", V::N(st.scale)),
                    ("highlighted", V::B(st.highlighted)),
                ])
            })
            .collect(),
    )
}

pub fn result<T>(r: Result<T, JsThrow>, f: impl FnOnce(T) -> V) -> V {
    match r {
        Ok(v) => f(v),
        Err(e) => V::Throw(format!("{}: {}", e.kind, e.message)),
    }
}

/// Runs every case, calling `check(op, case, ours, want)`.
pub fn replay(vectors: &Value, check: &mut dyn FnMut(&str, &Value, V, &V)) {
    for case in get(vectors, "cases").unwrap().as_array().unwrap() {
        let op = get(case, "op").unwrap().as_str().unwrap().to_owned();
        let want = dec(get(case, "out").unwrap());
        let ours = match op.as_str() {
            "style" => style_value(&captions::normalize_caption_style(Some(&node(case, "raw")))),
            "tokens" => {
                let text = node(case, "text");
                let text = pm_eval::node::string(Some(&text)).unwrap().clone();
                V::A(
                    captions::text_tokens(&text)
                        .iter()
                        .map(|t| obj([("text", s(&t.text)), ("gap", V::B(t.gap))]))
                        .collect(),
                )
            }
            "words" => {
                let cue = node(case, "cue");
                let text = pm_eval::node::string(pm_eval::node::member(Some(&cue), "text"))
                    .unwrap()
                    .clone();
                result(captions::cue_words(CueRef { node: &cue }, &text), |w| {
                    words_value(&w)
                })
            }
            "layout" => {
                let cue = node(case, "cue");
                let style = captions::normalize_caption_style(Some(&node(case, "style")));
                let apply = !matches!(get(case, "applyCase"), Some(Value::Bool(false)));
                let mut synth = Synth;
                let mut measure = |text: &JsString, size: f64| {
                    synth.spaced_width(
                        &captions::measure_content(&style, size),
                        &text.to_string_lossy(),
                    )
                };
                result(
                    captions::layout_caption(
                        CueRef { node: &cue },
                        &style,
                        &mut measure,
                        num(case, "width"),
                        apply,
                    ),
                    |l| layout_value(&l),
                )
            }
            "place" => {
                let style = captions::normalize_caption_style(Some(&node(case, "style")));
                let layout = get(case, "layout").unwrap();
                block_value(&captions::place_block(
                    num(layout, "width"),
                    num(layout, "height"),
                    &style,
                    num(case, "w"),
                    num(case, "h"),
                ))
            }
            "states" => {
                let style = captions::normalize_caption_style(Some(&node(case, "style")));
                let list = node(case, "words");
                let words: Vec<CaptionWord> = pm_eval::node::array(Some(&list))
                    .unwrap()
                    .iter()
                    .map(|w| CaptionWord {
                        text: pm_eval::node::to_js_string(pm_eval::node::member(Some(w), "text")),
                        start: pm_eval::node::js_value_opt(pm_eval::node::member(Some(w), "start")),
                        end: pm_eval::node::js_value_opt(pm_eval::node::member(Some(w), "end")),
                    })
                    .collect();
                states_value(&captions::word_states(
                    &words,
                    &style,
                    hexf(get(case, "t").unwrap()),
                ))
            }
            "index" => {
                let list = node(case, "cues");
                let cues = pm_eval::node::array(Some(&list)).unwrap();
                result(
                    captions::cue_index_at(cues, hexf(get(case, "t").unwrap())),
                    |i| V::N(i.map_or(-1.0, |i| i as f64)),
                )
            }
            "frame" => {
                let content = node(case, "content");
                let (w, h) = (num(case, "w"), num(case, "h"));
                let local = hexf(get(case, "local").unwrap());
                result(
                    captions::caption_frame(Some(&content), local, w, h, &mut Synth),
                    |f| match f {
                        Some(f) => obj([(
                            "frame",
                            obj([
                                ("cue", V::N(f.cue as f64)),
                                ("layout", layout_value(&f.layout)),
                                ("states", states_value(&f.states)),
                                ("block", block_value(&f.block)),
                                ("margin", V::N(f.margin)),
                            ]),
                        )]),
                        None => {
                            let b = captions::idle_block(Some(&content), w, h);
                            obj([
                                ("frame", V::Z),
                                (
                                    "idle",
                                    obj([
                                        ("x0", V::N(b.x0)),
                                        ("y0", V::N(b.y0)),
                                        ("x1", V::N(b.x1)),
                                        ("y1", V::N(b.y1)),
                                        ("w", V::N(b.x1 - b.x0)),
                                        ("h", V::N(b.y1 - b.y0)),
                                        ("ax", V::N(0.0)),
                                        ("ay", V::N(0.0)),
                                    ]),
                                ),
                            ])
                        }
                    },
                )
            }
            other => panic!("unknown op {other}"),
        };
        check(&op, case, ours, &want);
    }
}
