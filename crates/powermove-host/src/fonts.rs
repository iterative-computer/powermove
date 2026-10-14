//! System font families for the font picker (`fonts:families`).
//!
//! The renderer calls this on boot and whenever a window regains focus, so a
//! newly installed font appears without a restart. Scanning parses every
//! installed face's name table, so it runs on a blocking thread.

use std::collections::BTreeSet;

use serde_json::json;

use crate::router::Router;

/// Unique family names of the installed fonts, sorted.
pub fn system_families() -> Vec<String> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let families: BTreeSet<String> = database
        .faces()
        .filter_map(|face| face.families.first().map(|(name, _)| name.trim().to_owned()))
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .collect();
    families.into_iter().collect()
}

pub fn register(router: &mut Router) {
    router.json("fonts:families", |_| async {
        let families = tokio::task::spawn_blocking(system_families).await?;
        Ok(json!(families))
    });
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn finds_a_core_windows_font() {
        assert!(super::system_families().iter().any(|family| family == "Segoe UI" || family == "Arial"));
    }
}
