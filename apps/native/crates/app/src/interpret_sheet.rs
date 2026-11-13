//! Interpret Footage…: how the selected clip's file is read. Each value shows
//! what the file says (or the rule that filled it in), and can be set here to
//! correct a wrong tag: primaries, transfer, matrix, range and alpha. One
//! Apply is one Undo step on the asset, so the viewer, proxies, the render
//! cache and exports all read it the same way.
use crate::{
    dialogs::{DialogId, Field, FieldGroup, NativePiece, Surface, WindowId},
    session::Controller,
    sheets::{OpenSheet, from_surface},
};
use gpui::{AppContext, Context};
use pm_media::interpret::{AlphaChoice, Field as Read, Interpretation, Origin, VideoReading};
use pm_model::{Map, Value};
use pm_protocol::color::{AlphaMode, MatrixCoefficients, Primaries, Range, Transfer};
use std::{path::Path, rc::Rc};

const PRIMARIES: [(&str, Primaries); 11] = [
    ("Rec. 709", Primaries::Bt709),
    ("Rec. 601 (625-line, PAL)", Primaries::Bt601_625),
    ("Rec. 601 (525-line, NTSC)", Primaries::Bt601_525),
    ("Rec. 2020", Primaries::Bt2020),
    ("Display P3", Primaries::DisplayP3),
    ("DCI-P3", Primaries::DciP3),
    ("S-Gamut3", Primaries::SGamut3),
    ("S-Gamut3.Cine", Primaries::SGamut3Cine),
    ("V-Gamut", Primaries::VGamut),
    ("ARRI Wide Gamut 3", Primaries::AwgGen3),
    ("ARRI Wide Gamut 4", Primaries::AwgGen4),
];
const TRANSFERS: [(&str, Transfer); 12] = [
    ("sRGB / Rec. 709 (display)", Transfer::DisplayE),
    ("Rec. 1886 (gamma 2.4)", Transfer::Bt1886),
    ("Rec. 709 (scene)", Transfer::SceneReferredBt709),
    ("Gamma 2.2", Transfer::Gamma22),
    ("Linear", Transfer::Linear),
    ("PQ (ST 2084)", Transfer::Pq),
    ("HLG", Transfer::Hlg),
    ("Apple Log", Transfer::AppleLog),
    ("S-Log3", Transfer::SLog3),
    ("V-Log", Transfer::VLog),
    ("LogC3 EI 800", Transfer::LogC3),
    ("LogC4", Transfer::LogC4),
];
const MATRICES: [(&str, MatrixCoefficients); 4] = [
    ("Rec. 709", MatrixCoefficients::Bt709),
    ("Rec. 601", MatrixCoefficients::Bt601),
    ("Rec. 2020", MatrixCoefficients::Bt2020Ncl),
    ("RGB (no matrix)", MatrixCoefficients::Identity),
];
const RANGES: [(&str, Range); 2] = [
    ("Limited (video, 16–235)", Range::Limited),
    ("Full (data, 0–255)", Range::Full),
];
const ALPHAS: [(&str, AlphaChoice); 3] = [
    ("Ignore", AlphaChoice::Ignore),
    ("Straight", AlphaChoice::Straight),
    ("Premultiplied", AlphaChoice::Premultiplied),
];

/// A value as the document stores it (the engine's own name for it).
fn stored<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn label<T: PartialEq + std::fmt::Debug>(choices: &[(&str, T)], value: &T) -> String {
    choices
        .iter()
        .find(|(_, v)| v == value)
        .map_or_else(|| format!("{value:?}"), |(label, _)| (*label).to_owned())
}

/// Where the value in use came from, as the line under its pop-up.
fn origin(origin: &Origin, automatic: &str) -> String {
    match origin {
        Origin::File { source } => format!("From the file ({source})."),
        Origin::Default { rule } => format!("The file doesn’t say: {rule}."),
        Origin::Override => format!("Set here. Automatic is {automatic}."),
    }
}

/// One pop-up: Automatic (naming what that is, when the file was read), then
/// every choice; `chosen` is the override in the document.
fn popup<T: PartialEq + std::fmt::Debug>(
    id: &str,
    name: &str,
    choices: &[(&str, T)],
    chosen: Option<&T>,
    read: Option<(&T, &Origin)>,
) -> Field {
    let automatic = read.map(|(value, _)| label(choices, value));
    let mut values = vec![automatic.as_ref().map_or_else(
        || "Automatic".to_owned(),
        |automatic| format!("Automatic ({automatic})"),
    )];
    values.extend(choices.iter().map(|(label, _)| (*label).to_owned()));
    let mut field = Field::new(id, name, NativePiece::PopUpButton, values);
    field.selected = chosen
        .and_then(|value| choices.iter().position(|(_, v)| v == value))
        .map_or(0, |index| index + 1);
    field.description = read
        .zip(automatic)
        .map(|((_, from), automatic)| origin(from, &automatic));
    field
}

fn alpha_choice(mode: &AlphaMode) -> AlphaChoice {
    match mode {
        AlphaMode::Opaque => AlphaChoice::Ignore,
        AlphaMode::Straight => AlphaChoice::Straight,
        AlphaMode::Premultiplied | AlphaMode::PremultipliedWithColor { .. } => {
            AlphaChoice::Premultiplied
        }
    }
}

/// The automatic value of a read field and where it came from.
fn read<T>(field: &Read<T>) -> (&T, &Origin) {
    (&field.automatic, &field.origin)
}

/// The sheet for `name`. `video` has a matrix and a range; a still does not.
/// `reading` is the file as the engine reads it, when the file could be read.
pub fn surface(
    id: DialogId,
    owner: WindowId,
    name: &str,
    video: bool,
    chosen: &Interpretation,
    reading: Option<&VideoReading>,
) -> Surface {
    let alpha = reading.map(|r| (alpha_choice(&r.alpha.automatic), r.alpha.origin.clone()));
    let mut matrix = popup(
        "matrix",
        "Matrix",
        &MATRICES,
        chosen.matrix.as_ref(),
        reading.map(|r| read(&r.matrix)),
    );
    let mut range = popup(
        "range",
        "Range",
        &RANGES,
        chosen.range.as_ref(),
        reading.map(|r| read(&r.range)),
    );
    matrix.enabled = video;
    range.enabled = video;
    let fields = vec![
        popup(
            "primaries",
            "Primaries",
            &PRIMARIES,
            chosen.primaries.as_ref(),
            reading.map(|r| read(&r.primaries)),
        ),
        popup(
            "transfer",
            "Transfer",
            &TRANSFERS,
            chosen.transfer.as_ref(),
            reading.map(|r| read(&r.transfer)),
        ),
        matrix,
        range,
        popup(
            "alpha",
            "Alpha",
            &ALPHAS,
            chosen.alpha.as_ref(),
            alpha.as_ref().map(|(value, origin)| (value, origin)),
        ),
    ];
    let mut surface = Surface::sheet(id, owner, "Interpret Footage", fields, "Apply");
    surface.line = Some(name.to_owned());
    surface.groups = vec![
        FieldGroup {
            title: "Color",
            fields: vec!["primaries", "transfer", "matrix", "range"],
        },
        FieldGroup {
            title: "Alpha",
            fields: vec!["alpha"],
        },
    ];
    surface
}

/// What the pop-ups say, as indexes: 0 is Automatic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chosen {
    pub primaries: usize,
    pub transfer: usize,
    pub matrix: usize,
    pub range: usize,
    pub alpha: usize,
}

impl Chosen {
    pub fn read(popup: impl Fn(&str) -> Option<usize>) -> Self {
        let at = |id: &str| popup(id).unwrap_or(0);
        Self {
            primaries: at("primaries"),
            transfer: at("transfer"),
            matrix: at("matrix"),
            range: at("range"),
            alpha: at("alpha"),
        }
    }

    /// Writes the choices into an asset's `interpret` member: Automatic
    /// removes the override, and every other member stays as it was.
    pub fn write(&self, interpret: &mut Map) {
        fn set<T: serde::Serialize>(
            interpret: &mut Map,
            key: &str,
            choices: &[(&str, T)],
            index: usize,
        ) {
            match index.checked_sub(1).and_then(|i| choices.get(i)) {
                Some((_, value)) => {
                    interpret.insert(key, Value::from(stored(value).as_str()));
                }
                None => {
                    interpret.remove(key);
                }
            }
        }
        set(interpret, "primaries", &PRIMARIES, self.primaries);
        set(interpret, "transfer", &TRANSFERS, self.transfer);
        set(interpret, "matrix", &MATRICES, self.matrix);
        set(interpret, "range", &RANGES, self.range);
        set(interpret, "alpha", &ALPHAS, self.alpha);
    }
}

/// Worker: the file as the engine reads it under `chosen`.
fn video_reading(path: &Path, chosen: &Interpretation) -> Option<VideoReading> {
    let probed = pm_media::probe::probe(path).ok()?;
    let video = probed.probe.video.as_ref()?;
    Some(pm_media::interpret::resolve_video(video, chosen))
}

impl Controller {
    /// `Interpret Footage…` on the selected clip's asset.
    pub fn interpret_footage_sheet(&mut self, cx: &mut Context<Self>) {
        const SELECT: &str = "Select a video or image clip to interpret";
        let (Some(p), Some(layer), Some(session)) = (
            self.published.clone(),
            self.selected.clone(),
            self.session.clone(),
        ) else {
            self.message = SELECT.into();
            cx.notify();
            return;
        };
        let record = crate::colour::layer_asset(&p, &layer).and_then(|asset| {
            let record = p.snapshot.root()?.get("assets")?.get(&asset)?;
            let kind = record.get("kind")?.to_value().as_str()?.to_owned();
            matches!(kind.as_str(), "video" | "image").then(|| {
                let name = record
                    .get("name")
                    .and_then(|n| n.to_value().as_str().map(str::to_owned))
                    .unwrap_or_else(|| asset.clone());
                let chosen = pm_render::footage::interpretation(record).unwrap_or_default();
                (asset, kind == "video", name, chosen)
            })
        });
        let Some((asset, video, name, chosen)) = record else {
            self.message = SELECT.into();
            cx.notify();
            return;
        };
        let path = self.media.originals.get(&asset).cloned();
        cx.spawn(async move |this, cx| {
            // The file is read off the main thread; an offline or still file
            // opens the sheet without what "Automatic" resolves to.
            let reading = cx
                .background_spawn({
                    let chosen = chosen.clone();
                    async move {
                        path.filter(|_| video)
                            .and_then(|p| video_reading(&p, &chosen))
                    }
                })
                .await;
            let surface = surface(
                DialogId(format!("interpret:{}", session.project)),
                WindowId(session.project.clone()),
                &name,
                video,
                &chosen,
                reading.as_ref(),
            );
            let Ok(open) = this.update(cx, |v, _| {
                let host = v.host.borrow();
                Rc::new(OpenSheet::present(
                    from_surface(&surface, &host.tokens),
                    &host.window,
                    None,
                ))
            }) else {
                return;
            };
            let actions = open.actions.clone();
            while let Ok(action) = actions.recv().await {
                match action.as_str() {
                    "cancel" => break,
                    "apply" => {
                        let chosen = Chosen::read(|id| open.popup(id));
                        let _ = this.update(cx, |v, cx| {
                            v.change_interpretation(
                                |p, _| {
                                    crate::colour::edit_interpretation(p, &layer, |interpret| {
                                        chosen.write(interpret)
                                    })
                                },
                                cx,
                            )
                        });
                        break;
                    }
                    _ => {}
                }
            }
            open.close();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (DialogId, WindowId) {
        (DialogId("i".into()), WindowId("w".into()))
    }

    /// Every stored name is one the engine parses back to the same value.
    #[test]
    fn every_choice_is_a_value_the_engine_reads() {
        let all = Chosen {
            primaries: PRIMARIES.len(),
            transfer: TRANSFERS.len(),
            matrix: MATRICES.len(),
            range: RANGES.len(),
            alpha: ALPHAS.len(),
        };
        for step in 0..PRIMARIES.len().max(TRANSFERS.len()) {
            let at = |count: usize| 1 + step % count;
            let chosen = Chosen {
                primaries: at(all.primaries),
                transfer: at(all.transfer),
                matrix: at(all.matrix),
                range: at(all.range),
                alpha: at(all.alpha),
            };
            let mut interpret = Map::new();
            chosen.write(&mut interpret);
            let json: serde_json::Value = serde_json::from_str(
                &pm_model::json::to_canonical_string(&Value::Object(interpret)),
            )
            .unwrap();
            let (parsed, issues) = Interpretation::from_json(&json);
            assert!(issues.is_empty(), "{issues:?}");
            assert_eq!(parsed.primaries, Some(PRIMARIES[chosen.primaries - 1].1));
            assert_eq!(parsed.transfer, Some(TRANSFERS[chosen.transfer - 1].1));
            assert_eq!(parsed.matrix, Some(MATRICES[chosen.matrix - 1].1));
            assert_eq!(parsed.range, Some(RANGES[chosen.range - 1].1));
            assert_eq!(parsed.alpha, Some(ALPHAS[chosen.alpha - 1].1));
        }
    }

    #[test]
    fn automatic_removes_an_override_and_keeps_the_other_members() {
        let mut interpret = Map::new();
        interpret.insert("range", Value::from("full"));
        interpret.insert("cleanAperture", Value::from(true));
        Chosen {
            matrix: 2,
            ..Chosen::default()
        }
        .write(&mut interpret);
        assert!(interpret.get("range").is_none(), "Automatic removes it");
        assert_eq!(interpret.get("matrix"), Some(&Value::from("bt601")));
        assert_eq!(interpret.get("cleanAperture"), Some(&Value::from(true)));
    }

    #[test]
    fn the_sheet_shows_the_override_and_what_the_file_says() {
        let chosen = Interpretation {
            range: Some(Range::Full),
            ..Interpretation::default()
        };
        let (id, owner) = ids();
        let sheet = surface(id, owner, "Screen Recording.mov", true, &chosen, None);
        assert_eq!(sheet.line.as_deref(), Some("Screen Recording.mov"));
        let field = |id: &str| sheet.fields.iter().find(|f| f.id == id).unwrap();
        assert_eq!(field("range").selected, 2, "Full is chosen");
        assert_eq!(field("primaries").selected, 0, "Automatic");
        assert_eq!(field("primaries").values[0], "Automatic");
        // The pop-ups read back as what they show.
        let read = Chosen::read(|id| Some(field(id).selected));
        assert_eq!(read.range, 2);
        // A still has no matrix or range to set.
        let (id, owner) = ids();
        let still = surface(
            id,
            owner,
            "Plate.png",
            false,
            &Interpretation::default(),
            None,
        );
        let field = |id: &str| still.fields.iter().find(|f| f.id == id).unwrap();
        assert!(!field("matrix").enabled && !field("range").enabled);
        assert!(field("alpha").enabled);
    }

    #[test]
    fn each_value_names_where_it_came_from() {
        let from_file = Origin::File {
            source: "colr nclx".into(),
        };
        assert_eq!(origin(&from_file, "Rec. 709"), "From the file (colr nclx).");
        assert_eq!(
            origin(&Origin::Override, "Rec. 601"),
            "Set here. Automatic is Rec. 601."
        );
        let field = popup(
            "matrix",
            "Matrix",
            &MATRICES,
            Some(&MatrixCoefficients::Bt709),
            Some((&MatrixCoefficients::Bt601, &Origin::Override)),
        );
        assert_eq!(field.values[0], "Automatic (Rec. 601)");
        assert_eq!(field.selected, 1);
        assert_eq!(
            field.description.as_deref(),
            Some("Set here. Automatic is Rec. 601.")
        );
    }
}
