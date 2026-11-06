//! Connection/document fences for native agent sessions. The coordinator worker
//! serializes these actions; project mutation/publication stays on the kernel.
//! Connection loss revokes leases; owner loss leaves the external connection
//! available for an explicit fresh session after owner recovery.
use crate::error;
use serde_json::Value;
use std::io;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub document: String,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    connection: String,
    epoch: u64,
    owner: Owner,
    run: String,
}
impl Lease {
    pub fn owner(&self) -> &Owner {
        &self.owner
    }
    pub fn run(&self) -> &str {
        &self.run
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    connection: String,
    epoch: u64,
    owner: Owner,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finish {
    pub run: String,
    pub commit: bool,
    connection: String,
    epoch: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Committed {
    lease: Lease,
    pub receipt: Value,
}
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub result: Result<Value, String>,
    pub cleanup_error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelFence {
    lease: Lease,
    registration: String,
    generation: u64,
}
pub struct Gate {
    connection: String,
    epoch: u64,
    owner: Option<Owner>,
    opening: Option<Opening>,
    active: Option<Lease>,
    finishing: Option<Finish>,
    closed: bool,
}
impl Gate {
    pub fn new(connection: String, owner: Option<Owner>) -> io::Result<Self> {
        if !crate::valid_id(&connection, 200) {
            return Err(error("Invalid connection identity"));
        }
        Ok(Self {
            connection,
            epoch: 0,
            owner,
            opening: None,
            active: None,
            finishing: None,
            closed: false,
        })
    }
    fn advance(&mut self) -> io::Result<()> {
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| error("Session identity exhausted"))?;
        Ok(())
    }
    pub fn begin_open(&mut self) -> io::Result<Opening> {
        if self.closed {
            return Err(error("This MCP connection is closed."));
        }
        if self.opening.is_some() {
            return Err(error("A session is already opening."));
        }
        if self.active.is_some() || self.finishing.is_some() {
            return Err(error("Finish the current MCP session first."));
        }
        let owner = self
            .owner
            .clone()
            .ok_or_else(|| error("No native document host is available."))?;
        self.advance()?;
        let opening = Opening {
            connection: self.connection.clone(),
            epoch: self.epoch,
            owner,
        };
        self.opening = Some(opening.clone());
        Ok(opening)
    }
    /// Called after every awaited authorization/workspace preparation. Rejected
    /// resources must be archived by the owning host; they never become active.
    pub fn check_open(&self, opening: &Opening) -> io::Result<()> {
        if self.closed {
            return Err(error("This MCP connection is closed."));
        }
        if self.opening.as_ref() != Some(opening) || self.owner.as_ref() != Some(&opening.owner) {
            return Err(error("The document changed before the session opened."));
        }
        Ok(())
    }
    pub fn authorized(&mut self, opening: &Opening, run: String) -> io::Result<Lease> {
        self.check_open(opening)?;
        if !crate::valid_id(&run, 200) {
            return Err(error("Invalid run identity"));
        }
        let lease = Lease {
            connection: self.connection.clone(),
            epoch: opening.epoch,
            owner: opening.owner.clone(),
            run,
        };
        self.opening = None;
        self.active = Some(lease.clone());
        Ok(lease)
    }
    pub fn check(&self, lease: &Lease) -> io::Result<()> {
        if self.closed {
            return Err(error("This MCP connection is closed."));
        }
        if self.finishing.is_some() || self.active.as_ref() != Some(lease) {
            return Err(error("Call start_session before using Powermove tools."));
        }
        if self.owner.as_ref() != Some(&lease.owner) {
            return Err(error(
                "The native document host closed or changed. Start a new session.",
            ));
        }
        Ok(())
    }
    pub fn begin_finish(&mut self, commit: bool) -> io::Result<Finish> {
        if let Some(finishing) = &self.finishing {
            return Ok(finishing.clone());
        }
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| error("No MCP session is active."))?;
        let finish = Finish {
            run: active.run.clone(),
            commit,
            connection: self.connection.clone(),
            epoch: active.epoch,
        };
        self.finishing = Some(finish.clone());
        Ok(finish)
    }
    pub fn finished(
        &mut self,
        finish: &Finish,
        result: Result<Value, String>,
        cleanup_error: Option<String>,
    ) -> io::Result<Outcome> {
        if self.finishing.as_ref() != Some(finish) {
            return Err(error("Stale session finish completion"));
        }
        self.active = None;
        self.finishing = None;
        Ok(Outcome {
            result,
            cleanup_error,
        })
    }
    pub fn owner_lost(&mut self) -> io::Result<Option<Finish>> {
        self.owner = None;
        self.opening = None;
        self.advance()?;
        if self.active.is_some() {
            self.begin_finish(false).map(Some)
        } else {
            Ok(None)
        }
    }
    pub fn recovered(&mut self, owner: Owner) -> io::Result<()> {
        if self.closed {
            return Err(error("This MCP connection is closed."));
        }
        if self.active.is_some() || self.finishing.is_some() {
            return Err(error("Finish the interrupted session before recovery."));
        }
        self.advance()?;
        self.opening = None;
        self.owner = Some(owner);
        Ok(())
    }
    pub fn disconnect(&mut self) -> io::Result<Option<Finish>> {
        self.closed = true;
        self.opening = None;
        self.advance()?;
        if self.active.is_some() {
            self.begin_finish(false).map(Some)
        } else {
            Ok(None)
        }
    }
    /// One actor admission fence surrounds irreversible publication. Once it
    /// returns a receipt, subsequent stop/disconnect cannot relabel it cancelled.
    /// The closure uses the existing durable kernel/store transaction, not a
    /// second writer. Invoke off the UI thread; it must not run JS on main.
    pub fn commit_publication(
        &mut self,
        lease: &Lease,
        publish: impl FnOnce() -> io::Result<Value>,
    ) -> io::Result<Committed> {
        self.check(lease)?;
        let receipt = publish()?;
        Ok(Committed {
            lease: lease.clone(),
            receipt,
        })
    }
    pub fn may_prepare_next_stage(&self, committed: &Committed) -> bool {
        self.check(&committed.lease).is_ok()
    }
    pub fn capture_panel(
        &self,
        lease: &Lease,
        registration: &str,
        generation: u64,
    ) -> io::Result<PanelFence> {
        self.check(lease)?;
        Ok(PanelFence {
            lease: lease.clone(),
            registration: registration.into(),
            generation,
        })
    }
    /// Recheck after capture and immediately before actual native input.
    pub fn check_panel(
        &self,
        panel: &PanelFence,
        registration: &str,
        generation: u64,
    ) -> io::Result<()> {
        self.check(&panel.lease)?;
        if panel.registration != registration || panel.generation != generation {
            return Err(error(
                "The panel changed. Read its current state before another panel action.",
            ));
        }
        Ok(())
    }
}
