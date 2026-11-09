//! Platform-independent search/navigation state for the two pooled child windows.
#[derive(Clone, Debug)]
pub struct Choice {
    pub label: String,
    pub command: String,
}
#[derive(Clone, Debug, Default)]
pub struct Picker {
    pub choices: Vec<Choice>,
    pub query: String,
    pub selected: usize,
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
    /// Matching choice indices, best match first; ties keep their original order.
    pub fn filtered(&self) -> Vec<usize> {
        let mut ranked: Vec<(u32, usize)> = self
            .choices
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let label = rank(&c.label, &self.query);
                // Command ids only match literally; subsequences of ids are noise.
                let command = c
                    .command
                    .to_lowercase()
                    .contains(&self.query.to_lowercase())
                    .then_some(2);
                label.into_iter().chain(command).min().map(|r| (r, i))
            })
            .collect();
        ranked.sort();
        ranked.into_iter().map(|(_, i)| i).collect()
    }
    pub fn search(&mut self, q: String) {
        self.query = q;
        self.selected = 0;
    }
    pub fn navigate(&mut self, key: &str) -> Option<String> {
        let n = self.filtered().len();
        match key {
            "escape" => self.open = false,
            "up" if n > 0 => self.selected = (self.selected + n - 1) % n,
            "down" if n > 0 => self.selected = (self.selected + 1) % n,
            "home" => self.selected = 0,
            "end" if n > 0 => self.selected = n - 1,
            "enter" if n > 0 => {
                self.open = false;
                return Some(self.choices[self.filtered()[self.selected]].command.clone());
            }
            _ => {}
        }
        None
    }
}
