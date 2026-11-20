//! Platform-independent search/navigation state for the two pooled child windows.
use crate::layout::Rect;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposerPicker {
    Model,
    Effort,
    Access,
}
impl ComposerPicker {
    pub fn width(self) -> f32 {
        match self {
            Self::Model => 260.,
            Self::Effort => 156.,
            Self::Access => 360.,
        }
    }
}

pub const COMPOSER_PICKER_MARGIN: f32 = 8.;

/// Inset the owner's content rect before sizing and placing the menu.
pub fn composer_picker_interior(owner: Rect) -> Rect {
    let dx = COMPOSER_PICKER_MARGIN.min(owner.w / 2.);
    let dy = COMPOSER_PICKER_MARGIN.min(owner.h / 2.);
    Rect {
        x: owner.x + dx,
        y: owner.y + dy,
        w: owner.w - 2. * dx,
        h: owner.h - 2. * dy,
    }
}

/// Cocoa screen points (y increases upward). Keep the checked title aligned
/// with the button when it fits. Otherwise the menu stays against the
/// button, opening upward and contained by the owner's content rect.
pub fn composer_picker_frame(
    owner: Rect,
    anchor: Rect,
    size: (f32, f32),
    checked_from_top: Option<f32>,
) -> Rect {
    let owner = composer_picker_interior(owner);
    let (w, h) = (size.0.min(owner.w), size.1.min(owner.h));
    let top = anchor.y + anchor.h;
    let aligned = checked_from_top.map(|row| anchor.y + anchor.h / 2. + row - h);
    let min_y = owner.y;
    let max_y = owner.y + owner.h - h;
    let y = aligned
        .filter(|y| (min_y..=max_y).contains(y))
        .unwrap_or(top)
        .clamp(min_y, max_y);
    Rect {
        x: anchor.x.clamp(owner.x, owner.x + owner.w - w),
        y,
        w,
        h,
    }
}

/// Model sections resize above the trigger, keeping the bottom edge fixed.
pub fn model_picker_frame(owner: Rect, anchor: Rect, size: (f32, f32)) -> Rect {
    let owner = composer_picker_interior(owner);
    let w = size.0.min(owner.w);
    let top = owner.y + owner.h;
    // Leave enough room for the menu chrome even if the trigger is near the top.
    let y = (anchor.y + anchor.h).clamp(owner.y, top - owner.h.min(64.));
    Rect {
        x: anchor.x.clamp(owner.x, owner.x + owner.w - w),
        y,
        w,
        h: size.1.min(top - y),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerRow {
    Choice(usize),
    Provider(usize),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
    pub label: String,
    /// Optional explanatory line below a searchable menu row.
    pub detail: String,
    /// The provider's default model, independent of the current choice.
    pub is_default: bool,
    pub command: String,
    /// The section it is listed under; empty for none.
    pub category: String,
    pub provider: Option<String>,
    /// `Some` keeps the check column; `Some(true)` shows the mark.
    pub checked: Option<bool>,
    /// A font family the row's title is drawn in (a font picker previews
    /// each face in its own row).
    pub font: Option<String>,
    /// Choosing it keeps the menu open (a provider row: its models follow
    /// in the same menu); the owner sends the menu's new rows.
    pub stays: bool,
    /// Read-only text below the choices, outside keyboard and pointer selection.
    pub informational: bool,
    /// Layer and effect rows require a name search unless recently run.
    pub search_only: bool,
    pub aliases: Vec<String>,
}
impl Choice {
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            ..Default::default()
        }
    }
    pub fn with_after_effects_terms(mut self) -> Self {
        let command = crate::menu::split_palette_command(&self.command).0;
        self.aliases = AFTER_EFFECTS_TERMS
            .iter()
            .find(|(id, _)| *id == command)
            .map(|(_, terms)| terms.iter().map(|term| (*term).to_owned()).collect())
            .unwrap_or_default();
        self
    }
    pub fn display_label(&self, query: &str) -> String {
        let query = query.trim().to_lowercase();
        if !query.is_empty()
            && !self.label.to_lowercase().contains(&query)
            && let Some(term) = self
                .aliases
                .iter()
                .find(|term| term.to_lowercase().contains(&query))
        {
            return format!("{} — {term} in After Effects", self.label);
        }
        self.label.clone()
    }
    pub fn on_search(mut self) -> Self {
        self.search_only = true;
        self
    }
    pub fn detail(mut self, text: impl Into<String>) -> Self {
        self.detail = text.into();
        self
    }
    pub fn within(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }
    pub fn for_provider(mut self, id: &str) -> Self {
        self.provider = Some(id.to_owned());
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
    pub fn stays(mut self, on: bool) -> Self {
        self.stays = on;
        self
    }
    pub fn in_font(mut self, family: impl Into<String>) -> Self {
        self.font = Some(family.into());
        self
    }
}
/// What a menu's rows are doing: nothing is lit when it opens. The keyboard
/// lights a row (an arrow, or the best match of a typed query); the pointer
/// lights the row under it and takes its light away when it leaves.
#[derive(Clone, Debug, Default)]
pub struct Picker {
    pub choices: Vec<Choice>,
    pub recents: Vec<String>,
    pub query: String,
    /// The keyboard's row, a place in `rows`.
    pub keyed: Option<usize>,
    /// The row under the pointer, a place in `rows`.
    pub hover: Option<usize>,
    pub open: bool,
    /// The most rows listed at once (a font picker over every installed
    /// family); the search narrows the rest. Keys and the pointer reach
    /// only listed rows.
    pub page: Option<usize>,
    pub provider_choices: BTreeMap<String, bool>,
}
/// Match quality of `query` against `text` (lower is better), `None` if absent.
/// Label prefix, then a word start, then any substring, then an in-order
/// subsequence (`gtst` → Go to Start). An empty query matches everything equally.
pub fn rank(text: &str, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let t = text.to_lowercase();
    let q = query.to_lowercase();
    if t.starts_with(&q) {
        return Some(0);
    }
    if let Some(i) = t.find(&q) {
        let word = t[..i]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_alphanumeric());
        return Some(if word { 1 } else { 2 });
    }
    let mut chars = t.chars();
    q.chars()
        .filter(|c| !c.is_whitespace())
        .all(|qc| chars.any(|tc| tc == qc))
        .then_some(3)
}
impl Picker {
    pub fn open_providers(&mut self, saved: BTreeMap<String, bool>) {
        self.provider_choices = saved;
        self.expand_current_provider();
    }

    pub fn replace_choices(&mut self, choices: Vec<Choice>) {
        let previous = self.current_provider().map(str::to_owned);
        self.choices = choices;
        if self.current_provider() != previous.as_deref() {
            self.expand_current_provider();
        }
    }

    fn current_provider(&self) -> Option<&str> {
        self.choices.iter().find_map(|choice| {
            if choice.checked == Some(true) {
                choice.provider.as_deref()
            } else {
                None
            }
        })
    }

    fn expand_current_provider(&mut self) {
        if let Some(provider) = self.current_provider().map(str::to_owned) {
            self.provider_choices.insert(provider, true);
        }
    }

    pub fn has_provider_headers(&self) -> bool {
        let mut providers = self
            .choices
            .iter()
            .filter_map(|choice| choice.provider.as_ref());
        providers
            .next()
            .is_some_and(|first| providers.any(|provider| provider != first))
    }

    pub fn rows(&self) -> Vec<PickerRow> {
        let headers = self.has_provider_headers();
        let mut seen = HashSet::new();
        let mut rows = Vec::new();
        for index in self.filtered() {
            if headers && let Some(provider) = &self.choices[index].provider {
                if seen.insert(provider) {
                    rows.push(PickerRow::Provider(index));
                }
                if !self.provider_expanded(index) {
                    continue;
                }
            }
            rows.push(PickerRow::Choice(index));
        }
        rows
    }

    pub fn provider_expanded(&self, index: usize) -> bool {
        !self.query.trim().is_empty()
            || self.choices[index]
                .provider
                .as_ref()
                .and_then(|provider| self.provider_choices.get(provider))
                .copied()
                .unwrap_or(false)
    }

    pub fn provider_summary(&self, index: usize) -> String {
        let provider = &self.choices[index].provider;
        let members = || {
            self.choices
                .iter()
                .filter(|choice| &choice.provider == provider)
        };
        if !self.provider_expanded(index)
            && let Some(selected) = members().find(|choice| choice.checked == Some(true))
        {
            return selected.label.clone();
        }
        members().count().to_string()
    }

    pub fn toggle_provider(
        &mut self,
        index: usize,
        expanded: Option<bool>,
    ) -> Option<(String, bool)> {
        if !self.query.trim().is_empty() {
            return None;
        }
        let row = PickerRow::Provider(index);
        let place = self.rows().iter().position(|&item| item == row)?;
        let before = self.provider_expanded(index);
        let expanded = expanded.unwrap_or(!before);
        if expanded == before {
            return None;
        }
        let provider = self.choices[index].provider.clone()?;
        self.provider_choices.insert(provider.clone(), expanded);
        self.keyed = Some(place);
        self.hover = None;
        Some((provider, expanded))
    }

    pub fn provider_key(&mut self, key: &str) -> Option<(String, bool)> {
        let expanded = match key {
            "left" => Some(false),
            "right" => Some(true),
            "enter" | "space" => None,
            _ => return None,
        };
        let PickerRow::Provider(index) = self.lit_row()? else {
            return None;
        };
        self.toggle_provider(index, expanded)
    }

    pub fn lit_row(&self) -> Option<PickerRow> {
        self.lit().and_then(|place| self.rows().get(place).copied())
    }
    /// Matching choice indices. Sections stay whole: a section stands where
    /// its best match does, and its rows run best match first; ties keep
    /// their original order.
    pub fn filtered(&self) -> Vec<usize> {
        let mut rows = self.matches();
        if let Some(page) = self.page {
            rows.truncate(page);
        }
        rows
    }
    /// Matching rows past the page, not listed.
    pub fn unlisted(&self) -> usize {
        self.page
            .map_or(0, |page| self.matches().len().saturating_sub(page))
    }
    pub fn note(&self) -> Option<&str> {
        self.choices
            .iter()
            .find(|c| c.informational)
            .map(|c| c.label.as_str())
    }
    pub fn selected(&self) -> Option<usize> {
        self.rows().iter().position(
            |row| matches!(row, PickerRow::Choice(i) if self.choices[*i].checked == Some(true)),
        )
    }
    fn matches(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        let mut ranked: Vec<(u32, usize)> = self
            .choices
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.informational {
                    return None;
                }
                if c.search_only {
                    let query = self.query.trim().to_lowercase();
                    let name = if c.category == "Effect" {
                        c.label.strip_prefix("Effect · ").unwrap_or(&c.label)
                    } else {
                        &c.label
                    };
                    return ((query.is_empty() && self.recents.contains(&c.command))
                        || (!query.is_empty() && name.to_lowercase().contains(&query)))
                    .then_some((2, i));
                }
                let label = rank(&c.label, self.query.trim());
                // Command ids only match literally; subsequences of ids are noise.
                let command = c.command.to_lowercase().contains(&query).then_some(2);
                let alias = c
                    .aliases
                    .iter()
                    .filter(|term| {
                        term.to_lowercase()
                            .contains(self.query.trim().to_lowercase().as_str())
                    })
                    .map(|_| 2)
                    .min();
                label
                    .into_iter()
                    .chain(command)
                    .chain(alias)
                    .min()
                    .map(|r| (r, i))
            })
            .collect();
        let section = |i: usize| {
            let choice = &self.choices[i];
            (choice.provider.is_some(), choice.provider.as_deref().unwrap_or(&choice.category))
        };
        let mut best = std::collections::HashMap::new();
        for &(r, i) in &ranked {
            let at = best.entry(section(i)).or_insert((r, i));
            *at = (*at).min((r, i));
        }
        ranked.sort_by_key(|&(r, i)| (best[&section(i)], r, i));
        let mut indices: Vec<_> = ranked.into_iter().map(|(_, i)| i).collect();
        if self.query.trim().is_empty() {
            indices.sort_by_key(|&i| {
                self.recents
                    .iter()
                    .position(|id| id == &self.choices[i].command)
                    .unwrap_or(usize::MAX)
            });
        }
        indices
    }
    /// The lit row: the pointer's while it is over one, else the keyboard's.
    pub fn lit(&self) -> Option<usize> {
        self.hover.or(self.keyed)
    }
    /// A typed query lights its best match; an empty one lights nothing.
    pub fn search(&mut self, q: String) {
        self.hover = None;
        self.query = q;
        self.keyed = if self.query.trim().is_empty() {
            None
        } else if self.has_provider_headers() {
            self.rows()
                .iter()
                .position(|row| matches!(row, PickerRow::Choice(_)))
        } else {
            Some(0)
        };
    }
    /// The pointer is over `row`, or has left the rows. True if the light moved.
    pub fn point(&mut self, row: Option<usize>) -> bool {
        let before = self.lit();
        self.hover = row;
        before != self.lit()
    }
    /// Lights the first row whose label starts with `typed` (a menu with no
    /// search field). True if one does.
    pub fn type_ahead(&mut self, typed: &str) -> bool {
        let typed = typed.to_lowercase();
        let found = self.rows().iter().position(|row| {
            matches!(row, PickerRow::Choice(i)
                if self.choices[*i].label.to_lowercase().starts_with(&typed))
        });
        if found.is_some() {
            self.keyed = found;
            self.hover = None;
        }
        found.is_some()
    }
    pub fn navigate(&mut self, key: &str) -> Option<String> {
        let rows = self.rows();
        let n = rows.len();
        let lit = self.lit().filter(|&i| i < n);
        let to = match key {
            "escape" => {
                self.open = false;
                None
            }
            "up" if n > 0 => Some(lit.map_or(n - 1, |i| (i + n - 1) % n)),
            "down" if n > 0 => Some(lit.map_or(0, |i| (i + 1) % n)),
            "home" if n > 0 => Some(0),
            "end" if n > 0 => Some(n - 1),
            "left" => lit.and_then(|place| {
                let PickerRow::Choice(index) = rows[place] else {
                    return None;
                };
                let provider = self.choices[index].provider.as_ref()?;
                rows.iter().position(|row| {
                    matches!(row, PickerRow::Provider(i)
                    if self.choices[*i].provider.as_ref() == Some(provider))
                })
            }),
            "enter" => {
                let PickerRow::Choice(index) = rows[lit?] else {
                    return None;
                };
                let choice = &self.choices[index];
                self.open = choice.stays;
                return Some(choice.command.clone());
            }
            _ => None,
        };
        if to.is_some() {
            // The keyboard takes the light; the pointer has it back when it moves.
            self.keyed = to;
            self.hover = None;
        }
        None
    }
}

#[cfg(test)]
mod composer_tests {
    use super::*;

    #[test]
    fn menu_aligns_its_checked_row_and_stays_attached_at_window_edges() {
        let owner = Rect { x: 100., y: 200., w: 318., h: 540. };
        let anchor = Rect { x: 388., y: 210., w: 24., h: 24. };
        let frame = composer_picker_frame(owner, anchor, (360., 116.), Some(104.));
        assert_eq!(frame.x, owner.x + 8.);
        assert_eq!(frame.w, owner.w - 16.);
        assert_eq!(frame.y + frame.h - 104., anchor.y + anchor.h / 2.);
        let frame = composer_picker_frame(owner, anchor, (156., 116.), Some(30.));
        assert_eq!(frame.y, anchor.y + anchor.h);
        assert_eq!(frame.x + frame.w, owner.x + owner.w - 8.);
        let top = Rect { y: 710., ..anchor };
        let frame = composer_picker_frame(owner, top, (156., 180.), None);
        assert_eq!(frame.y + frame.h, owner.y + owner.h - 8.);
        assert!(frame.y <= top.y);
        let bottom = composer_picker_frame(owner, anchor, (360., 600.), None);
        assert_eq!(bottom.y, owner.y + 8.);
        assert_eq!(bottom.h, owner.h - 16.);
    }
}

#[cfg(test)]
mod palette_layer_tests {
    use super::*;
    #[test]
    fn layer_rows_require_a_trimmed_case_insensitive_name_search() {
        let mut picker = Picker {
            choices: vec![
                Choice::new("Logo", "palette.selectLayer:private-id")
                    .within("Layer")
                    .on_search(),
            ],
            ..Default::default()
        };
        assert!(picker.filtered().is_empty());
        picker.search("  LOG  ".into());
        assert_eq!(picker.filtered(), vec![0]);
        assert_eq!(
            picker.navigate("enter").as_deref(),
            Some("palette.selectLayer:private-id")
        );
        picker.search("private-id".into());
        assert!(picker.filtered().is_empty());
    }

    #[test]
    fn informational_text_is_visible_but_not_a_choice() {
        let mut picker = Picker {
            choices: vec![
                Choice::new("Opus 5.5", "model:opus"),
                Choice {
                    informational: true,
                    ..Choice::new("Finding models", "")
                },
            ],
            ..Default::default()
        };
        assert_eq!(picker.filtered(), vec![0]);
        picker.navigate("end");
        assert_eq!(picker.navigate("enter").as_deref(), Some("model:opus"));
        picker.search("Finding".into());
        assert_eq!(picker.note(), Some("Finding models"));
        assert!(picker.filtered().is_empty());
        assert_eq!(picker.navigate("enter"), None);
    }
}

/// Ported verbatim from Electron's palette-model.ts AFTER_EFFECTS_TERMS.
pub const AFTER_EFFECTS_TERMS: &[(&str, &[&str])] = &[
    ("toggleContinuousRaster", &["Collapse Transformations"]),
    ("toggle3DLayer", &["3D Layer Switch"]),
    ("precompose", &["Nest Layers", "Precompose"]),
    ("releasePrecomp", &["Unprecompose"]),
    ("newNull", &["Null Object"]),
    ("newAdjustment", &["Adjustment Layer"]),
    ("newShape", &["Shape Layer"]),
    ("graph", &["Speed Graph", "Value Graph"]),
    ("easyEase", &["Keyframe Assistant"]),
    ("split", &["Split Layer"]),
    ("groupLayers", &["Nest"]),
    ("compositionSettings", &["Composition Settings"]),
    ("palette", &["Search"]),
];
#[cfg(test)]
mod palette_alias_tests {
    use super::*;
    #[test]
    fn every_electron_alias_finds_the_original_command_with_electron_wording() {
        assert_eq!(AFTER_EFFECTS_TERMS.len(), 13);
        for (command, terms) in AFTER_EFFECTS_TERMS {
            for term in *terms {
                let choice = Choice::new("Original label", *command).with_after_effects_terms();
                let mut picker = Picker {
                    choices: vec![choice],
                    ..Default::default()
                };
                picker.search(term.to_uppercase());
                assert_eq!(picker.filtered(), vec![0], "{term}");
                assert_eq!(
                    picker.choices[0].display_label(term),
                    format!("Original label — {term} in After Effects")
                );
                assert_eq!(picker.navigate("enter").as_deref(), Some(*command));
            }
        }
    }
    #[test]
    fn matching_command_label_keeps_its_wording() {
        let choice = Choice::new("New Null Object", "newNull").with_after_effects_terms();
        assert_eq!(choice.display_label("Null Object"), "New Null Object");
        assert_eq!(choice.display_label(""), "New Null Object");
    }
}

#[cfg(test)]
mod palette_effect_tests {
    use super::*;
    #[test]
    fn effects_search_definition_names_and_preserve_electron_wording() {
        let mut picker = Picker {
            choices: vec![
                Choice::new("Effect · Gaussian Blur", "palette.addEffect")
                    .within("Effect")
                    .on_search(),
            ],
            ..Default::default()
        };
        assert!(picker.filtered().is_empty());
        picker.search("GAUSSIAN".into());
        assert_eq!(picker.filtered(), vec![0]);
        assert_eq!(
            picker.choices[0].display_label("GAUSSIAN"),
            "Effect · Gaussian Blur"
        );
        picker.search("Effect".into());
        assert!(picker.filtered().is_empty());
    }
}

/// Most recently executed palette commands, newest first.
pub fn remember_command(recents: &mut Vec<String>, command: &str) {
    recents.retain(|id| id != command);
    recents.insert(0, command.to_owned());
    recents.truncate(8);
}
#[cfg(test)]
mod recent_tests {
    use super::*;
    #[test]
    fn recents_are_unique_capped_and_first_only_for_empty_search() {
        let mut recents = vec![];
        for i in 0..10 {
            remember_command(&mut recents, &i.to_string());
        }
        remember_command(&mut recents, "8");
        assert_eq!(recents.len(), 8);
        assert_eq!(&recents[..2], &["8", "9"]);
        let mut picker = Picker {
            choices: vec![Choice::new("Alpha", "a"), Choice::new("Beta", "8")],
            recents,
            ..Default::default()
        };
        assert_eq!(picker.filtered(), vec![1, 0]);
        picker.search("Alpha".into());
        assert_eq!(picker.filtered(), vec![0]);
    }
}

#[cfg(test)]
mod recent_content_tests {
    use super::*;
    #[test]
    fn recent_effects_and_layers_reappear_and_run_without_a_search() {
        let mut picker = Picker {
            choices: vec![
                Choice::new("Command", "command"),
                Choice::new("Effect · Blur", "palette.addEffect:blur")
                    .within("Effect")
                    .on_search(),
                Choice::new("Logo", "palette.selectLayer:logo")
                    .within("Layer")
                    .on_search(),
                Choice::new("Other", "palette.selectLayer:other")
                    .within("Layer")
                    .on_search(),
            ],
            recents: vec![
                "palette.selectLayer:missing".into(),
                "palette.selectLayer:logo".into(),
                "palette.addEffect:blur".into(),
            ],
            ..Default::default()
        };
        for query in ["", "  "] {
            picker.search(query.into());
            assert_eq!(picker.filtered(), vec![2, 1, 0]);
            picker.navigate("down");
            assert_eq!(
                picker.navigate("enter").as_deref(),
                Some("palette.selectLayer:logo")
            );
            picker.navigate("down");
            assert_eq!(
                picker.navigate("enter").as_deref(),
                Some("palette.addEffect:blur")
            );
        }
        picker.search("Other".into());
        assert_eq!(picker.filtered(), vec![3]);
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;

    fn menu() -> Picker {
        let mut picker = Picker {
            choices: vec![
                Choice::new("GPT Sol", "sol")
                    .within("ChatGPT")
                    .checked(true)
                    .for_provider("codex"),
                Choice::new("Opus", "opus")
                    .within("Claude")
                    .checked(false)
                    .for_provider("claude"),
            ],
            open: true,
            ..Default::default()
        };
        picker.open_providers(Default::default());
        picker
    }

    #[test]
    fn provider_toggle_keeps_menu_and_model_and_reopen_restores_choices() {
        let mut picker = menu();
        assert_eq!(picker.filtered(), vec![0, 1]);
        assert_eq!(
            picker.rows(),
            vec![
                PickerRow::Provider(0),
                PickerRow::Choice(0),
                PickerRow::Provider(1)
            ]
        );
        assert_eq!(
            picker.toggle_provider(0, None),
            Some(("codex".into(), false))
        );
        assert!(picker.open);
        assert_eq!(picker.choices[0].checked, Some(true));
        assert_eq!(picker.provider_summary(0), "GPT Sol");
        picker.replace_choices(picker.choices.clone());
        assert!(!picker.provider_expanded(0), "refresh preserves a manual collapse");
        assert_eq!(
            picker.toggle_provider(1, None),
            Some(("claude".into(), true))
        );
        picker.open_providers([("codex".into(), false), ("claude".into(), true)].into());
        assert_eq!(
            picker.rows(),
            vec![
                PickerRow::Provider(0),
                PickerRow::Choice(0),
                PickerRow::Provider(1),
                PickerRow::Choice(1)
            ]
        );
    }

    #[test]
    fn search_expands_matching_sections_and_clearing_restores_them() {
        let mut picker = menu();
        picker.search("Opus".into());
        assert_eq!(
            picker.rows(),
            vec![PickerRow::Provider(1), PickerRow::Choice(1)]
        );
        assert!(picker.provider_expanded(1));
        assert_eq!(picker.toggle_provider(1, None), None);
        picker.search(String::new());
        assert!(!picker.provider_expanded(1));
        picker.choices.pop();
        assert_eq!(picker.rows(), vec![PickerRow::Choice(0)]);
    }

    #[test]
    fn keyboard_expands_collapses_and_returns_to_the_header() {
        let mut picker = menu();
        picker.navigate("down");
        assert_eq!(picker.lit_row(), Some(PickerRow::Provider(0)));
        picker.navigate("down");
        assert_eq!(picker.lit_row(), Some(PickerRow::Choice(0)));
        picker.navigate("left");
        assert_eq!(picker.lit_row(), Some(PickerRow::Provider(0)));
        assert_eq!(picker.provider_key("left"), Some(("codex".into(), false)));
        assert_eq!(picker.provider_key("right"), Some(("codex".into(), true)));
        assert_eq!(picker.provider_key("enter"), Some(("codex".into(), false)));
        assert_eq!(picker.provider_key("space"), Some(("codex".into(), true)));
        assert!(picker.open);
        picker.navigate("escape");
        assert!(!picker.open);
    }

    #[test]
    fn growing_a_model_menu_keeps_its_bottom_attached_and_inside_the_inset() {
        let owner = Rect {
            x: 0.,
            y: 0.,
            w: 320.,
            h: 500.,
        };
        let anchor = Rect {
            x: 16.,
            y: 10.,
            w: 80.,
            h: 24.,
        };
        let short = model_picker_frame(owner, anchor, (260., 64.));
        let tall = model_picker_frame(owner, anchor, (260., 500.));
        assert_eq!(short.y, 34.);
        assert_eq!(short.y, tall.y);
        assert_eq!(tall.y + tall.h, 492.);
    }
}
