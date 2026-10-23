//! The unity-gain document played by the source monitor.
//!
//! Keep the offline PCM parity probe on the same graph as device playback.

use pm_model::Document;

/// Builds the source monitor's isolated, one-voice document. Its source path
/// is resolved separately by the caller, including any browser rendition.
pub fn source_monitor_document(asset: &str, duration: f64) -> Result<Document, String> {
    if asset.is_empty() || !duration.is_finite() || duration <= 0.0 {
        return Err("The source audio needs an asset and a positive duration".into());
    }
    let document = serde_json::json!({
        "id": "source-monitor", "compId": "source", "name": "Source monitor",
        "w": 1, "h": 1, "fps": 30, "dur": duration,
        "layers": [{
            "id": "source-audio", "type": "audio", "name": "Source audio",
            "from": 0, "dur": duration, "on": true, "solo": false,
            "group": null, "parent": null, "p": {}, "fx": [], "masks": [],
            "d": {"asset": asset, "gain": 1, "trim": 0, "fadeIn": 0, "fadeOut": 0}
        }], "comps": {}, "assets": {}, "params": {}
    });
    let parsed = pm_model::json::parse(&document.to_string()).map_err(|e| e.to_string())?;
    Ok(Document::new(Some(parsed), 1))
}
