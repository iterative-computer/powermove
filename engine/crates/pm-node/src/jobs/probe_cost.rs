//! Machine-local decode measurements appended to `probeMedia` facts. The
//! labels are readable in Interpret Footage and stable for the host policy;
//! none of these measured values enter project document state.

use pm_media::decode::measure::DecodeMeasurement;
use pm_media::{MediaError, Result};
use pm_protocol::media::{FactLine, MediaFacts, MediaKind};
use std::path::Path;

fn line(label: &str, value: impl Into<String>) -> FactLine {
    FactLine {
        label: label.into(),
        value: value.into(),
    }
}

fn append_measurement(facts: &mut MediaFacts, measurement: DecodeMeasurement) {
    facts.facts.extend([
        line("Decode codec", measurement.codec),
        line(
            "Coded size",
            format!(
                "{} × {} pixels",
                measurement.coded_size.0, measurement.coded_size.1
            ),
        ),
        line(
            "Decode time (p95)",
            if measurement.sufficient_samples {
                measurement.sequential_p95_ms.map_or_else(
                    || "Not enough frames to measure".into(),
                    |v| format!("{v:.3} ms"),
                )
            } else {
                "Not enough frames to measure".into()
            },
        ),
        line(
            "Frame budget",
            format!("{:.3} ms", measurement.frame_budget_ms),
        ),
        line(
            "Random seek time",
            measurement.random_seek_p95_ms.map_or_else(
                || "Not enough frames to measure".into(),
                |v| format!("{v:.3} ms"),
            ),
        ),
        line("Measured frames", measurement.fresh_frames.to_string()),
        line(
            "Proxy recommended",
            if measurement.sufficient_samples {
                if measurement.needs_proxy { "Yes" } else { "No" }
            } else {
                "Unknown"
            },
        ),
    ]);
}

/// Measures the original source after its read capability resolves.
/// `wait_until_idle` pauses before each frame while playback or an export
/// runs. A measurement error leaves Interpret Footage usable while giving
/// the policy an explicit unknown answer. Shutdown cancellation propagates.
pub(super) fn append_decode_cost(
    path: &Path,
    facts: &mut MediaFacts,
    wait_until_idle: impl FnMut() -> Result<()>,
) -> Result<()> {
    if facts.kind != MediaKind::Video {
        return Ok(());
    }
    match pm_media::decode::measure::measure_source_with_pause(path, wait_until_idle) {
        Ok(Some(measurement)) => append_measurement(facts, measurement),
        Ok(None) => {
            facts.facts.push(line("Proxy recommended", "Unknown"));
            facts
                .facts
                .push(line("Decode measurement", "No video stream"));
        }
        Err(MediaError::Cancelled) => return Err(MediaError::Cancelled),
        Err(error) => {
            facts.facts.push(line("Proxy recommended", "Unknown"));
            facts.facts.push(line(
                "Decode measurement",
                error.to_string().chars().take(512).collect::<String>(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_media::probe::CostClass;

    #[test]
    fn insufficient_measurement_is_explicitly_unknown() {
        let mut facts = MediaFacts {
            kind: MediaKind::Video,
            facts: vec![],
            settings: vec![],
            issues: vec![],
            display_size: None,
            duration: None,
        };
        append_measurement(
            &mut facts,
            DecodeMeasurement {
                codec: "prores".into(),
                coded_size: (3840, 2160),
                cost_class: CostClass::Heavy,
                frame_budget_ms: 33.333,
                sequential_p95_ms: Some(41.0),
                random_seek_p95_ms: Some(170.0),
                fresh_frames: 5,
                sufficient_samples: false,
                needs_proxy: false,
            },
        );
        let value = |label| {
            facts
                .facts
                .iter()
                .find(|f| f.label == label)
                .unwrap()
                .value
                .as_str()
        };
        assert_eq!(value("Proxy recommended"), "Unknown");
        assert_eq!(value("Decode time (p95)"), "Not enough frames to measure");
        assert_eq!(value("Frame budget"), "33.333 ms");
        assert_eq!(value("Random seek time"), "170.000 ms");
    }
}
