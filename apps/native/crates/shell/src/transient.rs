//! Platform-independent search/navigation state for the two pooled child windows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
    pub label: String,
    pub command: String,
    /// The section it is listed under; empty for none.
    pub category: String,
    /// `Some` keeps the check column; `Some(true)` shows the mark.
    pub checked: Option<bool>,
    /// A font family the row's title is drawn in (a font picker previews
    /// each face in its own row).
    pub font: Option<String>,
    /// Choosing it keeps the menu open (a provider row: its models follow
    /// in the same menu); the owner sends the menu's new rows.
    pub stays: bool,
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
    pub fn within(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
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
    /// The keyboard's row, a place in `filtered`.
    pub keyed: Option<usize>,
    /// The row under the pointer, a place in `filtered`.
    pub hover: Option<usize>,
    pub open: bool,
    /// The most rows listed at once (a font picker over every installed
    /// family); the search narrows the rest. Keys and the pointer reach
    /// only listed rows.
    pub page: Option<usize>,
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
    fn matches(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        let mut ranked: Vec<(u32, usize)> = self
            .choices
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
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
        let mut best = std::collections::HashMap::<&str, (u32, usize)>::new();
        for &(r, i) in &ranked {
            let at = best
                .entry(self.choices[i].category.as_str())
                .or_insert((r, i));
            *at = (*at).min((r, i));
        }
        ranked.sort_by_key(|&(r, i)| (best[self.choices[i].category.as_str()], r, i));
        let mut indices: Vec<_> = ranked.into_iter().map(|(_, i)| i).collect();
        if self.query.trim().is_empty() {
            indices.sort_by_key(|&i| self.recents.iter().position(|id| id == &self.choices[i].command).unwrap_or(usize::MAX));
        }
        indices
    }
    /// The lit row: the pointer's while it is over one, else the keyboard's.
    pub fn lit(&self) -> Option<usize> {
        self.hover.or(self.keyed)
    }
    /// A typed query lights its best match; an empty one lights nothing.
    pub fn search(&mut self, q: String) {
        self.keyed = (!q.trim().is_empty()).then_some(0);
        self.hover = None;
        self.query = q;
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
        let found = self
            .filtered()
            .into_iter()
            .position(|i| self.choices[i].label.to_lowercase().starts_with(&typed));
        if found.is_some() {
            self.keyed = found;
            self.hover = None;
        }
        found.is_some()
    }
    pub fn navigate(&mut self, key: &str) -> Option<String> {
        let n = self.filtered().len();
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
            "enter" => {
                // Return with nothing lit does nothing.
                let i = lit?;
                let choice = &self.choices[self.filtered()[i]];
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
        for i in 0..10 { remember_command(&mut recents, &i.to_string()); }
        remember_command(&mut recents, "8");
        assert_eq!(recents.len(), 8);
        assert_eq!(&recents[..2], &["8", "9"]);
        let mut picker = Picker { choices: vec![Choice::new("Alpha", "a"), Choice::new("Beta", "8")], recents, ..Default::default() };
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
                Choice::new("Effect · Blur", "palette.addEffect:blur").within("Effect").on_search(),
                Choice::new("Logo", "palette.selectLayer:logo").within("Layer").on_search(),
                Choice::new("Other", "palette.selectLayer:other").within("Layer").on_search(),
            ],
            recents: vec!["palette.selectLayer:missing".into(), "palette.selectLayer:logo".into(), "palette.addEffect:blur".into()],
            ..Default::default()
        };
        for query in ["", "  "] {
            picker.search(query.into());
            assert_eq!(picker.filtered(), vec![2, 1, 0]);
            picker.navigate("down");
            assert_eq!(picker.navigate("enter").as_deref(), Some("palette.selectLayer:logo"));
            picker.navigate("down");
            assert_eq!(picker.navigate("enter").as_deref(), Some("palette.addEffect:blur"));
        }
        picker.search("Other".into());
        assert_eq!(picker.filtered(), vec![3]);
    }
}
