//! S087–S089/S093. v1.2.0 captions/CaptionsPanel.svelte and captions/install.ts.
//! Immutable publication projections and real pm-captions sidecar adapter.
use super::DocumentStamp;
use pm_captions::{CaptionCue, CaptionFormat, ParsedCaptions, WriteOptions};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct Layer {
    pub id: String,
    pub from: f64,
    pub locked: bool,
    pub cues: Vec<CaptionCue>,
}
pub fn choose_layer<'a>(
    layers: &'a [Layer],
    selected: &[String],
    chosen: Option<&str>,
) -> Option<&'a Layer> {
    layers
        .iter()
        .find(|l| selected.contains(&l.id))
        .or_else(|| layers.iter().find(|l| Some(l.id.as_str()) == chosen))
        .or_else(|| layers.first())
}
pub fn shown<'a>(layer: &'a Layer, query: &str) -> Vec<(usize, &'a CaptionCue)> {
    let query = query.trim().to_lowercase();
    layer
        .cues
        .iter()
        .enumerate()
        .filter(|(_, c)| query.is_empty() || c.text.to_lowercase().contains(&query))
        .collect()
}
#[derive(Debug, Default)]
pub struct Selection {
    pub layer_id: Option<String>,
    pub cues: BTreeSet<String>,
}
#[derive(Debug, Clone, Copy)]
pub enum SelectionMode {
    Replace,
    Extend,
    Toggle,
}
impl Selection {
    /// Compatibility helper for command-click; native Shift uses Extend below.
    pub fn choose(&mut self, layer: &Layer, cue_id: &str, extend: bool) -> Option<f64> {
        let cue = layer.cues.iter().find(|c| c.id == cue_id)?;
        if !extend || self.layer_id.as_ref() != Some(&layer.id) {
            self.cues.clear();
        }
        if !self.cues.remove(cue_id) {
            self.cues.insert(cue_id.into());
        }
        self.layer_id = Some(layer.id.clone());
        Some(layer.from + cue.start)
    }
    /// S087: Shift extends without removing an already selected cue; Command
    /// toggles. No unspecified range anchor/geometry is invented.
    pub fn select(&mut self, layer: &Layer, id: &str, mode: SelectionMode) -> Option<f64> {
        let cue = layer.cues.iter().find(|c| c.id == id)?;
        if self.layer_id.as_ref() != Some(&layer.id) || matches!(mode, SelectionMode::Replace) {
            self.cues.clear();
        }
        match mode {
            SelectionMode::Toggle => {
                if !self.cues.remove(id) {
                    self.cues.insert(id.into());
                }
            }
            _ => {
                self.cues.insert(id.into());
            }
        }
        self.layer_id = Some(layer.id.clone());
        Some(layer.from + cue.start)
    }
    pub fn prune(&mut self, layers: &[Layer]) {
        let Some(layer) = layers
            .iter()
            .find(|l| Some(&l.id) == self.layer_id.as_ref())
        else {
            self.layer_id = None;
            self.cues.clear();
            return;
        };
        self.cues
            .retain(|id| layer.cues.iter().any(|c| &c.id == id));
    }
}
pub fn follow(
    layer: &Layer,
    query: &str,
    composition_time: f64,
    playing: bool,
    editing: bool,
    list_focused: bool,
) -> Option<usize> {
    if !playing || editing || list_focused || !query.is_empty() {
        return None;
    }
    let index = pm_captions::cue_index_at(&layer.cues, composition_time - layer.from)?;
    shown(layer, query).iter().position(|(i, _)| *i == index)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub stamp: DocumentStamp,
    pub wire: Value,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditProblem {
    Locked,
    MissingCue,
    Stale,
    Unchanged,
    InvalidTime,
    NotSelected,
    InvalidRange,
}
#[derive(Debug)]
pub struct CueDraft {
    pub stamp: DocumentStamp,
    pub layer_id: String,
    pub cue_id: String,
    pub text: String,
    original: String,
}
impl CueDraft {
    pub fn begin(stamp: DocumentStamp, layer: &Layer, cue_id: &str) -> Result<Self, EditProblem> {
        if layer.locked {
            return Err(EditProblem::Locked);
        }
        let cue = layer
            .cues
            .iter()
            .find(|c| c.id == cue_id)
            .ok_or(EditProblem::MissingCue)?;
        Ok(Self {
            stamp,
            layer_id: layer.id.clone(),
            cue_id: cue_id.into(),
            text: cue.text.clone(),
            original: cue.text.clone(),
        })
    }
    /// Failure retains this draft. Caller closes only after kernel acceptance;
    /// a transport ACK with Reply::Throw is never an applied edit.
    pub fn command(&self, current: &DocumentStamp, layer: &Layer) -> Result<Command, EditProblem> {
        if current != &self.stamp || self.layer_id != layer.id {
            return Err(EditProblem::Stale);
        }
        if layer.locked {
            return Err(EditProblem::Locked);
        }
        let cue = layer
            .cues
            .iter()
            .find(|c| c.id == self.cue_id)
            .ok_or(EditProblem::MissingCue)?;
        if cue.text != self.original {
            return Err(EditProblem::Stale);
        }
        if self.text == self.original {
            return Err(EditProblem::Unchanged);
        }
        Ok(Command {
            stamp: self.stamp.clone(),
            wire: json!({"type":"edit_captions","target":self.layer_id,"op":"update","cues":[{"id":self.cue_id,"text":self.text}]}),
        })
    }
}
pub fn delete_selected(
    stamp: DocumentStamp,
    layer: &Layer,
    selection: &Selection,
    selected_layers: &[String],
) -> Result<Command, EditProblem> {
    if layer.locked {
        return Err(EditProblem::Locked);
    }
    if selection.layer_id.as_ref() != Some(&layer.id)
        || selection.cues.is_empty()
        || selected_layers != [layer.id.clone()]
    {
        return Err(EditProblem::NotSelected);
    }
    if selection
        .cues
        .iter()
        .any(|id| !layer.cues.iter().any(|c| &c.id == id))
    {
        return Err(EditProblem::MissingCue);
    }
    Ok(Command {
        stamp,
        wire: json!({"type":"edit_captions","target":layer.id,"op":"delete","ids":selection.cues}),
    })
}
pub fn split(
    stamp: DocumentStamp,
    layer: &Layer,
    selection: &Selection,
    composition_time: f64,
) -> Result<Command, EditProblem> {
    if layer.locked {
        return Err(EditProblem::Locked);
    }
    if !composition_time.is_finite() {
        return Err(EditProblem::InvalidTime);
    }
    let cue = pm_captions::cue_index_at(&layer.cues, composition_time - layer.from)
        .and_then(|i| layer.cues.get(i))
        .ok_or(EditProblem::MissingCue)?;
    if selection.layer_id.as_ref() != Some(&layer.id) || !selection.cues.contains(&cue.id) {
        return Err(EditProblem::NotSelected);
    }
    Ok(Command {
        stamp,
        wire: json!({"type":"edit_captions","target":layer.id,"op":"split","id":cue.id,"at":composition_time}),
    })
}

#[derive(Debug)]
pub struct Import {
    pub stamp: DocumentStamp,
    /// Original text remains retained even when parsing skips blocks.
    pub original: String,
    pub name: String,
    pub parsed: ParsedCaptions,
}
impl Import {
    /// File decoding is host-owned (UTF-8 with Windows-1252 fallback in tag).
    pub fn parse(stamp: DocumentStamp, text: String, name: String) -> Self {
        let parsed = pm_captions::parse_captions(&text, None, &name);
        Self {
            stamp,
            original: text,
            name,
            parsed,
        }
    }
    pub fn command(&self, current: &DocumentStamp, at: f64) -> Result<Command, EditProblem> {
        if current != &self.stamp {
            return Err(EditProblem::Stale);
        }
        if !at.is_finite() || at < 0. {
            return Err(EditProblem::InvalidTime);
        }
        if self.parsed.cues.is_empty() {
            return Err(EditProblem::MissingCue);
        }
        Ok(Command {
            stamp: self.stamp.clone(),
            wire: json!({"type":"add_captions","text":self.original,"format":self.parsed.format,"offset":at,"from":at}),
        })
    }
}
pub fn export(
    layer: &Layer,
    format: CaptionFormat,
    range: Option<[f64; 2]>,
) -> Result<String, EditProblem> {
    if !layer.from.is_finite() {
        return Err(EditProblem::InvalidTime);
    }
    if range.is_some_and(|[a, b]| !a.is_finite() || !b.is_finite() || a < 0. || b <= a) {
        return Err(EditProblem::InvalidRange);
    }
    let mut options = WriteOptions {
        offset: layer.from,
        ..WriteOptions::default()
    };
    if let Some([from, to]) = range {
        options.from = Some(from);
        options.to = Some(to);
        options.rebase = from;
    }
    Ok(pm_captions::format_captions(format, &layer.cues, &options))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stamp() -> DocumentStamp {
        DocumentStamp {
            project: "p".into(),
            session: 1,
            revision: 1,
        }
    }
    fn layer() -> Layer {
        Layer {
            id: "captions".into(),
            from: 10.,
            locked: false,
            cues: vec![
                CaptionCue {
                    id: "a".into(),
                    start: 0.,
                    end: 1.,
                    text: "First\ncaption".into(),
                    words: None,
                },
                CaptionCue {
                    id: "b".into(),
                    start: 2.,
                    end: 4.,
                    text: "Second".into(),
                    words: None,
                },
            ],
        }
    }
    #[test]
    fn layer_selection_precedes_remembered_then_first() {
        let a = layer();
        let mut b = layer();
        b.id = "other".into();
        let layers = [a, b];
        assert_eq!(
            choose_layer(&layers, &["other".into()], Some("captions"))
                .unwrap()
                .id,
            "other"
        );
        assert_eq!(
            choose_layer(&layers, &[], Some("other")).unwrap().id,
            "other"
        );
        assert_eq!(choose_layer(&layers, &[], None).unwrap().id, "captions");
    }
    #[test]
    fn search_keeps_source_indices_and_follow_suppresses_typing() {
        let l = layer();
        assert_eq!(shown(&l, " SECOND ")[0].0, 1);
        assert_eq!(follow(&l, "", 12., true, false, false), Some(1));
        assert_eq!(follow(&l, "first", 12., true, false, false), None);
        assert_eq!(follow(&l, "", 12., true, true, false), None);
        assert_eq!(follow(&l, "", 11., true, false, false), None);
    }
    #[test]
    fn modified_selection_toggles_and_seeks_in_composition_time() {
        let l = layer();
        let mut s = Selection::default();
        assert_eq!(s.choose(&l, "a", false), Some(10.));
        s.choose(&l, "b", true);
        assert_eq!(s.cues.len(), 2);
        s.choose(&l, "a", true);
        assert_eq!(s.cues, BTreeSet::from(["b".into()]));
        s.prune(&[]);
        assert!(s.cues.is_empty());
    }
    #[test]
    fn draft_stays_recoverable_on_lock_reopen_and_foreign_edit() {
        let mut l = layer();
        let mut d = CueDraft::begin(stamp(), &l, "a").unwrap();
        d.text = "local".into();
        let mut stale = stamp();
        stale.session = 2;
        assert_eq!(d.command(&stale, &l), Err(EditProblem::Stale));
        l.locked = true;
        assert_eq!(d.command(&stamp(), &l), Err(EditProblem::Locked));
        l.locked = false;
        l.cues[0].text = "foreign".into();
        assert_eq!(d.command(&stamp(), &l), Err(EditProblem::Stale));
        assert_eq!(d.text, "local");
    }
    #[test]
    fn edit_plan_is_exact_and_noop_does_not_mutate() {
        let l = layer();
        let mut d = CueDraft::begin(stamp(), &l, "a").unwrap();
        assert_eq!(d.command(&stamp(), &l), Err(EditProblem::Unchanged));
        d.text = "new\ntext".into();
        let command = d.command(&stamp(), &l).unwrap();
        assert_eq!(
            command.wire,
            json!({"type":"edit_captions","target":"captions","op":"update","cues":[{"id":"a","text":"new\ntext"}]})
        );
        assert_eq!(l.cues[0].text, "First\ncaption");
    }
    #[test]
    fn delete_requires_only_target_selected_and_split_uses_global_time() {
        let l = layer();
        let mut s = Selection::default();
        s.choose(&l, "b", false);
        assert_eq!(
            delete_selected(stamp(), &l, &s, &["captions".into(), "other".into()]),
            Err(EditProblem::NotSelected)
        );
        assert!(delete_selected(stamp(), &l, &s, &["captions".into()]).is_ok());
        assert_eq!(split(stamp(), &l, &s, 13.).unwrap().wire["at"], 13.);
        assert_eq!(split(stamp(), &l, &s, 10.5), Err(EditProblem::NotSelected));
    }
    #[test]
    fn real_srt_parse_retains_original_and_stamps_import() {
        let original = "1\n00:00:00,000 --> 00:00:01,000\nHello\n\nbroken block\n";
        let import = Import::parse(stamp(), original.into(), "demo.srt".into());
        assert_eq!(import.original, original);
        assert_eq!(import.parsed.cues.len(), 1);
        assert_eq!(import.parsed.skipped, 1);
        let command = import.command(&stamp(), 3.).unwrap();
        assert_eq!(command.wire["text"], original);
        assert_eq!(command.wire["offset"], 3.);
        let mut other = stamp();
        other.revision += 1;
        assert_eq!(import.command(&other, 0.), Err(EditProblem::Stale));
    }
    #[test]
    fn export_uses_real_codec_clips_and_rebases_work_area() {
        let l = layer();
        let srt = export(&l, CaptionFormat::Srt, Some([12.5, 13.5])).unwrap();
        assert!(srt.contains("00:00:00,000 --> 00:00:01,000"));
        assert!(srt.contains("Second"));
        assert!(!srt.contains("First"));
        let vtt = export(&l, CaptionFormat::Vtt, None).unwrap();
        assert!(vtt.starts_with("WEBVTT"));
        assert!(vtt.contains("00:00:10.000"));
        assert_eq!(
            export(&l, CaptionFormat::Srt, Some([2., 1.])),
            Err(EditProblem::InvalidRange)
        );
    }
    #[test]
    fn opus_shift_extends_and_focus_or_query_suppresses_playback_follow() {
        let l = layer();
        let mut s = Selection::default();
        s.select(&l, "a", SelectionMode::Replace);
        s.select(&l, "b", SelectionMode::Extend);
        s.select(&l, "b", SelectionMode::Extend);
        assert_eq!(s.cues.len(), 2);
        s.select(&l, "b", SelectionMode::Toggle);
        assert_eq!(s.cues.len(), 1);
        assert_eq!(follow(&l, "", 12., true, false, false), Some(1));
        assert_eq!(follow(&l, "", 12., true, false, true), None);
        assert_eq!(follow(&l, "second", 12., true, false, false), None);
    }
}
