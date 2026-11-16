//! Process-local session leases. These identities never come from tool arguments.
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
fn leases() -> &'static Mutex<HashSet<String>> {
    static LEASES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    LEASES.get_or_init(Default::default)
}
pub struct Lease(String);
impl Lease {
    pub fn new() -> Self {
        let id = pm_kernel::new_id("panel-session");
        leases().lock().unwrap().insert(id.clone());
        Self(id)
    }
    pub fn id(&self) -> &str {
        &self.0
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut leases) = leases().lock() {
            leases.remove(&self.0);
        }
    }
}
pub fn live(id: &str) -> bool {
    leases().lock().is_ok_and(|leases| leases.contains(id))
}
#[cfg(test)]
mod tests {
    #[test]
    fn lease_drop_revokes_without_a_panel_render() {
        let lease = super::Lease::new();
        let id = lease.id().to_owned();
        assert!(super::live(&id));
        drop(lease);
        assert!(!super::live(&id));
    }
}
