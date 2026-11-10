//! Window ▸ Panels ▸ Shader (S096, Shader Lab) over pm-services
//! `surfaces::shader`: the source of the selected shader layer (else the
//! first in the composition) in a native code editor. Typing writes the
//! layer live, 420 ms after a pause, as one `Edit Shader` gesture per focus,
//! so the viewer shows each change and Undo steps by gesture. Compile
//! (⌘Return) checks the source with the engine's own GLSL pipeline and shows
//! the uniform count or the first error; Presets replace the source as one
//! `Shader Preset` step. The renderer keeps the last good program meanwhile.
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_kernel::{Call, Caller, GestureCall, Reply, Request, WriteBody};
use pm_services::surfaces::{
    DocumentStamp,
    shader::{CompileInput, Compiled, Compiler, Diagnostic, Editor, Layer, selected_shader},
};
use pm_shell::MenuEntry;
use pm_shell::text::{TextEvent, TextField, TextKind};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(100);

/// The engine's verdict on shader-layer source: the GLSL the compositor
/// draws (`PM.SHADER_HEADER` + code), its diagnostics in the author's lines,
/// and the `@param` uniforms the inspector binds.
pub struct EngineCompiler;
impl Compiler for EngineCompiler {
    fn compile(&mut self, input: &CompileInput) -> pm_services::Result<Compiled> {
        use pm_render::shader_layers::{
            parse_uniforms, shader_layer_program,
            translate::{Verdict, verdict},
        };
        let (source, offset) = shader_layer_program(&input.code);
        let diagnostics = match verdict(&source, offset).1 {
            Verdict::Ready(_) => vec![],
            Verdict::Refused(failure) | Verdict::NeedsFallback(failure) => {
                let mut list: Vec<Diagnostic> = failure
                    .diagnostics
                    .iter()
                    .map(|d| Diagnostic {
                        message: d.message.clone(),
                        line: d.line.filter(|l| *l > 0),
                        column: None,
                    })
                    .collect();
                if list.is_empty() {
                    list.push(Diagnostic {
                        message: failure.message.clone(),
                        line: None,
                        column: None,
                    });
                }
                list
            }
        };
        let uniforms: Vec<Value> = parse_uniforms(&input.code)
            .iter()
            .map(|u| json!({"name":u.name,"type":u.ty.name()}))
            .collect();
        Ok(Compiled {
            diagnostics,
            uniform_definitions: json!(uniforms),
        })
    }
}
/// The status bar's line for a compile result.
pub fn status_line(result: &Compiled) -> (bool, String) {
    match result.diagnostics.first() {
        None => {
            let n = result.uniform_definitions.as_array().map_or(0, Vec::len);
            let unit = if n == 1 { "uniform" } else { "uniforms" };
            (true, format!("Compiled · {n} {unit}"))
        }
        Some(d) => {
            let first = d
                .message
                .lines()
                .next()
                .unwrap_or("The shader does not compile.");
            (
                false,
                match d.line {
                    Some(line) => format!("Line {line}: {first}"),
                    None => first.to_owned(),
                },
            )
        }
    }
}

struct Open {
    layer: String,
    editor: Editor,
    field: Entity<TextField>,
    _events: Subscription,
    /// The kernel gesture this focus writes through.
    gesture: Option<u64>,
}
#[derive(Default)]
pub struct State {
    open: Option<Open>,
    status: Option<(bool, String)>,
    compiling: Option<Instant>,
    painted: Option<Instant>,
    running: bool,
    epoch: Option<Instant>,
    /// The open composition's shader layers at (document, revision).
    layers: Option<((u64, u64), Vec<Layer>)>,
}
impl State {
    fn now(&mut self) -> u64 {
        self.epoch
            .get_or_insert_with(Instant::now)
            .elapsed()
            .as_millis() as u64
    }
}

pub fn panel(
    v: &mut Controller,
    id: &str,
    bounds: pm_shell::layout::Rect,
    window: &mut Window,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    (id == "shader").then(|| render(v, bounds, window, cx))
}

impl Controller {
    fn shader_stamp(&self) -> Option<DocumentStamp> {
        let p = self.published.as_ref()?;
        Some(DocumentStamp {
            project: p.project.clone(),
            session: p.snapshot.doc_id,
            revision: p.revision(),
        })
    }
    /// The open composition's shader layers, read once per revision.
    fn shader_layers(&mut self) -> Vec<Layer> {
        let Some(p) = self.published.clone() else {
            return vec![];
        };
        let stamp = (p.snapshot.doc_id, p.revision());
        if let Some((at, layers)) = &self.shader.layers
            && *at == stamp
        {
            return layers.clone();
        }
        let layers = Self::read_shader_layers(&p);
        self.shader.layers = Some((stamp, layers.clone()));
        layers
    }
    fn read_shader_layers(p: &pm_kernel::Published) -> Vec<Layer> {
        let Some(root) = p.snapshot.root() else {
            return vec![];
        };
        let open = p.snapshot.open_comp();
        let Some(comp) = pm_kernel::model::comp_node(root, open.as_deref()) else {
            return vec![];
        };
        // Only shader layers are read; a large composition isn't converted.
        pm_kernel::model::list_layers(root, open.as_deref())
            .into_iter()
            .filter(|l| l.kind == "shader")
            .filter_map(|l| {
                let node = pm_kernel::model::layers_of(comp).nth(l.index)?;
                Some(Layer {
                    code: node
                        .get("d")
                        .and_then(|d| d.get("code"))
                        .and_then(crate::node_string)
                        .unwrap_or_default()
                        .into(),
                    id: l.id,
                    kind: "shader".into(),
                    locked: l.locked,
                })
            })
            .collect()
    }
    /// Keeps the editor on the shown layer and takes publications into it.
    fn shader_sync(&mut self, layer: Option<&Layer>, window: &mut Window, cx: &mut Context<Self>) {
        let stamp = self.shader_stamp();
        let same =
            self.shader.open.as_ref().map(|o| o.layer.as_str()) == layer.map(|l| l.id.as_str());
        if same {
            if let (Some(open), Some(layer), Some(stamp)) = (&mut self.shader.open, layer, stamp) {
                let before = open.editor.draft.clone();
                let _ = open.editor.publication(stamp, layer);
                if open.editor.draft != before {
                    let draft = open.editor.draft.clone();
                    open.field.update(cx, |f, cx| f.set_value(&draft, cx));
                }
            }
            return;
        }
        self.shader_finish(cx);
        self.shader.open = None;
        self.shader.status = None;
        let (Some(layer), Some(stamp)) = (layer, stamp) else {
            return;
        };
        let tokens = self.host.borrow().tokens.clone();
        let field = cx.new(|cx| {
            let mut f = TextField::new(
                TextKind::Multiline,
                "Shader source code",
                &layer.code,
                tokens,
                window,
                cx,
            );
            f.code_editor(pm_design::MONO_FONT);
            f
        });
        let events = cx.subscribe(&field, |v, _, event: &TextEvent, cx| match event {
            TextEvent::Changed(code) => {
                let now = v.shader.now();
                if let Some(open) = &mut v.shader.open {
                    open.editor.focus();
                    if open.editor.input(code.clone(), now).is_err() && open.editor.locked {
                        v.shader.status =
                            Some((false, "Unlock the layer to edit its shader.".into()));
                    }
                }
                v.shader_start(cx);
            }
            TextEvent::Committed(_) | TextEvent::Cancelled => v.shader_blur(cx),
            _ => {}
        });
        self.shader.open = Some(Open {
            layer: layer.id.clone(),
            editor: Editor::new(stamp, layer.id.clone(), layer.code.clone(), layer.locked),
            field,
            _events: events,
            gesture: None,
        });
        self.shader_compile(cx);
    }
    /// Pending typing, flushed after the pause; the final write ends the gesture.
    fn shader_flush(&mut self, force: bool, cx: &mut Context<Self>) {
        let now = self.shader.now();
        let Some(open) = &mut self.shader.open else {
            return;
        };
        if let Some(write) = open.editor.flush(now, force) {
            self.shader_send(write, cx);
        }
    }
    /// One write through the focus's gesture; the editor hears the real ack.
    fn shader_send(&mut self, write: pm_services::surfaces::shader::Write, cx: &mut Context<Self>) {
        let Some(open) = &self.shader.open else {
            return;
        };
        let Some(session) = &self.session else { return };
        let (kernel, project) = (self.kernel.clone(), session.project.clone());
        let gesture = open.gesture;
        let time = self.time();
        cx.spawn(async move |this, cx| {
            let call = |call: Call| kernel.call(&project, Request::new(Caller::native(), call));
            let mut gesture = gesture;
            let mut result: Result<(), String> = Ok(());
            if let Some(command) = &write.command {
                if gesture.is_none() {
                    match call(
                        GestureCall::Begin {
                            label: "Edit Shader".into(),
                        }
                        .into(),
                    )
                    .await
                    {
                        Some(Ok(ack)) => match ack.reply {
                            Reply::Gesture(id) => gesture = Some(id),
                            Reply::Throw(e) => result = Err(e.to_string()),
                            _ => result = Err("The edit wasn't accepted".into()),
                        },
                        _ => result = Err("The edit wasn't accepted".into()),
                    }
                }
                if let (Ok(()), Some(id)) = (&result, gesture) {
                    let patch = pm_model::json::parse(&command["patch"].to_string())
                        .ok()
                        .and_then(|p| match p {
                            pm_model::Value::Object(map) => Some(map),
                            _ => None,
                        })
                        .unwrap_or_default();
                    let step = pm_kernel::ContentCommand::Set {
                        layer_id: write.layer_id.clone(),
                        patch,
                        time,
                    };
                    let body = WriteBody::Commands(vec![step.into()]);
                    match call(
                        GestureCall::Step {
                            gesture: id,
                            body,
                            replace: true,
                        }
                        .into(),
                    )
                    .await
                    {
                        Some(Ok(ack)) if !matches!(ack.reply, Reply::Throw(_)) => {}
                        Some(Ok(ack)) => {
                            if let Reply::Throw(e) = ack.reply {
                                result = Err(e.to_string());
                            }
                        }
                        _ => result = Err("The edit wasn't accepted".into()),
                    }
                }
            }
            if write.finish_gesture
                && let Some(id) = gesture.take()
            {
                let _ = call(GestureCall::Commit { gesture: id }.into()).await;
            }
            let _ = this.update(cx, |v, cx| {
                let stamp = v.shader_stamp();
                if let Some(open) = &mut v.shader.open {
                    open.gesture = gesture;
                    let answer = match (result, stamp) {
                        (Ok(()), Some(stamp)) => Ok(stamp),
                        (Err(e), _) => Err(e),
                        (Ok(()), None) => Err("The project closed".into()),
                    };
                    open.editor.acknowledged(write.ticket, answer);
                }
                // The status follows the source once typing pauses.
                if v.shader
                    .open
                    .as_ref()
                    .is_some_and(|o| o.editor.compile().is_ok())
                {
                    v.shader_compile(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn shader_blur(&mut self, cx: &mut Context<Self>) {
        let now = self.shader.now();
        if let Some(open) = &mut self.shader.open
            && let Some(write) = open.editor.blur(now)
        {
            self.shader_send(write, cx);
        }
    }
    /// Ends the open editor's gesture (its layer is leaving the panel).
    fn shader_finish(&mut self, cx: &mut Context<Self>) {
        self.shader_blur(cx);
    }
    /// ⌘Return / Compile: the engine's check of the written source.
    fn shader_compile(&mut self, cx: &mut Context<Self>) {
        self.shader_flush(true, cx);
        let Some(open) = &self.shader.open else {
            return;
        };
        let Ok(input) = open.editor.compile() else {
            // Still writing: compile once the write lands.
            let entity = cx.entity();
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                entity.update(cx, |v, cx| v.shader_compile(cx));
            })
            .detach();
            return;
        };
        self.shader.compiling = Some(Instant::now());
        let layer = input.layer_id.clone();
        let job = cx
            .background_executor()
            .spawn(async move { EngineCompiler.compile(&input) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |v, cx| {
                v.shader.compiling = None;
                if v.shader.open.as_ref().map(|o| &o.layer) == Some(&layer) {
                    v.shader.status = Some(match result {
                        Ok(compiled) => status_line(&compiled),
                        Err(e) => (false, e.to_string()),
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn shader_start(&mut self, cx: &mut Context<Self>) {
        if self.shader.running {
            return;
        }
        self.shader.running = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |v, cx| {
                    v.shader_flush(false, cx);
                    let busy = v
                        .shader
                        .open
                        .as_ref()
                        .is_some_and(|o| !o.editor.can_retire());
                    if !busy
                        && v.shader
                            .painted
                            .is_none_or(|p| p.elapsed() > Duration::from_secs(1))
                    {
                        v.shader.running = false;
                        return false;
                    }
                    if v.shader
                        .compiling
                        .is_some_and(|c| c.elapsed() >= Duration::from_millis(500))
                    {
                        cx.notify();
                    }
                    true
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }
    /// Presets ▸ a shipped preset: the whole source, one `Shader Preset` step.
    pub fn shader_preset(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(preset) = pm_render::shader_layers::builtins().presets.get(index) else {
            return;
        };
        let Some(open) = &self.shader.open else {
            return;
        };
        if open.editor.locked {
            return self.note("Unlock the layer to edit its shader.".into(), cx);
        }
        // Typing so far is its own step, committed first; the preset is the next.
        if open.gesture.is_some() || !open.editor.can_retire() {
            self.shader_blur(cx);
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let _ = this.update(cx, |v, cx| v.shader_preset(index, cx));
            })
            .detach();
            return;
        }
        let mut patch = pm_model::Map::new();
        patch.insert("code", pm_model::Value::from(preset.code.as_str()));
        let command = pm_kernel::ContentCommand::Set {
            layer_id: open.layer.clone(),
            patch,
            time: self.time(),
        };
        self.captions_apply(
            "Shader Preset",
            vec![command.into()],
            |v, cx| {
                // The editor opens again on the preset's source.
                v.shader.open = None;
                cx.notify();
            },
            cx,
        );
    }
    pub fn shader_command(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        match name {
            "shader.compile" => self.shader_compile(cx),
            other => match other
                .strip_prefix("shader.preset:")
                .and_then(|i| i.parse().ok())
            {
                Some(index) => self.shader_preset(index, cx),
                None => return false,
            },
        }
        true
    }
}

fn render(
    v: &mut Controller,
    bounds: pm_shell::layout::Rect,
    window: &mut Window,
    cx: &mut Context<Controller>,
) -> AnyElement {
    let t = v.host.borrow().tokens.clone();
    let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
    let height = (bounds.h - m.header).max(0.);
    v.shader.painted = Some(Instant::now());
    let layers = v.shader_layers();
    let selection = v
        .published
        .as_ref()
        .map(|p| p.selection.layers().to_vec())
        .unwrap_or_default();
    let shown = selected_shader(&layers, &selection).cloned();
    v.shader_sync(shown.as_ref(), window, cx);
    let Some(open) = &v.shader.open else {
        return pm_shell::view::empty_state(
            &t,
            "No Shader Layers",
            "This project does not contain any editable shader layers.",
        )
        .h(px(height))
        .into_any_element();
    };
    let field = open.field.clone();
    let locked = open.editor.locked;
    let (ok, text) = if locked {
        (true, "Unlock the layer to edit its shader.".to_owned())
    } else if v
        .shader
        .compiling
        .is_some_and(|c| c.elapsed() >= Duration::from_millis(500))
    {
        (true, "Compiling…".to_owned())
    } else {
        v.shader.status.clone().unwrap_or((true, String::new()))
    };
    let compiling = text == "Compiling…";
    let symbol = (!text.is_empty() && !locked && !compiling).then(|| {
        pm_shell::symbol::symbol(
            if ok {
                "checkmark.circle.fill"
            } else {
                "exclamationmark.triangle.fill"
            },
            m.symbol_panel,
            pm_shell::symbol::Weight::Medium,
            if ok { c.success } else { c.danger },
            window,
        )
    });
    let presets: Vec<MenuEntry> = pm_render::shader_layers::builtins()
        .presets
        .iter()
        .enumerate()
        .map(|(i, p)| MenuEntry::item(p.name.clone(), format!("shader.preset:{i}")))
        .collect();
    let button = |id: &'static str, label: &'static str| {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .flex_none()
            .h(px(m.control))
            .px(px(m.gap * 1.5))
            .flex()
            .items_center()
            .rounded(px(m.control_radius))
            .bg(rgb(c.field))
            .border_1()
            .border_color(rgb(c.hairline))
            .text_size(px(ty.body))
            .text_color(rgb(c.text))
            .child(label)
    };
    let bar = div()
        .flex_none()
        .h(px(m.header))
        .px(px(m.inset))
        .flex()
        .items_center()
        .gap(px(m.gap))
        .border_t_1()
        .border_color(rgb(c.separator))
        .children(symbol)
        .child(
            div()
                .id("shader-status")
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_size(px(ty.body))
                .text_color(rgb(if locked { c.muted } else { c.text }))
                .child(text),
        )
        .child(button("shader-presets", "Presets").on_click(cx.listener(
            move |v, e: &ClickEvent, _, _| {
                let at = e.position();
                v.host
                    .borrow()
                    .context_menu(&presets, f32::from(at.x), f32::from(at.y));
            },
        )))
        .child(
            button("shader-compile", "Compile")
                .tooltip(pm_shell::view::tooltip(&t, "Compile · ⌘Return"))
                .on_click(cx.listener(|v, _, _, cx| v.shader_compile(cx))),
        );
    div()
        .id("shader-lab")
        .w_full()
        .h(px(height))
        .flex()
        .flex_col()
        .on_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
            if e.keystroke.key == "enter" && e.keystroke.modifiers.platform {
                v.shader_compile(cx);
                cx.stop_propagation();
            }
        }))
        .child(div().flex_1().min_h(px(0.)).p(px(m.gap)).child(field))
        .child(bar)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{CompileInput, Compiler, DocumentStamp, EngineCompiler, status_line};
    #[test]
    fn the_engine_checks_shader_source_and_counts_its_uniforms() {
        let good = pm_render::shader_layers::builtins().presets[0].code.clone();
        let input = |code: &str| CompileInput {
            stamp: DocumentStamp {
                project: "p".into(),
                session: 1,
                revision: 1,
            },
            layer_id: "s".into(),
            code: code.into(),
        };
        let compiled = EngineCompiler.compile(&input(&good)).unwrap();
        let (ok, line) = status_line(&compiled);
        assert!(ok, "{line}");
        assert!(line.starts_with("Compiled · "), "{line}");
        let broken = EngineCompiler
            .compile(&input("void main() {\n  gl_FragColor = vec4(1.0)\n}"))
            .unwrap();
        let (ok, line) = status_line(&broken);
        assert!(!ok);
        assert!(!line.is_empty());
    }
}
