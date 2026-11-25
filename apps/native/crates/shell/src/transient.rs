//! Platform-independent search/navigation state for the two pooled child windows.
use crate::layout::Rect;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposerPicker {
    Model,
    Effort,
    Access,
    /// The composer's "+": attachments, Select Part of the Screen and the
    /// Approvals choice.
    Add,
}
impl ComposerPicker {
    /// The kind a panel's menu request names (`picker`).
    pub fn named(name: Option<&str>) -> Option<Self> {
        match name? {
            "model" => Some(Self::Model),
            "effort" => Some(Self::Effort),
            "access" => Some(Self::Access),
            "add" => Some(Self::Add),
            _ => None,
        }
    }
    pub fn width(self) -> f32 {
        match self {
            Self::Model => 260.,
            Self::Effort => 156.,
            Self::Access | Self::Add => 360.,
        }
    }
    /// Its rows carry their description under the label (Approvals').
    pub fn describes(self) -> bool {
        matches!(self, Self::Access | Self::Add)
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
    /// Read-only text below the choices, outside keyboard and pointer
    /// selection; with a provider, a greyed row in that provider's group.
    pub informational: bool,
    /// In a menu of provider groups, a row outside them listed under its
    /// own plain section heading (the harness), not a group.
    pub section: bool,
    /// A small mark drawn before the row's title (a symbol name).
    pub icon: Option<String>,
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
        if self.aliases.is_empty() {
            return self.label.clone();
        }
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
    rank_lowercase(text, &query.to_lowercase())
}

fn lowercase(text: &str) -> std::borrow::Cow<'_, str> {
    if text.is_ascii() && !text.bytes().any(|b| b.is_ascii_uppercase()) {
        return text.into();
    }
    text.to_lowercase().into()
}

fn rank_lowercase(text: &str, q: &str) -> Option<u32> {
    if q.is_empty() {
        return Some(0);
    }
    let t = lowercase(text);
    if let Some(rank) = rank_contiguous(&t, q) {
        return Some(rank);
    }
    let mut chars = t.chars();
    q.chars()
        .filter(|c| !c.is_whitespace())
        .all(|qc| chars.any(|tc| tc == qc))
        .then_some(3)
}

fn rank_contiguous(text: &str, q: &str) -> Option<u32> {
    if text.starts_with(q) {
        return Some(0);
    }
    let i = text.find(q)?;
    let word = text[..i]
        .chars()
        .next_back()
        .is_some_and(|c| !c.is_alphanumeric());
    Some(if word { 1 } else { 2 })
}

fn rank_content_name(name: &str, query: &str) -> Option<u32> {
    if name == query {
        return Some(0);
    }
    let rank = rank_contiguous(name, query)?;
    // An earlier substring can precede a better word-start match.
    if rank == 2
        && name
            .char_indices()
            .any(|(i, c)| !c.is_alphanumeric() && name[i + c.len_utf8()..].starts_with(query))
    {
        return Some(2);
    }
    Some(rank + 1)
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

    /// Provider groups get their collapsible headers when there are two
    /// or more, or when plain sections stand beside even one.
    pub fn has_provider_headers(&self) -> bool {
        let mut providers = self
            .choices
            .iter()
            .filter_map(|choice| choice.provider.as_ref());
        providers.next().is_some_and(|first| {
            providers.any(|provider| provider != first)
                || self.choices.iter().any(|choice| choice.section)
        })
    }
    /// A displayed row a person can light and choose: not a provider's
    /// greyed, read-only row.
    fn selectable(&self, row: PickerRow) -> bool {
        !matches!(row, PickerRow::Choice(i) if self.choices[i].informational)
    }

    pub fn rows(&self) -> Vec<PickerRow> {
        self.rows_and_unlisted().0
    }

    /// The displayed rows and their overflow, from one ranked list.
    pub fn rows_and_unlisted(&self) -> (Vec<PickerRow>, usize) {
        let (indices, unlisted) = self.filtered_and_unlisted();
        let headers = self.has_provider_headers();
        let mut seen = HashSet::new();
        let mut rows = Vec::new();
        for index in indices {
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
        (rows, unlisted)
    }

    /// A group is open while searched, else as last left; one never
    /// touched opens only when it has nothing to choose (Aerolite's note).
    pub fn provider_expanded(&self, index: usize) -> bool {
        let provider = &self.choices[index].provider;
        !self.query.trim().is_empty()
            || provider
                .as_ref()
                .and_then(|provider| self.provider_choices.get(provider))
                .copied()
                .unwrap_or_else(|| {
                    self.choices
                        .iter()
                        .filter(|choice| &choice.provider == provider)
                        .all(|choice| choice.informational)
                })
    }

    pub fn provider_summary(&self, index: usize) -> String {
        self.provider_summary_with_count(index, || {
            self.filtered()
                .into_iter()
                .filter(|&i| self.choices[i].provider == self.choices[index].provider)
                .count()
        })
    }

    pub(crate) fn provider_summary_with_count(
        &self,
        index: usize,
        matched: impl FnOnce() -> usize,
    ) -> String {
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
        let count = if self.query.trim().is_empty() {
            members().count()
        } else {
            matched()
        };
        count.to_string()
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
    /// Matching choice indices. Within each search priority, sections stand
    /// where their best match does and their rows run best match first.
    pub fn filtered(&self) -> Vec<usize> {
        self.filtered_and_unlisted().0
    }

    fn filtered_and_unlisted(&self) -> (Vec<usize>, usize) {
        let mut rows = self.matches();
        let mut unlisted = 0;
        if let Some(page) = self.page {
            unlisted = rows.len().saturating_sub(page);
            rows.truncate(page);
        }
        (rows, unlisted)
    }
    /// Matching rows past the page, not listed.
    pub fn unlisted(&self) -> usize {
        self.page
            .map_or(0, |page| self.matches().len().saturating_sub(page))
    }
    /// The one read-only line below the choices (an informational row
    /// outside any provider group).
    pub fn note(&self) -> Option<&str> {
        self.choices
            .iter()
            .find(|c| c.informational && c.provider.is_none())
            .map(|c| c.label.as_str())
    }
    pub fn selected(&self) -> Option<usize> {
        self.rows().iter().position(
            |row| matches!(row, PickerRow::Choice(i) if self.choices[*i].checked == Some(true)),
        )
    }
    fn matches(&self) -> Vec<usize> {
        let trimmed = self.query.trim();
        let trimmed_lower = lowercase(trimmed);
        let query_lower = if trimmed.len() == self.query.len() {
            None
        } else {
            Some(lowercase(&self.query))
        };
        let query_lower = query_lower.as_deref().unwrap_or(&trimmed_lower);
        // Sort by command or content priority, then by content match quality and length.
        let mut ranked: Vec<_> = self
            .choices
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.informational {
                    // A provider's greyed row stands in its group while
                    // nothing is searched.
                    return (c.provider.is_some() && trimmed.is_empty()).then_some(((0, 0, 0), i));
                }
                if c.search_only {
                    if trimmed.is_empty() {
                        return self.recents.contains(&c.command).then_some(((2, 0, 0), i));
                    }
                    let is_effect = c.category == "Effect";
                    let name = if is_effect {
                        c.label.strip_prefix("Effect · ").unwrap_or(&c.label)
                    } else {
                        &c.label
                    };
                    let name = lowercase(name);
                    let quality = rank_content_name(&name, &trimmed_lower)?;
                    let priority = if is_effect { 3 } else { 4 };
                    return Some(((priority, quality, name.len()), i));
                }
                let label = rank_lowercase(&c.label, &trimmed_lower);
                if let Some(rank) = label.filter(|&rank| rank <= 2) {
                    return Some(((rank, 0, 0), i));
                }
                // Command ids only match literally; subsequences of ids are noise.
                if lowercase(&c.command).contains(query_lower)
                    || c.aliases
                        .iter()
                        .any(|term| lowercase(term).contains(trimmed_lower.as_ref()))
                {
                    return Some(((2, 0, 0), i));
                }
                label.map(|_| ((5, 0, 0), i))
            })
            .collect();
        let has_content_matches =
            !trimmed.is_empty() && ranked.iter().any(|&(_, i)| self.choices[i].search_only);
        let section = |priority: u32, i: usize| {
            let choice = &self.choices[i];
            // Keep literal command matches together ahead of content matches.
            let priority = if has_content_matches {
                priority.max(2)
            } else {
                0
            };
            (
                priority,
                choice.provider.is_some(),
                choice.provider.as_deref().unwrap_or(&choice.category),
            )
        };
        let mut best = std::collections::HashMap::new();
        for &(r, i) in &ranked {
            let at = best.entry(section(r.0, i)).or_insert((r, i));
            *at = (*at).min((r, i));
        }
        ranked.sort_by_key(|&(r, i)| (best[&section(r.0, i)], r, i));
        let mut indices: Vec<_> = ranked.into_iter().map(|(_, i)| i).collect();
        if trimmed.is_empty() {
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
    /// Changes the query and selects its best displayed result using the same rows.
    pub(crate) fn search_and_list(&mut self, query: String) -> (Vec<PickerRow>, usize) {
        self.hover = None;
        self.query = query;
        let (rows, unlisted) = self.rows_and_unlisted();
        self.keyed = if self.query.trim().is_empty() {
            None
        } else {
            rows.iter()
                .position(|row| matches!(row, PickerRow::Choice(_)))
        };
        (rows, unlisted)
    }

    /// The pointer is over `row`, or has left the rows. True if the light moved.
    pub fn point(&mut self, row: Option<usize>) -> bool {
        let before = self.lit();
        let rows = self.rows();
        self.hover = row.filter(|&place| rows.get(place).is_some_and(|&row| self.selectable(row)));
        before != self.lit()
    }
    /// Lights the first row whose label starts with `typed` (a menu with no
    /// search field). True if one does.
    pub fn type_ahead(&mut self, typed: &str) -> bool {
        let typed = typed.to_lowercase();
        let found = self.rows().iter().position(|row| {
            matches!(row, PickerRow::Choice(i)
                if !self.choices[*i].informational
                    && self.choices[*i].label.to_lowercase().starts_with(&typed))
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
        // The next row the keyboard can light, past greyed ones.
        let step = |from: Option<usize>, back: bool| {
            let first = match from {
                Some(i) if back => (i + n - 1) % n,
                Some(i) => (i + 1) % n,
                None if back => n - 1,
                None => 0,
            };
            (0..n)
                .map(|k| {
                    if back {
                        (first + n - k) % n
                    } else {
                        (first + k) % n
                    }
                })
                .find(|&place| self.selectable(rows[place]))
        };
        let to = match key {
            "escape" => {
                self.open = false;
                None
            }
            "up" if n > 0 => step(lit, true),
            "down" if n > 0 => step(lit, false),
            "home" if n > 0 => step(None, false),
            "end" if n > 0 => step(None, true),
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
                if choice.informational {
                    return None;
                }
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

    #[test]
    fn harness_section_stands_beside_one_provider_group_whose_greyed_row_is_never_chosen() {
        let harness = |label: &str, id: &str, on: bool| Choice {
            section: true,
            ..Choice::new(label, format!("provider:{id}"))
                .within("Harness")
                .checked(on)
                .stays(true)
        };
        let mut picker = Picker {
            choices: vec![
                harness("Powermove", "aero", true),
                harness("Codex", "chatgpt", false),
                Choice::new("GPT Sol", "model:sol")
                    .within("OpenAI")
                    .for_provider("openai")
                    .checked(true),
                Choice {
                    informational: true,
                    ..Choice::new("Aerolite", "")
                        .within("Powermove")
                        .for_provider("powermove")
                },
                Choice {
                    section: true,
                    ..Choice::new("Add a provider…", "providers:")
                },
            ],
            ..Default::default()
        };
        picker.open_providers(Default::default());
        // One provider group still gets its header beside the harness.
        let mut lone = picker.clone();
        lone.choices.remove(3);
        assert!(lone.has_provider_headers());
        assert_eq!(picker.note(), None);
        assert_eq!(
            picker.rows(),
            vec![
                PickerRow::Choice(0),
                PickerRow::Choice(1),
                PickerRow::Provider(2),
                PickerRow::Choice(2),
                PickerRow::Provider(3),
                PickerRow::Choice(3),
                PickerRow::Choice(4),
            ]
        );
        assert_eq!(picker.provider_summary(3), "1");
        // The keyboard passes over Aerolite; the pointer never lights it.
        picker.keyed = Some(3);
        picker.navigate("down");
        picker.navigate("down");
        assert_eq!(picker.lit(), Some(6));
        picker.navigate("up");
        assert_eq!(picker.lit(), Some(4));
        assert!(!picker.point(Some(5)));
        assert_eq!(picker.hover, None);
        assert_eq!(picker.navigate("end"), None);
        assert_eq!(picker.navigate("enter").as_deref(), Some("providers:"));
        picker.search("Aerolite".into());
        assert!(!picker.filtered().contains(&3));
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

    #[test]
    fn effects_precede_layers_and_closer_name_matches_come_first() {
        let mut picker = Picker {
            choices: vec![
                Choice::new("Deep Glow Gradient 1", "layer:gradient")
                    .within("Layer")
                    .on_search(),
                Choice::new("Deep Glow 1", "layer:glow")
                    .within("Layer")
                    .on_search(),
                Choice::new("Effect · Deep Glow Gradient", "effect:gradient")
                    .within("Effect")
                    .on_search(),
                Choice::new("Effect · Deep Glow", "effect:glow")
                    .within("Effect")
                    .on_search(),
            ],
            ..Default::default()
        };
        picker.search("deep glow".into());
        assert_eq!(picker.filtered(), vec![3, 2, 1, 0]);
    }

    #[test]
    fn effect_substrings_precede_other_search_only_prefixes() {
        let mut picker = Picker {
            choices: vec![
                Choice::new("Glowing Borders", "preset:borders")
                    .within("Preset")
                    .on_search(),
                Choice::new("Glow 1", "layer:glow")
                    .within("Layer")
                    .on_search(),
                Choice::new("Effect · Afterglow", "effect:afterglow")
                    .within("Effect")
                    .on_search(),
                Choice::new("Lowglow Settings", "settings"),
                Choice::new("Grid Layout Overview", "overview"),
                Choice::new("Glow Settings", "glowSettings"),
                Choice::new("Adjust Glow", "adjustGlow"),
                Choice::new("Effect · Afterglow · Glow", "effect:word-start")
                    .within("Effect")
                    .on_search(),
            ],
            ..Default::default()
        };
        picker.search("glow".into());
        assert_eq!(picker.filtered(), vec![5, 6, 3, 7, 2, 1, 0, 4]);
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
    fn rows_and_unlisted_preserves_paging_and_provider_disclosure() {
        let mut picker = menu();
        picker
            .choices
            .push(Choice::new("Read only", "note").on_search());
        for query in ["", "  ", "O", "  OpUs  ", "unmatched"] {
            picker.search(query.to_owned());
            for page in [None, Some(0), Some(1), Some(2), Some(10)] {
                picker.page = page;
                assert_eq!(
                    picker.rows_and_unlisted(),
                    (picker.rows(), picker.unlisted())
                );
            }
        }
        picker.search(String::new());
        picker.page = Some(1);
        assert_eq!(
            picker.rows_and_unlisted(),
            (vec![PickerRow::Provider(0), PickerRow::Choice(0)], 1)
        );
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
    fn provider_summary_counts_only_matching_rows_during_search() {
        let mut picker = menu();
        picker.choices.extend([
            Choice::new("Sonnet 5.5", "sonnet-5.5")
                .within("Claude")
                .for_provider("claude"),
            Choice::new("Sonnet 5", "sonnet-5")
                .within("Claude")
                .for_provider("claude"),
            Choice::new("Sonnet 4.6", "sonnet-4.6")
                .within("Claude")
                .for_provider("claude"),
        ]);
        assert_eq!(picker.provider_summary(1), "4");
        picker.search("sonnet".into());
        let matched = picker.filtered();
        assert_eq!(matched.len(), 3);
        assert_eq!(picker.provider_summary(matched[0]), "3");
        picker.search(String::new());
        assert_eq!(picker.provider_summary(1), "4");
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
