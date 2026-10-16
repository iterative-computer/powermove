//! The same comparison through pm-wasm: every item becomes the pm-wasm
//! queries the renderer would send, answered by the built WebAssembly module
//! in Node (V8) and by the same engine compiled natively. The two must reply
//! byte for byte the same (the Phase 1 gate's "native and WASM
//! bit-identical"); the module's replies are judged against the corpus.
//!
//! Per project:
//!
//! 1. **Discovery** (native `pm_wasm::Engine`): requests are built and asked
//!    one by one; a `{"fonts"}` reply is answered as main answers the
//!    renderer (registry answers for the misses, face bytes for pending
//!    faces) and asked again, until the mirror has every font it needs.
//! 2. **Replay**: a fresh native engine and a fresh module instance in Node,
//!    each given the same answers and faces up front, answer every request.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use pm_model::JsString;
use pm_model::json::{Map, Value, parse, to_canonical_string};
use pm_text::answers::{Answers, Query};
use pm_wasm::Engine;

use super::compare::{Answer, revive};
use super::fonts::FontService;
use super::items::{Item, Kind};
use super::native::HOST;

pub const FONTS_MISSING: &str = "fonts the mirror lacks after discovery";
pub const INK: &str =
    "glyph ink with metrics-and-shaping subsets (the renderer keeps the legacy answer)";

/// How an item's answer is assembled from replies (indexes into the
/// project's request list).
#[derive(Clone, Debug)]
pub enum Plan {
    Doc,
    One(usize),
    /// Layer fields: the raw members plus one `evP` reply per field.
    Fields(Map, Vec<(&'static str, usize)>),
    /// Paths: per path, its id and the `pathValues` and `groupMatrix` replies.
    Paths(Vec<(Value, usize, usize)>),
}

/// A project's requests and plans.
pub struct Project {
    pub doc: String,
    pub requests: Vec<String>,
    pub plans: Vec<Vec<Plan>>,
    pub answers: Answers,
    pub faces: BTreeSet<(String, u32)>,
}

fn obj(items: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in items {
        m.insert(k, v);
    }
    Value::Object(m)
}

fn str_value(s: &str) -> Value {
    Value::String(JsString::from(s))
}

/// Discovery for one project.
pub struct Discovery<'f> {
    fonts: &'f mut FontService,
    engine: Engine,
    project: Project,
    /// `allProps` paths per (composition, layer index).
    props: HashMap<(String, usize), Vec<(JsString, Value)>>,
}

impl<'f> Discovery<'f> {
    pub fn new(fonts: &'f mut FontService, canonical: &Value) -> Result<Discovery<'f>, String> {
        let doc = to_canonical_string(canonical);
        let mut engine = Engine::new();
        engine.resync(doc.as_bytes(), 1)?;
        engine.set_font_outlines(!fonts.subsets);
        Ok(Discovery {
            fonts,
            engine,
            project: Project {
                doc,
                requests: Vec::new(),
                plans: Vec::new(),
                answers: Answers::default(),
                faces: BTreeSet::new(),
            },
            props: HashMap::new(),
        })
    }

    fn add_answers(&mut self, answers: &Answers) -> Result<(), String> {
        let json = serde_json::to_vec(answers).map_err(|e| e.to_string())?;
        self.engine.add_font_answers(&json)?;
        self.project.answers.merge(answers);
        Ok(())
    }

    fn add_face(&mut self, blob: &str, index: u32) -> Result<(), String> {
        if self.project.faces.contains(&(blob.to_owned(), index)) {
            return Ok(());
        }
        let bytes = self.fonts.face(blob, index)?;
        self.engine.add_font(blob, index, bytes.as_ref().clone());
        self.project.faces.insert((blob.to_owned(), index));
        Ok(())
    }

    /// Asks a request, serving fonts until the reply no longer asks for them.
    fn ask(&mut self, request: Value) -> Result<(usize, String), String> {
        let text = to_canonical_string(&request);
        let mut reply = self.engine.query(text.as_bytes()).text;
        for _ in 0..16 {
            if reply != "{\"fonts\":true}" {
                break;
            }
            let report: serde_json::Value =
                serde_json::from_str(&self.engine.font_report()).map_err(|e| e.to_string())?;
            let misses: Vec<Query> =
                serde_json::from_value(report["misses"].clone()).map_err(|e| e.to_string())?;
            let mut progress = false;
            if !misses.is_empty() {
                let answers = self.fonts.answer(&misses)?;
                self.add_answers(&answers)?;
                progress = true;
            }
            if let Some(pending) = report["pending"].as_array() {
                for p in pending {
                    let (Some(blob), Some(index)) = (p[0].as_str(), p[1].as_u64()) else {
                        continue;
                    };
                    if !self
                        .project
                        .faces
                        .contains(&(blob.to_owned(), index as u32))
                    {
                        self.add_face(blob, index as u32)?;
                        progress = true;
                    }
                }
            }
            if !progress {
                break;
            }
            reply = self.engine.query(text.as_bytes()).text;
        }
        self.project.requests.push(text);
        Ok((self.project.requests.len() - 1, reply))
    }

    fn base(it: &Item, op: &str) -> Vec<(&'static str, Value)> {
        let mut v: Vec<(&'static str, Value)> = vec![
            ("comp", str_value(&it.comp)),
            ("open", str_value(&it.open)),
            ("t", Value::Number(it.time)),
        ];
        if let Some(i) = it.layer {
            v.push(("layer", Value::Number(i as f64)));
        }
        v.push(("op", str_value(op)));
        v
    }

    fn one(
        &mut self,
        it: &Item,
        op: &str,
        extra: Vec<(&'static str, Value)>,
    ) -> Result<Plan, String> {
        let mut fields = Self::base(it, op);
        fields.extend(extra);
        let (i, _) = self.ask(obj(fields))?;
        Ok(Plan::One(i))
    }

    /// The `allProps` paths of a layer (asked once per layer and dump).
    fn paths_of(&mut self, it: &Item) -> Result<Vec<(JsString, Value)>, String> {
        let key = (it.path.clone(), it.layer.unwrap_or(0));
        if let Some(p) = self.props.get(&key) {
            return Ok(p.clone());
        }
        let (_, reply) = self.ask(obj(Self::base(it, "allProps")))?;
        let parsed = parse(&reply).map_err(|e| format!("allProps reply: {e}"))?;
        let entries = parsed
            .get("ok")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|e| {
                        let key = e.get("key").map(Value::to_js_string).unwrap_or_default();
                        let path = match e.get("path") {
                            Some(p @ Value::Array(_)) => p.clone(),
                            _ => Value::Null,
                        };
                        (key, path)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.props.insert(key, entries.clone());
        Ok(entries)
    }

    /// The requests of one dump's items.
    pub fn dump(&mut self, items: &[Item]) -> Result<(), String> {
        self.props.clear();
        let mut plans = Vec::with_capacity(items.len());
        for it in items {
            let plan = match &it.kind {
                Kind::Doc(_) => Plan::Doc,
                Kind::Active => self.one(it, "active", vec![])?,
                Kind::Channel { key, in_p } => {
                    if *in_p {
                        self.one(it, "ev", vec![("key", Value::String(key.clone()))])?
                    } else {
                        let paths = self.paths_of(it)?;
                        let path = paths
                            .iter()
                            .find(|(k, _)| k == key)
                            .map_or(Value::Null, |(_, p)| p.clone());
                        self.one(
                            it,
                            "evP",
                            vec![("key", Value::String(key.clone())), ("path", path)],
                        )?
                    }
                }
                Kind::ExprError { key } => {
                    let fields = vec![
                        ("op", str_value("expressionError")),
                        ("comp", str_value(&it.comp)),
                        ("open", str_value(&it.open)),
                        ("layerId", str_value(&it.layer_id)),
                        ("key", Value::String(key.clone())),
                    ];
                    let (i, _) = self.ask(obj(fields))?;
                    Plan::One(i)
                }
                Kind::LayerFields { raw, evaluated } => {
                    let mut asked = Vec::new();
                    for (field, key) in evaluated {
                        let mut fields = Self::base(it, "evP");
                        fields.push(("key", str_value(key)));
                        fields.push(("path", Value::Array(vec![str_value(field)])));
                        let (i, _) = self.ask(obj(fields))?;
                        asked.push((*field, i));
                    }
                    Plan::Fields(raw.clone(), asked)
                }
                Kind::Content => self.one(it, "resolveContent", vec![])?,
                Kind::LocalMatrix => self.one(it, "localMatrix", vec![])?,
                Kind::WorldMatrix => self.one(it, "worldMatrix", vec![])?,
                Kind::WorldOpacity => self.one(it, "worldOpacity", vec![])?,
                Kind::PlaneMatrix => self.one(it, "planeMatrix", vec![])?,
                Kind::Bounds => self.one(it, "bounds", vec![])?,
                Kind::TextLayout => self.one(it, "glyphLayout", vec![])?,
                Kind::TextRaster => self.one(it, "textRasterGeometry", vec![])?,
                Kind::TextAnimation => self.one(it, "textAnimation", vec![])?,
                Kind::CaptionBounds => self.one(it, "captionBounds", vec![])?,
                Kind::Paths { ids } => {
                    let mut asked = Vec::new();
                    for (i, id) in ids.iter().enumerate() {
                        let (values, _) = {
                            let mut f = Self::base(it, "pathValues");
                            f.push(("index", Value::Number(i as f64)));
                            self.ask(obj(f))?
                        };
                        let (matrix, _) = {
                            let mut f = Self::base(it, "groupMatrix");
                            f.push(("index", Value::Number(i as f64)));
                            self.ask(obj(f))?
                        };
                        asked.push((id.clone(), values, matrix));
                    }
                    Plan::Paths(asked)
                }
                Kind::SourceTime { legacy } => self.one(
                    it,
                    "sourceTime",
                    vec![(
                        "retiming",
                        str_value(if *legacy { "legacy" } else { "engine" }),
                    )],
                )?,
                Kind::Scene => {
                    let fields = vec![
                        ("op", str_value("compositionScene")),
                        ("comp", str_value(&it.open)),
                        ("open", str_value(&it.open)),
                        ("t", Value::Number(it.time)),
                    ];
                    let (i, _) = self.ask(obj(fields))?;
                    Plan::One(i)
                }
            };
            plans.push(plan);
        }
        self.project.plans.push(plans);
        Ok(())
    }

    pub fn finish(self) -> Project {
        self.project
    }
}

/// A fresh engine with the project's fonts, replying to every request.
pub fn native_replies(project: &Project, fonts: &mut FontService) -> Result<Vec<String>, String> {
    let mut engine = Engine::new();
    engine.set_font_outlines(!fonts.subsets);
    engine.add_font_answers(&serde_json::to_vec(&project.answers).map_err(|e| e.to_string())?)?;
    for (blob, index) in &project.faces {
        engine.add_font(blob, *index, fonts.face(blob, *index)?.as_ref().clone());
    }
    engine.resync(project.doc.as_bytes(), 1)?;
    Ok(project
        .requests
        .iter()
        .map(|r| engine.query(r.as_bytes()).text)
        .collect())
}

/// The Node side: `scripts/parity-wasm.mjs` running the built module.
pub struct NodeRunner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    dir: PathBuf,
    written: BTreeMap<(String, u32), PathBuf>,
}

impl NodeRunner {
    pub fn start(
        node: &str,
        script: &Path,
        module: &Path,
        dir: PathBuf,
    ) -> Result<NodeRunner, String> {
        std::fs::create_dir_all(dir.join("faces")).map_err(|e| e.to_string())?;
        let mut child = Command::new(node)
            .arg(script)
            .arg(module)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("starting {node}: {e}"))?;
        let stdin = child.stdin.take().ok_or("node stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("node stdout")?);
        let mut runner = NodeRunner {
            child,
            stdin,
            stdout,
            dir,
            written: BTreeMap::new(),
        };
        let ready = runner.line()?;
        if !ready.starts_with("ready") {
            return Err(format!("parity-wasm.mjs: {ready}"));
        }
        Ok(runner)
    }

    fn line(&mut self) -> Result<String, String> {
        let mut s = String::new();
        let n = self.stdout.read_line(&mut s).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("parity-wasm.mjs exited".into());
        }
        Ok(s.trim_end().to_owned())
    }

    /// The face as `pm_fonts_add` takes it: key length, key, face index, bytes.
    fn face_file(
        &mut self,
        fonts: &mut FontService,
        blob: &str,
        index: u32,
    ) -> Result<PathBuf, String> {
        let key = (blob.to_owned(), index);
        if let Some(p) = self.written.get(&key) {
            return Ok(p.clone());
        }
        let bytes = fonts.face(blob, index)?;
        let mut payload = Vec::with_capacity(bytes.len() + blob.len() + 8);
        payload.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        payload.extend_from_slice(blob.as_bytes());
        payload.extend_from_slice(&index.to_le_bytes());
        payload.extend_from_slice(&bytes);
        let path = self
            .dir
            .join("faces")
            .join(format!("{}-{index}.bin", self.written.len()));
        std::fs::write(&path, payload).map_err(|e| e.to_string())?;
        self.written.insert(key, path.clone());
        Ok(path)
    }

    pub fn replies(
        &mut self,
        project: &Project,
        fonts: &mut FontService,
    ) -> Result<Vec<String>, String> {
        let doc = self.dir.join("doc.json");
        let answers = self.dir.join("answers.json");
        let requests = self.dir.join("requests.jsonl");
        let out = self.dir.join("replies.jsonl");
        std::fs::write(&doc, &project.doc).map_err(|e| e.to_string())?;
        std::fs::write(
            &answers,
            serde_json::to_vec(&project.answers).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut text = String::new();
        for r in &project.requests {
            text.push_str(r);
            text.push('\n');
        }
        std::fs::write(&requests, text).map_err(|e| e.to_string())?;
        let mut faces = Vec::new();
        for (blob, index) in &project.faces {
            faces.push(serde_json::Value::String(
                self.face_file(fonts, blob, *index)?
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        let command = serde_json::json!({
            "doc": doc, "answers": answers, "faces": faces, "outlines": !fonts.subsets,
            "requests": requests, "out": out,
        });
        writeln!(self.stdin, "{command}").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        let done = self.line()?;
        if !done.starts_with("done") {
            return Err(format!("parity-wasm.mjs: {done}"));
        }
        let replies = std::fs::read_to_string(&out).map_err(|e| e.to_string())?;
        Ok(replies.lines().map(str::to_owned).collect())
    }

    pub fn close(mut self) {
        drop(self.stdin);
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A reply as an answer.
pub fn reply_answer(reply: &str) -> Answer {
    let Ok(v) = parse(reply) else {
        return Answer::Threw(format!("unparsable reply {reply}"));
    };
    if let Some(ok) = v.get("ok") {
        return Answer::Value(revive(ok.clone()));
    }
    if let Some(t) = v.get("throw") {
        return Answer::Threw(
            t.get("message")
                .map(|m| m.to_js_string().to_string_lossy())
                .unwrap_or_default(),
        );
    }
    if v.get("host").is_some() {
        return Answer::Unanswered(HOST);
    }
    if v.get("fonts").is_some() {
        return Answer::Unanswered(FONTS_MISSING);
    }
    if v.get("ink").is_some() {
        return Answer::Unanswered(INK);
    }
    Answer::Threw(format!("malformed request: {reply}"))
}

/// Answers of one dump's items from the project's replies.
pub fn answers(items: &[Item], plans: &[Plan], replies: &[String]) -> Vec<Answer> {
    items
        .iter()
        .zip(plans)
        .map(|(it, plan)| match plan {
            Plan::Doc => match &it.kind {
                Kind::Doc(v) => Answer::Value(v.clone()),
                _ => Answer::Threw("plan".into()),
            },
            Plan::One(i) => reply_answer(&replies[*i]),
            Plan::Fields(raw, asked) => {
                let mut m = raw.clone();
                for (field, i) in asked {
                    match reply_answer(&replies[*i]) {
                        Answer::Value(v) => {
                            m.insert(*field, v);
                        }
                        other => return other,
                    }
                }
                Answer::Value(Value::Object(m))
            }
            Plan::Paths(asked) => {
                let mut out = Vec::new();
                for (id, values, matrix) in asked {
                    let values = match reply_answer(&replies[*values]) {
                        Answer::Value(v) => v,
                        other => return other,
                    };
                    let matrix = match reply_answer(&replies[*matrix]) {
                        Answer::Value(v) => v,
                        other => return other,
                    };
                    out.push(obj(vec![
                        ("id", id.clone()),
                        ("values", values),
                        ("matrix", matrix),
                    ]));
                }
                Answer::Value(Value::Array(out))
            }
        })
        .collect()
}
