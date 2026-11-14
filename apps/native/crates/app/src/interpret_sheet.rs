//! Interpret Footage…: how the selected clip's file is read. Each value shows
//! what the file says (or the rule that filled it in), and can be set here to
//! correct a wrong tag: primaries, transfer, matrix, range and alpha; a
//! still with a gain map can be read for its SDR base only. One
//! Apply is one Undo step on the asset, so the viewer, proxies, the render
//! cache and exports all read it the same way.
use crate::{
    dialogs::{DialogId, Field, FieldGroup, NativePiece, Surface, WindowId},
    session::Controller,
    sheets::{OpenSheet, from_surface},
};
use gpui::{AppContext, Context};
use pm_media::image::Rendition;
use pm_media::interpret::{AlphaChoice, Field as Read, Interpretation, Origin, VideoReading};
use pm_model::{Map, Value};
use pm_protocol::color::{AlphaMode, MatrixCoefficients, Primaries, Range, Transfer};
use std::{path::Path, rc::Rc};

// Every value the engine reads is a choice here: an override the sheet could
// not show would look like Automatic.
const PRIMARIES: [(&str, Primaries); 14] = [
    ("Rec. 709", Primaries::Bt709),
    ("Rec. 601 (625-line, PAL)", Primaries::Bt601_625),
    ("Rec. 601 (525-line, NTSC)", Primaries::Bt601_525),
    ("Rec. 2020", Primaries::Bt2020),
    ("Display P3", Primaries::DisplayP3),
    ("DCI-P3", Primaries::DciP3),
    ("NTSC 1953 (BT.470 System M)", Primaries::Bt470M),
    ("EBU Tech 3213", Primaries::Ebu3213),
    ("Generic Film", Primaries::GenericFilm),
    ("S-Gamut3", Primaries::SGamut3),
    ("S-Gamut3.Cine", Primaries::SGamut3Cine),
    ("V-Gamut", Primaries::VGamut),
    ("ARRI Wide Gamut 3", Primaries::AwgGen3),
    ("ARRI Wide Gamut 4", Primaries::AwgGen4),
];
const TRANSFERS: [(&str, Transfer); 14] = [
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
    ("Logarithmic (100:1)", Transfer::Log100),
    ("Logarithmic (316:1)", Transfer::Log316),
];
const MATRICES: [(&str, MatrixCoefficients); 5] = [
    ("Rec. 709", MatrixCoefficients::Bt709),
    ("Rec. 601", MatrixCoefficients::Bt601),
    ("Rec. 2020", MatrixCoefficients::Bt2020Ncl),
    (
        "Rec. 2020 (constant luminance)",
        MatrixCoefficients::Bt2020Cl,
    ),
    ("RGB (no matrix)", MatrixCoefficients::Identity),
];
const RANGES: [(&str, Range); 2] = [
    ("Limited (video, 16–235)", Range::Limited),
    ("Full (data, 0–255)", Range::Full),
];
const ALPHAS: [(&str, AlphaChoice); 4] = [
    ("Ignore", AlphaChoice::Ignore),
    ("Straight", AlphaChoice::Straight),
    ("Premultiplied", AlphaChoice::Premultiplied),
    (
        "Premultiplied (Matted With Color)",
        AlphaChoice::PremultipliedWithColor,
    ),
];

const GAIN_MAPS: [(&str, Rendition); 2] = [
    ("HDR Picture", Rendition::Hdr),
    ("SDR Base Only", Rendition::SdrBase),
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
        Origin::PowermoveOne { file: Some(file) } => {
            format!("As Powermove 1 showed it; the file says {file}.")
        }
        Origin::PowermoveOne { file: None } => "As Powermove 1 showed it.".into(),
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
    // A v1 project's asset reads as Powermove 1 showed it until a value is
    // chosen here.
    let first = match read {
        Some((_, Origin::PowermoveOne { .. })) => "As in Powermove 1",
        _ => "Automatic",
    };
    // The pop-up names the value short ("Automatic (Limited)"); the line
    // under it says it whole.
    let mut values = vec![automatic.as_ref().map_or_else(
        || first.to_owned(),
        |automatic| {
            let short = automatic
                .rsplit_once(" (")
                .filter(|_| automatic.ends_with(')'))
                .map_or(automatic.as_str(), |(name, _)| name);
            format!("{first} ({short})")
        },
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
        AlphaMode::Premultiplied => AlphaChoice::Premultiplied,
        AlphaMode::PremultipliedWithColor { .. } => AlphaChoice::PremultipliedWithColor,
    }
}

/// The matte as the document stores it.
fn matte_hex(matte: [f64; 3]) -> String {
    let c = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02X}{:02X}{:02X}", c(matte[0]), c(matte[1]), c(matte[2]))
}

/// Tone Mapping's choices (an HDR source in an SDR project): the Color
/// Space Transform's labels, stored by pm-media's names.
fn tones() -> Vec<(&'static str, &'static str)> {
    pm_color::cst::TONE_MAPPING
        .into_iter()
        .zip(pm_media::interpret::TONE_MAPPING)
        .collect()
}

/// The automatic value of a read field and where it came from.
fn read<T>(field: &Read<T>) -> (&T, &Origin) {
    (&field.automatic, &field.origin)
}

/// The sheet for `name`. `video` has a matrix and a range; a still does not.
/// `reading` is the file as the engine reads it, when the file could be read.
/// `gain_map`: the still carries a gain map (its picture is HDR unless its
/// SDR base is chosen).
pub fn surface(
    id: DialogId,
    owner: WindowId,
    name: &str,
    video: bool,
    chosen: &Interpretation,
    reading: Option<&VideoReading>,
    gain_map: bool,
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
    // An HDR source drawn in an SDR project: BT.2390 per channel unless
    // chosen here.
    let choices = tones();
    let automatic = Origin::Default {
        rule: "BT.2390 per channel, to SDR white".into(),
    };
    let names = pm_media::interpret::TONE_MAPPING;
    let mut tone = popup(
        "toneMapping",
        "Tone Mapping",
        &choices,
        chosen.tone_mapping.and_then(|i| names.get(i as usize)),
        reading
            .and_then(|r| r.tone_map)
            .map(|_| (&names[1], &automatic)),
    );
    tone.enabled = video && reading.is_some_and(|r| r.tone_map.is_some());
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
        tone,
        popup(
            "alpha",
            "Alpha",
            &ALPHAS,
            chosen.alpha.as_ref(),
            alpha.as_ref().map(|(value, origin)| (value, origin)),
        ),
        Field {
            description: Some("The color a matted file’s edges were blended with.".into()),
            ..Field::new(
                "alphaMatte",
                "Matte Color",
                NativePiece::ColourWell,
                vec![matte_hex(chosen.alpha_matte)],
            )
        },
        Field {
            enabled: gain_map,
            description: Some(if gain_map {
                "Automatic is the HDR picture, at the headroom its gain map describes.".into()
            } else {
                "Only a still with a gain map has an SDR base to read.".into()
            }),
            ..popup(
                "gainMap",
                "Gain Map",
                &GAIN_MAPS,
                chosen.gain_map.as_ref(),
                None,
            )
        },
    ];
    let mut surface = Surface::sheet(id, owner, "Interpret Footage", fields, "Apply");
    surface.line = Some(name.to_owned());
    surface.groups = vec![
        FieldGroup {
            title: "Color",
            fields: vec!["primaries", "transfer", "matrix", "range", "toneMapping"],
        },
        FieldGroup {
            title: "Alpha",
            fields: vec!["alpha", "alphaMatte"],
        },
        FieldGroup {
            title: "HDR",
            fields: vec!["gainMap"],
        },
    ];
    surface
}

/// What the pop-ups say, as indexes: 0 is Automatic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chosen {
    pub primaries: usize,
    pub transfer: usize,
    pub matrix: usize,
    pub range: usize,
    pub tone_mapping: usize,
    pub alpha: usize,
    pub gain_map: usize,
    /// The matte well's colour, once the person has changed it.
    pub matte: Option<String>,
}

impl Chosen {
    pub fn read(popup: impl Fn(&str) -> Option<usize>, matte: Option<String>) -> Self {
        let at = |id: &str| popup(id).unwrap_or(0);
        Self {
            primaries: at("primaries"),
            transfer: at("transfer"),
            matrix: at("matrix"),
            range: at("range"),
            tone_mapping: at("toneMapping"),
            alpha: at("alpha"),
            gain_map: at("gainMap"),
            matte,
        }
    }

    /// What the sheet showed when it opened.
    pub fn shown(surface: &Surface) -> Self {
        Self::read(
            |id| {
                surface
                    .fields
                    .iter()
                    .find(|f| f.id == id)
                    .map(|f| f.selected)
            },
            None,
        )
    }

    fn matted(&self) -> bool {
        self.alpha
            .checked_sub(1)
            .and_then(|i| ALPHAS.get(i))
            .is_some_and(|(_, alpha)| *alpha == AlphaChoice::PremultipliedWithColor)
    }

    /// Writes what the person changed since the sheet `opened` into an
    /// asset's `interpret` member. A pop-up left alone leaves its member as
    /// stored, whatever that is; Automatic removes the override; every other
    /// member stays as it was. Returns whether anything was written.
    pub fn write(&self, opened: &Chosen, interpret: &mut Map) -> bool {
        fn set<T: serde::Serialize>(
            interpret: &mut Map,
            key: &str,
            choices: &[(&str, T)],
            (index, opened): (usize, usize),
        ) -> bool {
            if index == opened {
                return false;
            }
            match index.checked_sub(1).and_then(|i| choices.get(i)) {
                Some((_, value)) => {
                    interpret.insert(key, Value::from(stored(value).as_str()));
                }
                None => {
                    interpret.remove(key);
                }
            }
            true
        }
        let mut changed = set(
            interpret,
            "primaries",
            &PRIMARIES,
            (self.primaries, opened.primaries),
        );
        changed |= set(
            interpret,
            "transfer",
            &TRANSFERS,
            (self.transfer, opened.transfer),
        );
        changed |= set(interpret, "matrix", &MATRICES, (self.matrix, opened.matrix));
        changed |= set(interpret, "range", &RANGES, (self.range, opened.range));
        changed |= set(
            interpret,
            "gainMap",
            &GAIN_MAPS,
            (self.gain_map, opened.gain_map),
        );
        changed |= set(
            interpret,
            "toneMapping",
            &tones(),
            (self.tone_mapping, opened.tone_mapping),
        );
        let alpha = set(interpret, "alpha", &ALPHAS, (self.alpha, opened.alpha));
        changed |= alpha;
        // The matte belongs to the matted choice: it leaves with it, and a
        // colour picked under any other choice is not stored.
        if alpha && !self.matted() {
            interpret.remove("alphaMatte");
        }
        if let Some(matte) = self.matte.as_deref().filter(|_| self.matted()) {
            interpret.insert("alphaMatte", Value::from(matte));
            changed = true;
        }
        changed
    }
}

/// Worker: the file as the engine reads it under `chosen` (`sequence`: the
/// asset is an image sequence).
fn video_reading(path: &Path, chosen: &Interpretation, sequence: bool) -> Option<VideoReading> {
    let probed = pm_media::probe::probe(path).ok()?;
    let video = probed.probe.video.as_ref()?;
    let mut reading = pm_media::interpret::resolve_video(video, chosen);
    if chosen.electron_color {
        as_in_powermove_1(&mut reading, video, sequence);
    }
    Some(reading)
}

/// What a v1 project's asset is read with when nothing is chosen here: the
/// reading Powermove 1 showed (the renderer's own rule), naming the file's.
fn as_in_powermove_1(
    reading: &mut VideoReading,
    video: &pm_media::probe::VideoProbe,
    sequence: bool,
) {
    let v1 =
        pm_media::probe::color::electron_frame_reading(&video.color, &video.codec.codec, sequence);
    fn adopt<T: PartialEq + Copy + std::fmt::Debug>(
        field: &mut Read<T>,
        v1: T,
        choices: &[(&str, T)],
    ) {
        let file = field.automatic;
        field.automatic = v1;
        if !field.origin.is_override() {
            field.value = v1;
            field.origin = Origin::PowermoveOne {
                file: (file != v1).then(|| label(choices, &file)),
            };
        }
    }
    adopt(&mut reading.primaries, v1.primaries, &PRIMARIES);
    adopt(&mut reading.transfer, v1.transfer, &TRANSFERS);
    adopt(&mut reading.matrix, v1.matrix, &MATRICES);
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
                let sequence = pm_render::footage::is_sequence(record);
                (asset, kind == "video", name, chosen, sequence)
            })
        });
        let Some((asset, video, name, chosen, sequence)) = record else {
            self.message = SELECT.into();
            cx.notify();
            return;
        };
        let path = self.media.originals.get(&asset).cloned();
        cx.spawn(async move |this, cx| {
            // The file is read off the main thread; an offline or still file
            // opens the sheet without what "Automatic" resolves to.
            let (reading, gain_map) = cx
                .background_spawn({
                    let chosen = chosen.clone();
                    async move {
                        let reading = path
                            .clone()
                            .filter(|_| video)
                            .and_then(|p| video_reading(&p, &chosen, sequence));
                        let gain_map = path
                            .filter(|_| !video && !sequence)
                            .is_some_and(|p| pm_media::image::has_gain_map(&p).unwrap_or(false));
                        (reading, gain_map)
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
                gain_map,
            );
            let opened = Chosen::shown(&surface);
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
                        let chosen =
                            Chosen::read(|id| open.popup(id), open.changed_colour("alphaMatte"));
                        // Nothing changed: nothing is written, and Undo gains
                        // no step.
                        if chosen.write(&opened, &mut Map::new()) {
                            let _ = this.update(cx, |v, cx| {
                                v.change_interpretation(
                                    |p, _| {
                                        crate::colour::edit_interpretation(p, &layer, |interpret| {
                                            chosen.write(&opened, interpret);
                                        })
                                    },
                                    cx,
                                )
                            });
                        }
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

    fn parsed(interpret: &Map) -> Interpretation {
        let json: serde_json::Value = serde_json::from_str(&pm_model::json::to_canonical_string(
            &Value::Object(interpret.clone()),
        ))
        .unwrap();
        let (parsed, issues) = Interpretation::from_json(&json);
        assert!(issues.is_empty(), "{issues:?}");
        parsed
    }

    fn sheet(chosen: &Interpretation) -> Surface {
        let (id, owner) = ids();
        surface(id, owner, "Clip.mov", true, chosen, None, false)
    }

    /// Every stored name is one the engine parses back to the same value.
    #[test]
    fn every_choice_is_a_value_the_engine_reads() {
        for step in 0..PRIMARIES.len().max(TRANSFERS.len()) {
            let at = |count: usize| 1 + step % count;
            let chosen = Chosen {
                primaries: at(PRIMARIES.len()),
                transfer: at(TRANSFERS.len()),
                matrix: at(MATRICES.len()),
                range: at(RANGES.len()),
                tone_mapping: at(tones().len()),
                alpha: at(ALPHAS.len()),
                gain_map: at(GAIN_MAPS.len()),
                matte: None,
            };
            let mut interpret = Map::new();
            assert!(chosen.write(&Chosen::default(), &mut interpret));
            let parsed = parsed(&interpret);
            assert_eq!(parsed.tone_mapping, Some(chosen.tone_mapping as u32 - 1));
            assert_eq!(parsed.primaries, Some(PRIMARIES[chosen.primaries - 1].1));
            assert_eq!(parsed.transfer, Some(TRANSFERS[chosen.transfer - 1].1));
            assert_eq!(parsed.matrix, Some(MATRICES[chosen.matrix - 1].1));
            assert_eq!(parsed.range, Some(RANGES[chosen.range - 1].1));
            assert_eq!(parsed.alpha, Some(ALPHAS[chosen.alpha - 1].1));
            assert_eq!(parsed.gain_map, Some(GAIN_MAPS[chosen.gain_map - 1].1));
        }
    }

    /// Every value the engine reads has a choice: an override the sheet
    /// could not show would look like Automatic.
    #[test]
    fn every_value_the_engine_reads_is_a_choice() {
        fn offered<T: PartialEq + serde::de::DeserializeOwned>(
            choices: &[(&str, T)],
            names: &[&str],
        ) {
            for name in names {
                let value: T = serde_json::from_value(serde_json::json!(name)).expect(name);
                assert!(choices.iter().any(|(_, v)| *v == value), "{name}");
            }
            assert_eq!(choices.len(), names.len());
        }
        offered(
            &PRIMARIES,
            &[
                "bt709",
                "bt601_625",
                "bt601_525",
                "bt2020",
                "displayP3",
                "bt470M",
                "genericFilm",
                "dciP3",
                "ebu3213",
                "sGamut3",
                "sGamut3Cine",
                "vGamut",
                "awgGen3",
                "awgGen4",
            ],
        );
        offered(
            &TRANSFERS,
            &[
                "displayE",
                "bt1886",
                "sceneReferredBt709",
                "linear",
                "gamma22",
                "log100",
                "log316",
                "pq",
                "hlg",
                "appleLog",
                "sLog3",
                "vLog",
                "logC3",
                "logC4",
            ],
        );
        offered(
            &MATRICES,
            &["identity", "bt709", "bt601", "bt2020Ncl", "bt2020Cl"],
        );
        offered(&RANGES, &["limited", "full"]);
        offered(
            &ALPHAS,
            &[
                "ignore",
                "straight",
                "premultiplied",
                "premultipliedWithColor",
            ],
        );
        offered(&GAIN_MAPS, &["hdr", "sdrBase"]);
        // A variant added to the engine fails to compile here until it is
        // offered above.
        let _ = |p: Primaries, t: Transfer, m: MatrixCoefficients, a: AlphaChoice, g: Rendition| {
            match g {
                Rendition::Hdr | Rendition::SdrBase => {}
            }
            match p {
                Primaries::Bt709
                | Primaries::Bt601_625
                | Primaries::Bt601_525
                | Primaries::Bt2020
                | Primaries::DisplayP3
                | Primaries::Bt470M
                | Primaries::GenericFilm
                | Primaries::DciP3
                | Primaries::Ebu3213
                | Primaries::SGamut3
                | Primaries::SGamut3Cine
                | Primaries::VGamut
                | Primaries::AwgGen3
                | Primaries::AwgGen4
                | Primaries::Unspecified => {}
            }
            match t {
                Transfer::DisplayE
                | Transfer::Bt1886
                | Transfer::SceneReferredBt709
                | Transfer::Linear
                | Transfer::Gamma22
                | Transfer::Log100
                | Transfer::Log316
                | Transfer::Pq
                | Transfer::Hlg
                | Transfer::AppleLog
                | Transfer::SLog3
                | Transfer::VLog
                | Transfer::LogC3
                | Transfer::LogC4 => {}
            }
            match m {
                MatrixCoefficients::Identity
                | MatrixCoefficients::Bt709
                | MatrixCoefficients::Bt601
                | MatrixCoefficients::Bt2020Ncl
                | MatrixCoefficients::Bt2020Cl => {}
            }
            match a {
                AlphaChoice::Ignore
                | AlphaChoice::Straight
                | AlphaChoice::Premultiplied
                | AlphaChoice::PremultipliedWithColor => {}
            }
        };
    }

    /// Opening the sheet on any stored override and pressing Apply writes
    /// nothing: the override is shown as itself, never as Automatic, and a
    /// pop-up the person left alone leaves its member as stored.
    #[test]
    fn apply_with_nothing_changed_keeps_every_override() {
        for stored in [
            serde_json::json!({"alpha":"premultipliedWithColor","alphaMatte":"#FFFFFF"}),
            serde_json::json!({"primaries":"bt470M","transfer":"log100"}),
            serde_json::json!({"primaries":"genericFilm","transfer":"log316","matrix":"bt2020Cl"}),
            serde_json::json!({"primaries":"ebu3213","range":"full","cleanAperture":true}),
            // Members this build cannot read stay too.
            serde_json::json!({"transfer":"aLaterCurve","primaries":"bt2020","siting":"left"}),
        ] {
            let mut interpret = Map::new();
            for (key, value) in stored.as_object().unwrap() {
                let value = match value {
                    serde_json::Value::Bool(b) => Value::from(*b),
                    other => Value::from(other.as_str().unwrap()),
                };
                interpret.insert(key.as_str(), value);
            }
            let before = interpret.clone();
            let chosen = Interpretation::from_json(&stored).0;
            let surface = sheet(&chosen);
            let opened = Chosen::shown(&surface);
            // What the engine reads is on show, not Automatic.
            for (id, member) in [
                ("primaries", chosen.primaries.is_some()),
                ("transfer", chosen.transfer.is_some()),
                ("matrix", chosen.matrix.is_some()),
                ("range", chosen.range.is_some()),
                ("alpha", chosen.alpha.is_some()),
            ] {
                let field = surface.fields.iter().find(|f| f.id == id).unwrap();
                assert_eq!(field.selected != 0, member, "{id} of {stored}");
            }
            assert!(!opened.clone().write(&opened, &mut interpret), "{stored}");
            assert_eq!(interpret, before, "{stored}");
            // Changing one field leaves the others as stored.
            let other = Chosen {
                range: if opened.range == 1 { 2 } else { 1 },
                ..opened.clone()
            };
            assert!(other.write(&opened, &mut interpret));
            for (key, value) in before.iter() {
                if key.to_string_lossy() != "range" {
                    assert_eq!(interpret.get_key(key), Some(value), "{stored}");
                }
            }
        }
    }

    #[test]
    fn a_matte_is_stored_with_its_choice_and_leaves_with_it() {
        let matted = ALPHAS.len();
        let mut interpret = Map::new();
        // A colour picked under another choice is not stored.
        let straight = Chosen {
            alpha: 2,
            matte: Some("#FF0000".into()),
            ..Chosen::default()
        };
        assert!(straight.write(&Chosen::default(), &mut interpret));
        assert!(interpret.get("alphaMatte").is_none());
        let white = Chosen {
            alpha: matted,
            matte: Some("#FFFFFF".into()),
            ..Chosen::default()
        };
        assert!(white.write(&straight, &mut interpret));
        let read = parsed(&interpret);
        assert_eq!(read.alpha, Some(AlphaChoice::PremultipliedWithColor));
        assert_eq!(read.alpha_matte, [1.0, 1.0, 1.0]);
        // The sheet shows the stored matte in its well.
        let surface = sheet(&read);
        let well = surface
            .fields
            .iter()
            .find(|f| f.id == "alphaMatte")
            .unwrap();
        assert_eq!(well.values, ["#FFFFFF"]);
        assert_eq!(Chosen::shown(&surface).alpha, matted);
        // Back to Automatic: the matte goes with the choice.
        assert!(Chosen::default().write(&Chosen::shown(&surface), &mut interpret));
        assert!(interpret.is_empty(), "{interpret:?}");
    }

    #[test]
    fn automatic_removes_an_override_and_keeps_the_other_members() {
        let mut interpret = Map::new();
        interpret.insert("range", Value::from("full"));
        interpret.insert("cleanAperture", Value::from(true));
        // The sheet opened showing Full; the person chose Automatic, and a
        // matrix.
        let opened = Chosen {
            range: 2,
            ..Chosen::default()
        };
        Chosen {
            matrix: 2,
            ..Chosen::default()
        }
        .write(&opened, &mut interpret);
        assert!(interpret.get("range").is_none(), "Automatic removes it");
        assert_eq!(interpret.get("matrix"), Some(&Value::from("bt601")));
        assert_eq!(interpret.get("cleanAperture"), Some(&Value::from(true)));
    }

    /// Tone Mapping is written as pm-media reads it, and Automatic (BT.2390
    /// per channel) removes it; it shows disabled with no HDR reading.
    #[test]
    fn tone_mapping_is_chosen_per_clip() {
        let opened = Chosen::default();
        let mut interpret = Map::new();
        let chosen = Chosen {
            tone_mapping: 1 + 5,
            ..Default::default()
        };
        assert!(chosen.write(&opened, &mut interpret));
        assert_eq!(parsed(&interpret).tone_mapping, Some(5));
        assert!(Chosen::default().write(&chosen, &mut interpret));
        assert_eq!(parsed(&interpret).tone_mapping, None);
        let field = sheet(&Interpretation::default())
            .fields
            .into_iter()
            .find(|f| f.id == "toneMapping")
            .unwrap();
        assert!(!field.enabled);
        assert_eq!(field.values[1], "None");
        assert_eq!(field.values[6], "BT.2446 Method C");
    }

    #[test]
    fn the_sheet_shows_the_override_and_what_the_file_says() {
        let chosen = Interpretation {
            range: Some(Range::Full),
            ..Interpretation::default()
        };
        let (id, owner) = ids();
        let sheet = surface(
            id,
            owner,
            "Screen Recording.mov",
            true,
            &chosen,
            None,
            false,
        );
        assert_eq!(sheet.line.as_deref(), Some("Screen Recording.mov"));
        let field = |id: &str| sheet.fields.iter().find(|f| f.id == id).unwrap();
        assert_eq!(field("range").selected, 2, "Full is chosen");
        assert_eq!(field("primaries").selected, 0, "Automatic");
        assert_eq!(field("primaries").values[0], "Automatic");
        // The pop-ups read back as what they show.
        let read = Chosen::read(|id| Some(field(id).selected), None);
        assert_eq!(read.range, 2);
        assert_eq!(read, Chosen::shown(&sheet));
        // A still has no matrix or range to set.
        let (id, owner) = ids();
        let still = surface(
            id,
            owner,
            "Plate.png",
            false,
            &Interpretation::default(),
            None,
            false,
        );
        let field = |id: &str| still.fields.iter().find(|f| f.id == id).unwrap();
        assert!(!field("matrix").enabled && !field("range").enabled);
        assert!(field("alpha").enabled);
        assert!(
            !field("gainMap").enabled,
            "no gain map, no SDR base to read"
        );
    }

    /// A still with a gain map is read as its HDR picture unless its SDR
    /// base is chosen: the choice is stored as the engine reads it, and
    /// shown as itself when the sheet opens again.
    #[test]
    fn a_still_with_a_gain_map_can_be_read_for_its_sdr_base() {
        let (id, owner) = ids();
        let open = |chosen: &Interpretation| {
            surface(
                id.clone(),
                owner.clone(),
                "Sunset.heic",
                false,
                chosen,
                None,
                true,
            )
        };
        let sheet = open(&Interpretation::default());
        let field = sheet.fields.iter().find(|f| f.id == "gainMap").unwrap();
        assert!(field.enabled);
        assert_eq!(
            field.values,
            ["Automatic", "HDR Picture", "SDR Base Only"].map(String::from)
        );
        assert_eq!(field.selected, 0);
        let opened = Chosen::shown(&sheet);
        let mut interpret = Map::new();
        let base = Chosen {
            gain_map: 2,
            ..opened.clone()
        };
        assert!(base.write(&opened, &mut interpret));
        let parsed = parsed(&interpret);
        assert_eq!(parsed.gain_map, Some(Rendition::SdrBase));
        let again = open(&parsed);
        let field = again.fields.iter().find(|f| f.id == "gainMap").unwrap();
        assert_eq!(field.selected, 2);
        // Automatic again removes the member.
        let automatic = Chosen {
            gain_map: 0,
            ..Chosen::shown(&again)
        };
        assert!(automatic.write(&Chosen::shown(&again), &mut interpret));
        assert_eq!(interpret.get("gainMap"), None);
    }

    /// A v1 project's asset (`videoColor`) is read as Powermove 1 showed it:
    /// the sheet says so, names the file's own reading, and a value chosen
    /// here still wins. An untagged HD FFV1 clip: Rec. 601 in v1, Rec. 709
    /// natively.
    #[test]
    fn a_v1_asset_shows_the_reading_as_in_powermove_1() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("untagged.mkv");
        let ffmpeg = std::path::PathBuf::from(
            std::env::var_os("PM_FFMPEG_DIR").expect("source the native env before this test"),
        )
        .join("bin/ffmpeg");
        let status = std::process::Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=30",
            ])
            .args(["-frames:v", "1", "-c:v", "ffv1", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg wrote the clip");
        let (id, owner) = ids();
        let shown = |chosen: &Interpretation| {
            let reading = video_reading(&clip, chosen, false).unwrap();
            let surface = surface(
                id.clone(),
                owner.clone(),
                "Clip.mkv",
                true,
                chosen,
                Some(&reading),
                false,
            );
            let field = |name: &str| {
                surface
                    .fields
                    .iter()
                    .find(|f| f.id == name)
                    .unwrap()
                    .clone()
            };
            (field("matrix"), field("primaries"))
        };
        let native = Interpretation::default();
        let (matrix, _) = shown(&native);
        assert_eq!(matrix.values[0], "Automatic (Rec. 709)");
        let v1 = Interpretation {
            electron_color: true,
            ..Default::default()
        };
        let (matrix, primaries) = shown(&v1);
        assert_eq!(matrix.values[0], "As in Powermove 1 (Rec. 601)");
        assert_eq!(matrix.selected, 0);
        assert_eq!(
            matrix.description.as_deref(),
            Some("As Powermove 1 showed it; the file says Rec. 709.")
        );
        assert_eq!(
            primaries.description.as_deref(),
            Some("As Powermove 1 showed it."),
            "{primaries:?}"
        );
        // Chosen here: the choice is in force, and Automatic is the v1 reading.
        let chosen = Interpretation {
            matrix: Some(MatrixCoefficients::Bt709),
            ..v1
        };
        let (matrix, _) = shown(&chosen);
        assert_eq!(
            matrix.selected,
            1 + MATRICES
                .iter()
                .position(|(_, m)| *m == MatrixCoefficients::Bt709)
                .unwrap()
        );
        assert_eq!(
            matrix.description.as_deref(),
            Some("Set here. Automatic is Rec. 601.")
        );
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
        // A long value is named short in the pop-up, whole under it.
        let range = popup(
            "range",
            "Range",
            &RANGES,
            None,
            Some((&Range::Limited, &Origin::Override)),
        );
        assert_eq!(range.values[0], "Automatic (Limited)");
        assert_eq!(
            range.description.as_deref(),
            Some("Set here. Automatic is Limited (video, 16–235).")
        );
        assert_eq!(
            field.description.as_deref(),
            Some("Set here. Automatic is Rec. 601.")
        );
    }
}
