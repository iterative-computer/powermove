//! Paste with Figma layers on the pasteboard (`runFigmaPaste`): the archive
//! is decoded and planned on a worker, then the layers land as one Undo
//! step, or the selection's preview image is offered as a fallback.
use crate::{
    alert::{Alert, Button},
    layer_commands::Plan,
    session::Controller,
};
use gpui::Context;
use pm_builtins::{
    figma,
    media_import::{ImportRequest, PlacedAs},
};
use pm_kernel::{Command, LayerCommand};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    PasteImage,
    Cancel,
    Ok,
}

impl Controller {
    /// Claims the paste when the pasteboard holds Figma layers.
    pub(crate) fn paste_figma(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((html, png)) = crate::platform::pasteboard_figma() else {
            return false;
        };
        self.paste_figma_html(html, png, cx)
    }

    /// The pasteboard's Figma HTML and PNG preview, read.
    pub(crate) fn paste_figma_html(
        &mut self,
        html: String,
        png: Option<Vec<u8>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(published) = self.published.clone() else {
            return false;
        };
        let Some(root) = published.snapshot.root() else {
            return false;
        };
        let open = published.snapshot.open_comp();
        let comp = pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root);
        let number = |key: &str, default: f64| {
            comp.get(key)
                .and_then(|n| n.to_value().as_f64())
                .filter(|n| n.is_finite() && *n > 0.)
                .unwrap_or(default)
        };
        let target = figma::Target {
            w: number("w", 1920.),
            h: number("h", 1080.),
            fps: number("fps", 30.),
            dur: number("dur", 10.),
        };
        let source = (published.snapshot.doc_id, open);
        let at = self.time();
        let fonts = crate::colour_panel::font_families();
        // Private-format bytes from another app: never on the main thread.
        let work = cx.background_executor().spawn(async move {
            let planned = figma::decode(&html).and_then(|s| figma::plan(&s, target, at, &fonts));
            let image = png.or_else(|| figma::inline_png(&html));
            (planned, image)
        });
        cx.spawn(async move |this, cx| {
            let (planned, image) = work.await;
            let _ = this.update(cx, |v, cx| {
                if !v.figma_target_is(&source) {
                    return;
                }
                match planned {
                    Ok(plan) if plan.issues.is_empty() => v.apply_figma(plan, cx),
                    Ok(plan) => {
                        let reasons = figma::reasons(None, &plan.issues);
                        v.offer_figma_image(reasons, image, Some(&plan), source, at, cx)
                    }
                    Err(problem) => {
                        let reasons = figma::reasons(Some(&problem), &[]);
                        v.offer_figma_image(reasons, image, None, source, at, cx)
                    }
                }
            });
        })
        .detach();
        true
    }

    /// The paste still has its destination: the same document and composition.
    fn figma_target_is(&self, source: &(u64, Option<String>)) -> bool {
        self.published
            .as_ref()
            .is_some_and(|p| p.snapshot.doc_id == source.0 && p.snapshot.open_comp() == source.1)
    }

    fn apply_figma(&mut self, plan: figma::Plan, cx: &mut Context<Self>) {
        let count = plan.layers.len();
        self.run_layer_plan(
            Plan {
                label: "Paste Figma layers".into(),
                commands: vec![Command::Layer(LayerCommand::InsertBlock {
                    label: "Paste Figma layers".into(),
                    layers: plan.layers,
                })],
                select: Some(plan.roots),
                count_keys: None,
            },
            cx,
        );
        self.message = format!(
            "Pasted {count} Figma {}",
            if count == 1 { "layer" } else { "layers" }
        );
        cx.notify();
    }

    /// Explains what can't stay editable, then pastes the whole selection
    /// as one image (fitted to the selection's size) once the person agrees.
    fn offer_figma_image(
        &mut self,
        reasons: String,
        image: Option<Vec<u8>>,
        plan: Option<&figma::Plan>,
        source: (u64, Option<String>),
        at: f64,
        cx: &mut Context<Self>,
    ) {
        let window = self.host.borrow().window.clone();
        let Some(image) = image else {
            let alert = Alert::new(
                "These Figma layers need an image fallback",
                format!(
                    "{reasons}\n\nFigma did not include a preview image. Use Copy as PNG in Figma, \
                     then paste again. The selection will paste as one image; its individual parts \
                     will not be editable."
                ),
                vec![Button::new("OK", Choice::Ok).default()],
            );
            let _ = alert.sheet(&window);
            return;
        };
        let fit = plan.map(|p| (p.width, p.height));
        let alert = Alert::new(
            "Paste Figma selection as an image?",
            format!(
                "{reasons}\n\nThese features cannot stay editable yet. The entire selection will \
                 use an image fallback. You can animate the image, but its individual parts will \
                 not be editable."
            ),
            vec![
                Button::new("Paste as Image", Choice::PasteImage).default(),
                Button::new("Cancel", Choice::Cancel).cancel(),
            ],
        );
        let answer = alert.sheet(&window);
        cx.spawn(async move |this, cx| {
            if answer.recv().await.ok().flatten() != Some(Choice::PasteImage) {
                return;
            }
            let _ = this.update(cx, |v, cx| {
                if !v.figma_target_is(&source) {
                    return;
                }
                let staged = tempfile::Builder::new()
                    .prefix("powermove-figma-")
                    .tempdir()
                    .and_then(|dir| {
                        let path = dir.path().join("Figma selection.png");
                        std::fs::write(&path, &image)?;
                        Ok((dir, path))
                    });
                match staged {
                    Ok((dir, path)) => v.import(
                        ImportRequest {
                            paths: vec![path],
                            place_at: Some(at),
                            placed_as: Some(PlacedAs {
                                name: "Figma selection (image fallback)".into(),
                                fit,
                            }),
                            staged: Some(dir),
                            note: Some("Pasted Figma selection as an image fallback".into()),
                            ..Default::default()
                        },
                        cx,
                    ),
                    Err(error) => {
                        v.message = format!("Could not paste the Figma preview image: {error}");
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}
