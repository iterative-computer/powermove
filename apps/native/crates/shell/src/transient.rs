//! Platform-independent search/navigation state for the two pooled child windows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
    pub label: String,
    pub command: String,
    /// The section it is listed under; empty for none.
    pub category: String,
    /// `Some` keeps the check column; `Some(true)` shows the mark.
    pub checked: Option<bool>,
}
impl Choice {
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            ..Default::default()
        }
    }
    pub fn within(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
}
/// What a menu's rows are doing: nothing is lit when it opens. The keyboard
/// lights a row (an arrow, or the best match of a typed query); the pointer
/// lights the row under it and takes its light away when it leaves.
#[derive(Clone, Debug, Default)]
pub struct Picker {
    pub choices: Vec<Choice>,
    pub query: String,
    /// The keyboard's row, a place in `filtered`.
    pub keyed: Option<usize>,
    /// The row under the pointer, a place in `filtered`.
    pub hover: Option<usize>,
    pub open: bool,
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
        let query = self.query.to_lowercase();
        let mut ranked: Vec<(u32, usize)> = self
            .choices
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let label = rank(&c.label, &self.query);
                // Command ids only match literally; subsequences of ids are noise.
                let command = c.command.to_lowercase().contains(&query).then_some(2);
                label.into_iter().chain(command).min().map(|r| (r, i))
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
        ranked.into_iter().map(|(_, i)| i).collect()
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
                self.open = false;
                return Some(self.choices[self.filtered()[i]].command.clone());
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
