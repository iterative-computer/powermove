//! Semantic focus, paging and follow state. DESIGN-TODO(opus): the v4 view
//! maps these facts to reviewed controls; this crate specifies no geometry.
use crate::{Error, Result};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    pub busy: bool,
    pub updated_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerFocus {
    Search,
    Thread(String),
    Trigger,
    Outside,
}
#[derive(Clone, Debug)]
pub struct ThreadPicker {
    pub open: bool,
    pub focus: PickerFocus,
    pub query: String,
    threads: Vec<ThreadSummary>,
    matching: Vec<usize>,
}
impl Default for ThreadPicker {
    fn default() -> Self {
        Self {
            open: false,
            focus: PickerFocus::Trigger,
            query: String::new(),
            threads: vec![],
            matching: vec![],
        }
    }
}
impl ThreadPicker {
    pub fn open(&mut self, threads: Vec<ThreadSummary>, blocked: bool) -> Result<()> {
        if blocked {
            return Err(Error::Busy);
        }
        self.threads = threads;
        self.query.clear();
        self.refilter();
        self.open = true;
        self.focus = PickerFocus::Search;
        Ok(())
    }
    fn refilter(&mut self) {
        let needle = self.query.trim().to_lowercase();
        self.matching = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.title.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
    }
    pub fn search(&mut self, query: String) {
        self.query = query;
        self.refilter();
        self.focus = PickerFocus::Search;
    }
    pub fn count(&self) -> usize {
        self.matching.len()
    }
    pub fn page(&self, range: Range<usize>) -> Vec<&ThreadSummary> {
        let start = range.start.min(self.count());
        self.matching[start..range.end.max(start).min(self.count())]
            .iter()
            .map(|i| &self.threads[*i])
            .collect()
    }
    pub fn step(&mut self, delta: i32) {
        if !self.open || self.matching.is_empty() {
            return;
        }
        let index = if let PickerFocus::Thread(id) = &self.focus {
            self.matching
                .iter()
                .position(|i| self.threads[*i].id == *id)
        } else {
            None
        };
        let next = match index {
            Some(i) => i as i64 + i64::from(delta),
            None if delta > 0 => 0,
            None => self.matching.len() as i64 - 1,
        };
        self.focus = if next < 0 {
            PickerFocus::Search
        } else {
            PickerFocus::Thread(
                self.threads[self.matching[(next as usize).min(self.count() - 1)]]
                    .id
                    .clone(),
            )
        };
    }
    pub fn choose(&mut self, id: Option<&str>) -> Result<String> {
        if !self.open {
            return Err(Error::Stale);
        }
        let id = id
            .map(str::to_owned)
            .or_else(|| match &self.focus {
                PickerFocus::Thread(id) => Some(id.clone()),
                _ => self.page(0..1).first().map(|t| t.id.clone()),
            })
            .ok_or(Error::Invalid("empty thread results"))?;
        if !self.matching.iter().any(|i| self.threads[*i].id == id) {
            return Err(Error::UnknownId(id));
        }
        self.close(false);
        Ok(id)
    }
    /// Escape/Tab restore the trigger; outside press/scroll/blur/resize do not.
    pub fn close(&mut self, restore_trigger: bool) {
        self.open = false;
        self.focus = if restore_trigger {
            PickerFocus::Trigger
        } else {
            PickerFocus::Outside
        };
    }
}
#[derive(Clone, Debug, Default)]
pub struct Follow {
    pub show_jump: bool,
    pub user_scrolled: bool,
    pub pointer_selecting: bool,
    pub has_text_selection: bool,
}
impl Follow {
    /// The host supplies booleans from the reviewed scroll thresholds. Tagged
    /// behavior was away >48 and near <160 (AgentPanel.svelte:126–155).
    pub fn scroll(&mut self, away: bool) {
        self.show_jump = away;
        self.user_scrolled = away;
    }
    pub fn should_follow(&self, near_end: bool) -> bool {
        !self.pointer_selecting && !self.has_text_selection && (!self.user_scrolled || near_end)
    }
    pub fn jump(&mut self) {
        self.user_scrolled = false;
        self.show_jump = false;
    }
    pub fn switch_thread(&mut self) {
        *self = Self::default();
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Checking,
    Connecting,
    Waiting,
    Connected,
    Disconnected,
    Unavailable,
}
#[derive(Clone, Debug)]
pub struct Connection {
    pub provider: String,
    pub generation: u64,
    pub status: ConnectionStatus,
    pub busy: bool,
    required: bool,
}
impl Connection {
    pub fn new(provider: String) -> Self {
        Self {
            provider,
            generation: 1,
            status: ConnectionStatus::Checking,
            busy: false,
            required: false,
        }
    }
    pub fn switch(&mut self, provider: String) -> u64 {
        self.provider = provider;
        self.generation += 1;
        self.status = ConnectionStatus::Checking;
        self.busy = false;
        self.generation
    }
    pub fn receive(&mut self, generation: u64, status: ConnectionStatus) -> bool {
        if generation != self.generation {
            return false;
        }
        if status != ConnectionStatus::Checking {
            self.required = status != ConnectionStatus::Connected;
        }
        self.status = status;
        self.busy = false;
        true
    }
    pub fn gate(&self) -> bool {
        if self.status == ConnectionStatus::Checking {
            self.required
        } else {
            self.status != ConnectionStatus::Connected
        }
    }
    pub fn setup(
        &self,
        idle: bool,
        conversation_empty: bool,
        activity_empty: bool,
        draft_empty: bool,
    ) -> bool {
        self.gate() && idle && conversation_empty && activity_empty && draft_empty
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisualState {
    pub expanded: bool,
    pub replay: u64,
    zoom_quarters: i32,
}
impl VisualState {
    pub fn expand(&mut self) {
        self.expanded = true;
        if self.zoom_quarters == 0 {
            self.zoom_quarters = 4;
        }
    }
    pub fn close(&mut self) {
        self.expanded = false;
        self.zoom_quarters = 4;
    }
    pub fn zoom(&mut self, delta: i32) {
        self.zoom_quarters = (if self.zoom_quarters == 0 {
            4
        } else {
            self.zoom_quarters
        } + delta)
            .clamp(2, 8);
    }
    pub fn reset_zoom(&mut self) {
        self.zoom_quarters = 4;
    }
    pub fn scale(&self) -> f64 {
        f64::from(if self.zoom_quarters == 0 {
            4
        } else {
            self.zoom_quarters
        }) / 4.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionAction {
    Connect,
    Retry,
    OpenSettings,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionRequest {
    pub provider: String,
    pub generation: u64,
    pub action: ConnectionAction,
}
impl Connection {
    pub fn begin(&mut self, retry: bool) -> Result<ConnectionRequest> {
        if self.busy {
            return Err(Error::Busy);
        }
        let action = if self.provider == "compatible" {
            ConnectionAction::OpenSettings
        } else if retry {
            ConnectionAction::Retry
        } else {
            ConnectionAction::Connect
        };
        if action != ConnectionAction::OpenSettings {
            self.busy = true;
            self.status = if retry {
                ConnectionStatus::Checking
            } else {
                ConnectionStatus::Connecting
            };
        }
        Ok(ConnectionRequest {
            provider: self.provider.clone(),
            generation: self.generation,
            action,
        })
    }
}
impl ThreadPicker {
    /// Search Enter chooses the first match; focused-row Enter/Space activates
    /// that row; Up/Down, Escape and Tab stay local to the picker.
    pub fn key(&mut self, key: &crate::input::Key) -> Result<Option<String>> {
        if !self.open || key.composing {
            return Ok(None);
        }
        match key.name.as_str() {
            "ArrowDown" => {
                self.step(1);
                Ok(None)
            }
            "ArrowUp" => {
                self.step(-1);
                Ok(None)
            }
            "Escape" | "Tab" => {
                self.close(true);
                Ok(None)
            }
            "Enter" => self.choose(None).map(Some),
            " " | "Space" if matches!(self.focus, PickerFocus::Thread(_)) => {
                self.choose(None).map(Some)
            }
            _ => Ok(None),
        }
    }
}
