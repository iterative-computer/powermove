//! File ▸ Import Image Sequence… and numbered stills picked or dropped as
//! media: today's `chooseSequence` sheet, then an import that reads the
//! frames into one clip on the import worker (`pm_builtins::media_import`).
use crate::{platform, session::Controller, sheets::OpenSheet};
use gpui::Context;
use pm_builtins::media_import::{ImportRequest, SequenceImport, order_sequence};
use std::{path::PathBuf, rc::Rc};

/// The picker's filter: `IMAGE_SEQUENCE_ACCEPT`, and the film scan and
/// design formats renders come in.
pub const ACCEPT: [&str; 11] = [
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "exr", "dpx", "tga", "psd",
];
const TITLE: &str = "Import Image Sequence";
const INTRO: &str = "Turn your numbered images into one clip.";
const SKIPPED: &str = "You can still import. Missing frames show as Media Offline until they are rendered into the folder.";

/// `sequenceCandidate`: two or more files sharing a name and image extension,
/// each ending in a frame number.
pub fn candidate(paths: &[PathBuf]) -> bool {
    fn frame(path: &std::path::Path) -> Option<(String, String)> {
        let name = path.file_name()?.to_str()?;
        let (stem, extension) = name.rsplit_once('.')?;
        let extension = extension.to_ascii_lowercase();
        let prefix = stem.trim_end_matches(|c: char| c.is_ascii_digit());
        (ACCEPT.contains(&extension.as_str()) && prefix.len() < stem.len())
            .then(|| (prefix.to_owned(), extension))
    }
    let first = paths.first().and_then(|p| frame(p));
    paths.len() >= 2 && first.is_some() && paths.iter().all(|p| frame(p) == first)
}

/// What the sheet shows for one selection, recomputed as the rate is typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub selected: Vec<PathBuf>,
    /// The frames in order, or why they can't import.
    pub ordered: Result<Vec<PathBuf>, String>,
    gaps: Vec<(i64, i64)>,
    /// Import Image Sequence: no Individual Images button.
    pub required: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Sequence { frames: Vec<PathBuf>, fps: f64 },
    Individual(Vec<PathBuf>),
    Reimport,
}
impl Choice {
    pub fn new(selected: Vec<PathBuf>, required: bool) -> Self {
        let ordered = order_sequence(&selected);
        let gaps = ordered
            .as_ref()
            .ok()
            .and_then(|_| pm_media::image::sequence::ImageSequence::from_paths(&selected).ok())
            .map(|s| s.gaps)
            .unwrap_or_default();
        Self {
            selected,
            ordered,
            gaps,
            required,
        }
    }
    pub fn count(&self) -> String {
        format!("{} frames selected", self.selected.len())
    }
    pub fn files(&self) -> String {
        let name = |p: Option<&PathBuf>| {
            p.and_then(|p| p.file_name())
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        };
        let list = self.ordered.as_ref().unwrap_or(&self.selected);
        format!("{} → {}", name(list.first()), name(list.last()))
    }
    /// `validSequenceFps`, read from the field's text.
    pub fn fps(text: &str) -> Option<f64> {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite() && (1.0..=240.0).contains(f))
    }
    pub fn duration(&self, fps: &str) -> String {
        Self::fps(fps).map_or_else(
            || "—".into(),
            |fps| format!("{:.3} s", self.selected.len() as f64 / fps),
        )
    }
    /// The warning's title and text: a selection that can't import, or the
    /// missing numbers that will be skipped.
    pub fn warning(&self) -> Option<(String, String)> {
        if let Err(why) = &self.ordered {
            return Some(("Check your selection".into(), why.clone()));
        }
        let missing: i64 = self.gaps.iter().map(|(a, b)| b - a + 1).sum();
        if missing == 0 {
            return None;
        }
        let ranges = self
            .gaps
            .iter()
            .take(8)
            .map(|(a, b)| {
                if a == b {
                    a.to_string()
                } else {
                    format!("{a}–{b}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let more = match self.gaps.len() {
            n if n > 8 => format!(", and {} more gaps", n - 8),
            _ => String::new(),
        };
        let noun = if missing == 1 { "number" } else { "numbers" };
        Some((
            format!("Missing frame {noun}: {ranges}{more}"),
            SKIPPED.into(),
        ))
    }
    /// The sheet's button ids, in reading order.
    pub fn buttons(&self) -> Vec<&'static str> {
        let mut out = vec![];
        if !self.required {
            out.push("individual");
        }
        out.extend(["cancel", "import"]);
        out
    }
    pub fn can_import(&self, fps: &str) -> bool {
        self.ordered.is_ok() && Self::fps(fps).is_some()
    }
    /// A button press; `None` closes without importing.
    pub fn answer(&self, id: &str, fps: &str) -> Option<Answer> {
        match id {
            "import" if self.can_import(fps) => Some(Answer::Sequence {
                frames: self.ordered.clone().ok()?,
                fps: Self::fps(fps)?,
            }),
            "individual" if !self.required => Some(Answer::Individual(self.selected.clone())),
            "reimport" => Some(Answer::Reimport),
            _ => None,
        }
    }
}

/// `fps` as the field shows it: `30`, `29.97`.
fn rate_text(fps: f64) -> String {
    let text = format!("{fps:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

impl Controller {
    /// File ▸ Import Image Sequence…: choose the frames, then the sheet.
    pub(crate) fn pick_sequence(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            self.message = "Open a project before importing media".into();
            cx.notify();
            return;
        }
        let rx = platform::import_paths(&self.host.borrow().window, false, true, &ACCEPT);
        cx.spawn(async move |this, cx| {
            if let Ok(paths) = rx.recv().await
                && !paths.is_empty()
            {
                let _ = this.update(cx, |v, cx| {
                    let at = Some(v.time());
                    v.choose_sequence(paths, true, at, None, cx)
                });
            }
        })
        .detach();
    }

    /// The sheet for `paths`; its answer imports one clip, the separate
    /// images, or chooses again. Layers land at `place_at`, as the import
    /// that brought the files would have placed them.
    pub(crate) fn choose_sequence(
        &mut self,
        paths: Vec<PathBuf>,
        required: bool,
        place_at: Option<f64>,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let choice = Choice::new(paths, required);
        let fps = rate_text(self.composition_fps());
        let (window, tokens) = {
            let host = self.host.borrow();
            (host.window.clone(), host.tokens.clone())
        };
        let (open, duration) = sheet(&choice, &fps, &window, &tokens);
        let open = Rc::new(open);
        let actions = open.actions.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                let rate = open.text("fps");
                if action == "changed:fps" {
                    duration.setStringValue(&objc2_foundation::NSString::from_str(
                        &choice.duration(&rate),
                    ));
                    open.enable("import", choice.can_import(&rate));
                    continue;
                }
                if action == "fps" || (action == "import" && !choice.can_import(&rate)) {
                    continue;
                }
                if !matches!(
                    action.as_str(),
                    "import" | "individual" | "reimport" | "cancel"
                ) {
                    continue;
                }
                open.close();
                let answer = choice.answer(&action, &rate);
                let _ = this.update(cx, |v, cx| match answer {
                    Some(Answer::Sequence { frames, fps }) => {
                        let progress = Default::default();
                        v.import(
                            ImportRequest {
                                paths: frames,
                                place_at,
                                folder: folder.clone(),
                                sequence: Some(SequenceImport { fps, progress }),
                                ..Default::default()
                            },
                            cx,
                        );
                    }
                    Some(Answer::Individual(paths)) => v.import(
                        ImportRequest {
                            paths,
                            place_at,
                            folder: folder.clone(),
                            ..Default::default()
                        },
                        cx,
                    ),
                    Some(Answer::Reimport) => v.repick_sequence(required, place_at, folder, cx),
                    None => {}
                });
                return;
            }
        })
        .detach();
    }

    /// Reimport…: the picker again, then the same sheet for the new frames.
    fn repick_sequence(
        &mut self,
        required: bool,
        place_at: Option<f64>,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let rx = platform::import_paths(&self.host.borrow().window, false, true, &ACCEPT);
        cx.spawn(async move |this, cx| {
            if let Ok(paths) = rx.recv().await
                && !paths.is_empty()
            {
                let _ = this.update(cx, |v, cx| {
                    v.choose_sequence(paths, required, place_at, folder, cx)
                });
            }
        })
        .detach();
    }

    /// The open composition's rate: the sheet's starting frame rate.
    fn composition_fps(&self) -> f64 {
        let published = self.published.as_ref();
        published
            .and_then(|p| {
                let root = p.snapshot.root()?;
                let open = p.snapshot.open_comp();
                let comp = pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root);
                comp.get("fps")
                    .or_else(|| root.get("fps"))
                    .and_then(|n| n.to_value().as_f64())
            })
            .and_then(|fps| Choice::fps(&fps.to_string()))
            .unwrap_or(30.)
    }
}

/// The sheet on the shared Settings-sheet recipe, and its duration label.
fn sheet(
    choice: &Choice,
    fps: &str,
    window: &objc2_app_kit::NSWindow,
    tokens: &pm_design::Tokens,
) -> (OpenSheet, objc2::rc::Retained<objc2_app_kit::NSTextField>) {
    use crate::dialogs::sheet_layout::native::{Sheet, colour};
    use objc2_app_kit::{NSAccessibility, NSFont, NSTextField};
    use objc2_foundation::NSString;
    let mtm = objc2::MainThreadMarker::new().expect("sheets open on the main thread");
    let warning = choice.warning();
    let mut sheet = Sheet::new(TITLE, tokens, false, mtm);
    let (body, minimum) = (
        f64::from(tokens.typography.body),
        f64::from(tokens.typography.minimum),
    );
    sheet.line(INTRO, body, tokens.colors.secondary);
    sheet.line(&choice.count(), body, tokens.colors.text);
    sheet.line(&choice.files(), minimum, tokens.colors.muted);
    sheet.number("fps", "Frame rate", fps, "fps", true);
    sheet.note("1–240 fps", "1–240 fps", tokens.colors.muted);
    let duration = NSTextField::labelWithString(&NSString::from_str(&choice.duration(fps)), mtm);
    duration.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
        body, 0.,
    )));
    duration.setTextColor(Some(&colour(tokens.colors.text)));
    duration.setAccessibilityLabel(Some(&NSString::from_str("Clip duration")));
    sheet.row("Clip duration", &duration);
    if let Some((title, text)) = &warning {
        sheet.line(title, body, tokens.colors.text);
        sheet.line(text, minimum, tokens.colors.muted);
        sheet.button("reimport", "Reimport…", true, false, false);
    }
    if !choice.required {
        sheet.button("individual", "Individual Images", true, false, false);
    }
    sheet.button("cancel", "Cancel", true, false, true);
    sheet.button(
        "import",
        "Import Sequence",
        choice.can_import(fps),
        true,
        false,
    );
    sheet.footer(&choice.buttons(), warning.is_some().then_some("reimport"));
    (
        OpenSheet::present((sheet, Default::default()), window, Some("fps")),
        duration,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|n| PathBuf::from(format!("/r/{n}")))
            .collect()
    }

    #[test]
    fn numbered_stills_are_offered_as_a_sequence_and_anything_else_is_not() {
        assert!(candidate(&paths(&["a_1.png", "a_2.PNG"])));
        assert!(candidate(&paths(&["shot.0001.jpg", "shot.0003.jpg"])));
        // Film scans and renders from compositing and design tools.
        for ext in ["dpx", "tga", "psd"] {
            assert!(candidate(&paths(&[
                &format!("scan.0001.{ext}"),
                &format!("scan.0002.{ext}")
            ])));
        }
        assert!(!candidate(&paths(&["a_1.png"])));
        assert!(!candidate(&paths(&["a_1.png", "b_2.png"])));
        assert!(!candidate(&paths(&["a_1.png", "a_2.jpg"])));
        assert!(!candidate(&paths(&["a.png", "a_2.png"])));
        assert!(!candidate(&paths(&["a_1.mov", "a_2.mov"])));
    }

    #[test]
    fn the_sheet_reads_frames_rate_duration_and_skipped_numbers() {
        let choice = Choice::new(paths(&["f_7.png", "f_1.png", "f_2.png", "f_4.png"]), true);
        assert_eq!(choice.count(), "4 frames selected");
        assert_eq!(choice.files(), "f_1.png → f_7.png");
        assert_eq!(choice.duration("24"), "0.167 s");
        assert_eq!(choice.duration("0"), "—");
        assert_eq!(choice.duration("abc"), "—");
        assert_eq!(
            choice.warning(),
            Some(("Missing frame numbers: 3, 5–6".into(), SKIPPED.into()))
        );
        assert_eq!(choice.buttons(), ["cancel", "import"]);
        assert!(!choice.can_import("241"));
        assert_eq!(choice.answer("import", "241"), None);
        assert_eq!(
            choice.answer("import", " 12 "),
            Some(Answer::Sequence {
                frames: paths(&["f_1.png", "f_2.png", "f_4.png", "f_7.png"]),
                fps: 12.
            })
        );
        assert_eq!(choice.answer("individual", "12"), None, "required");
    }

    #[test]
    fn one_missing_number_many_gaps_and_a_bad_selection_read_as_today() {
        let one = Choice::new(paths(&["f_1.png", "f_3.png"]), false);
        assert_eq!(one.warning().unwrap().0, "Missing frame number: 2");
        assert_eq!(one.buttons(), ["individual", "cancel", "import"]);
        assert_eq!(
            one.answer("individual", ""),
            Some(Answer::Individual(paths(&["f_1.png", "f_3.png"])))
        );
        let many: Vec<String> = (0..10).map(|i| format!("f_{}.png", i * 3)).collect();
        let many = Choice::new(
            paths(&many.iter().map(String::as_str).collect::<Vec<_>>()),
            true,
        );
        assert!(
            many.warning()
                .unwrap()
                .0
                .ends_with("22–23, and 1 more gaps"),
            "{:?}",
            many.warning()
        );
        let bad = Choice::new(paths(&["f_1.png", "f_01.png"]), true);
        assert_eq!(
            bad.warning(),
            Some((
                "Check your selection".into(),
                "Duplicate frame number 1".into()
            ))
        );
        assert!(!bad.can_import("30"));
        assert!(
            Choice::new(paths(&["f_1.png", "f_2.png"]), true)
                .warning()
                .is_none()
        );
    }

    #[test]
    fn rates_show_without_trailing_zeros() {
        assert_eq!(rate_text(30.), "30");
        assert_eq!(rate_text(29.97), "29.97");
        assert_eq!(rate_text(23.976), "23.976");
    }
}
