//! Per-clip time inputs that are not in `pm_eval`'s source time: the
//! clip's `pitch` choice and a conformed frame rate (plan section 16:
//! "Frame-rate override keeps audio in sync").
//!
//! * **Pitch** (`d.pitch`, additive, section 6): `"preserve"` (the
//!   default, also for an absent or unknown value) keeps the pitch when a
//!   clip plays faster or slower (Signalsmith Stretch); `"follow"` lets the
//!   pitch follow the speed (band-limited resampling, as tape would).
//! * **Conform** (Interpret Footage's `frameRate` on a video asset, the
//!   formats lane's `interpret` shape: `frameRate` and `frameRateRational`):
//!   presented frame n shows from n / rate, so clip time c shows the frame
//!   the file presents at c × conformed / native. The soundtrack follows the
//!   same map: it plays `conformed / native` times as fast (23.976 fps
//!   footage conformed to 24 plays its audio at 1.001×), with the clip's
//!   pitch choice.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use pm_eval::node;
use pm_model::Node;
use pm_time::Rate;

/// How a clip's pitch follows its speed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pitch {
    /// Speed changes keep the pitch (time stretch).
    #[default]
    Preserve,
    /// The pitch follows the speed (resampling).
    Follow,
}

impl Pitch {
    /// `d.pitch`: `"follow"` follows; anything else (absent, null,
    /// `"preserve"`, an unknown value) preserves.
    pub fn of(value: Option<&Node>) -> Pitch {
        match node::string(value).map(|s| s.to_string_lossy()) {
            Some(s) if s == "follow" => Pitch::Follow,
            _ => Pitch::Preserve,
        }
    }
}

/// The conformed rate of an asset record (`interpret.frameRate`, with
/// `frameRateRational` beside it or as an exact `[num, den]`), when it sets
/// one. The same reading as the video's (`pm_media::interpret`), so picture
/// and sound conform together.
pub fn conformed_rate(asset: Option<&Node>) -> Option<Rate> {
    let interpret = node::member(asset, "interpret");
    if node::is_nullish(interpret) {
        return None;
    }
    let text = pm_model::json::write::to_canonical_string(interpret?);
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    pm_media::interpret::Interpretation::from_json(&value)
        .0
        .frame_rate
}

type Key = (PathBuf, u64, Option<SystemTime>);

fn memo() -> &'static Mutex<HashMap<Key, Option<Rate>>> {
    static MEMO: OnceLock<Mutex<HashMap<Key, Option<Rate>>>> = OnceLock::new();
    MEMO.get_or_init(Default::default)
}

/// The nominal frame rate of a file's video stream (section 13 item 5), from
/// the probe, kept per file, size, and modification time.
pub fn native_rate(path: &Path) -> Option<Rate> {
    let meta = std::fs::metadata(path).ok()?;
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(r) = memo().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return *r;
    }
    let rate = pm_media::probe::probe(path)
        .ok()
        .and_then(|p| pm_media::probe::nominal(&p.probe));
    let mut memo = memo().lock().unwrap_or_else(|e| e.into_inner());
    if memo.len() > 4096 {
        memo.clear();
    }
    memo.insert(key, rate);
    rate
}

/// Source seconds per clip second for a conformed asset: `conformed /
/// native` (1 when nothing is conformed or the file has no video rate).
pub fn conform_factor(conformed: Option<Rate>, native: Option<Rate>) -> f64 {
    match (conformed, native) {
        (Some(c), Some(n)) if c != n => {
            // Exact in i128, then one rounding.
            (c.num() as i128 * n.den() as i128) as f64 / (c.den() as i128 * n.num() as i128) as f64
        }
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::Document;

    fn asset(json: &str) -> Document {
        Document::new(Some(pm_model::json::parse(json).unwrap()), 1)
    }

    #[test]
    fn pitch_defaults_to_preserve() {
        let doc = asset(r#"{"a":"follow","b":"preserve","c":3,"d":null}"#);
        let root = doc.root();
        assert_eq!(Pitch::of(node::member(root, "a")), Pitch::Follow);
        assert_eq!(Pitch::of(node::member(root, "b")), Pitch::Preserve);
        assert_eq!(Pitch::of(node::member(root, "c")), Pitch::Preserve);
        assert_eq!(Pitch::of(node::member(root, "d")), Pitch::Preserve);
        assert_eq!(Pitch::of(node::member(root, "missing")), Pitch::Preserve);
    }

    #[test]
    fn conformed_rates_read_the_interpret_shape() {
        let doc = asset(
            r#"{"exact":{"interpret":{"frameRate":23.976,"frameRateRational":[24000,1001]}},
                "float":{"interpret":{"frameRate":25}},
                "none":{"interpret":{"primaries":"bt709"}},
                "auto":{"interpret":{"frameRate":null}},
                "pair":{"interpret":{"frameRate":[30000,1001]}},
                "bad":{"interpret":{"frameRate":[24.5,1]}},
                "bare":{}}"#,
        );
        let root = doc.root();
        assert_eq!(
            conformed_rate(node::member(root, "exact")),
            Some(Rate::FPS_23_976)
        );
        assert_eq!(
            conformed_rate(node::member(root, "float")),
            Some(Rate::FPS_25)
        );
        assert_eq!(conformed_rate(node::member(root, "none")), None);
        assert_eq!(conformed_rate(node::member(root, "auto")), None);
        assert_eq!(conformed_rate(node::member(root, "bare")), None);
        assert_eq!(
            conformed_rate(node::member(root, "pair")),
            Some(Rate::FPS_29_97)
        );
        assert_eq!(conformed_rate(node::member(root, "bad")), None);
    }

    #[test]
    fn the_factor_is_conformed_over_native() {
        let f = conform_factor(Some(Rate::FPS_24), Some(Rate::FPS_23_976));
        assert!((f - 1.001).abs() < 1e-15, "{f}");
        assert_eq!(conform_factor(Some(Rate::FPS_25), Some(Rate::FPS_25)), 1.0);
        assert_eq!(conform_factor(None, Some(Rate::FPS_25)), 1.0);
        assert_eq!(conform_factor(Some(Rate::FPS_25), None), 1.0);
        let f = conform_factor(Some(Rate::FPS_25), Some(Rate::FPS_24));
        assert!((f - 25.0 / 24.0).abs() < 1e-15);
    }
}
