use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::{ProxyError, Result};

/// Measurements belong to this machine, never to a project document.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MachineProfile {
    pub gpu_frame_p95_ms: f64,
    pub gpu_headroom_ms: f64,
    pub disk_write_mib_s: f64,
    pub on_battery: bool,
}

/// A decoder sample measures one asset on this machine, including decode and
/// transfer. Its p95 comes from actual frames, not a codec-name guess.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DecodeMeasurement {
    pub original_p95_ms: f64,
    pub reduced_half_p95_ms: Option<f64>,
    pub reduced_quarter_p95_ms: Option<f64>,
    pub proxy_p95_ms: Option<f64>,
}

/// GPU p95 and per-asset decode samples are supplied by the playback engine;
/// the store volume's disk write throughput and power state are sampled here.
pub fn probe_machine_profile(store_root: &Path, gpu_frame_p95_ms: f64) -> Result<MachineProfile> {
    if !gpu_frame_p95_ms.is_finite() || gpu_frame_p95_ms < 0.0 {
        return Err(ProxyError::InvalidMedia("invalid GPU measurement".into()));
    }
    let mut file = NamedTempFile::new_in(store_root)?;
    let buf = vec![0x7b_u8; 1024 * 1024];
    let start = Instant::now();
    for _ in 0..8 {
        file.write_all(&buf)?;
    }
    file.as_file().sync_all()?;
    let elapsed = start.elapsed().as_secs_f64().max(f64::MIN_POSITIVE);
    let batt = Command::new("pmset").args(["-g", "batt"]).output()?;
    if !batt.status.success() {
        return Err(ProxyError::InvalidMedia(
            "power state measurement failed".into(),
        ));
    }
    let on_battery = String::from_utf8_lossy(&batt.stdout).contains("Battery Power");
    Ok(MachineProfile {
        gpu_frame_p95_ms,
        gpu_headroom_ms: (16.667 - gpu_frame_p95_ms).max(0.0),
        disk_write_mib_s: 8.0 / elapsed,
        on_battery,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepresentationIntent {
    Automatic,
    Originals,
    Proxies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionIntent {
    Automatic,
    Full,
    Half,
    Quarter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackMoment {
    Start,
    ClipBoundary,
    Continuing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Representation {
    Original,
    ReducedHalf,
    ReducedQuarter,
    Proxy,
}

/// An explicit state; an offline original must never masquerade as a proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaAvailability {
    Online,
    OriginalOfflineProxyAvailable,
    OriginalOfflineNoProxy,
    ProxyPending,
    ProxyFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutomaticInput {
    pub intent: RepresentationIntent,
    pub resolution: ResolutionIntent,
    pub moment: PlaybackMoment,
    pub availability: MediaAvailability,
    pub current: Representation,
    pub proxy_ready: bool,
    pub reduced_half_available: bool,
    pub reduced_quarter_available: bool,
    pub decode: DecodeMeasurement,
    pub profile: MachineProfile,
    pub fps: f64,
    /// Physical display pixels, after device-pixel-ratio.
    pub viewer_size: (u32, u32),
    pub source_size: (u32, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepresentationChoice {
    pub representation: Option<Representation>,
    pub offline: MediaAvailability,
    pub generate_proxy: bool,
}

/// Chooses once at playback start or clip boundaries. A paused Automatic
/// viewer uses the original at every preview resolution. No choice waits for
/// generation; if Proxy Only is selected before it exists, the source has a
/// named pending state rather than silently showing an original.
pub fn choose_representation(input: AutomaticInput) -> RepresentationChoice {
    if input.intent == RepresentationIntent::Originals {
        return RepresentationChoice {
            representation: (!matches!(
                input.availability,
                MediaAvailability::OriginalOfflineProxyAvailable
                    | MediaAvailability::OriginalOfflineNoProxy
            ))
            .then_some(Representation::Original),
            offline: input.availability,
            generate_proxy: false,
        };
    }
    let missing_original = matches!(
        input.availability,
        MediaAvailability::OriginalOfflineProxyAvailable
            | MediaAvailability::OriginalOfflineNoProxy
    );
    if input.intent == RepresentationIntent::Proxies {
        return RepresentationChoice {
            representation: input.proxy_ready.then_some(Representation::Proxy),
            offline: if missing_original {
                if input.proxy_ready {
                    MediaAvailability::OriginalOfflineProxyAvailable
                } else {
                    MediaAvailability::OriginalOfflineNoProxy
                }
            } else if input.proxy_ready {
                MediaAvailability::Online
            } else {
                MediaAvailability::ProxyPending
            },
            generate_proxy: !input.proxy_ready && !missing_original,
        };
    }
    if missing_original {
        return RepresentationChoice {
            representation: (input.moment != PlaybackMoment::Paused && input.proxy_ready)
                .then_some(Representation::Proxy),
            offline: if input.proxy_ready {
                MediaAvailability::OriginalOfflineProxyAvailable
            } else {
                MediaAvailability::OriginalOfflineNoProxy
            },
            generate_proxy: false,
        };
    }
    if input.moment == PlaybackMoment::Paused {
        return RepresentationChoice {
            representation: Some(Representation::Original),
            offline: MediaAvailability::Online,
            generate_proxy: false,
        };
    }
    if input.moment == PlaybackMoment::Continuing {
        return RepresentationChoice {
            representation: Some(input.current),
            offline: MediaAvailability::Online,
            generate_proxy: false,
        };
    }
    let original_cost = input.decode.original_p95_ms;
    let gpu = input.profile.gpu_frame_p95_ms;
    let budget = if input.fps.is_finite() && input.fps > 0.0 {
        1000.0 / input.fps
    } else {
        0.0
    };
    let power_margin = if input.profile.on_battery { 0.75 } else { 0.9 };
    let fits = |ms: f64| ms.is_finite() && ms >= 0.0 && ms + gpu <= budget * power_margin;
    let target_scale = match input.resolution {
        ResolutionIntent::Full => 1.0,
        ResolutionIntent::Half => 0.5,
        ResolutionIntent::Quarter => 0.25,
        ResolutionIntent::Automatic => {
            let (vw, vh) = input.viewer_size;
            let (sw, sh) = input.source_size;
            if sw == 0 || sh == 0 {
                1.0
            } else {
                (vw as f64 / sw as f64).max(vh as f64 / sh as f64)
            }
        }
    };
    let reduced_half = input.reduced_half_available
        && target_scale <= 0.5
        && input.decode.reduced_half_p95_ms.is_some_and(fits);
    let reduced_quarter = input.reduced_quarter_available
        && target_scale <= 0.25
        && input.decode.reduced_quarter_p95_ms.is_some_and(fits);
    let choice = if fits(original_cost) {
        Representation::Original
    } else if reduced_quarter {
        Representation::ReducedQuarter
    } else if reduced_half {
        Representation::ReducedHalf
    } else if input.proxy_ready && input.decode.proxy_p95_ms.is_some_and(fits) {
        Representation::Proxy
    } else {
        Representation::Original
    };
    RepresentationChoice {
        representation: Some(choice),
        offline: MediaAvailability::Online,
        generate_proxy: !input.proxy_ready
            && !fits(original_cost)
            && !reduced_half
            && !reduced_quarter,
    }
}

/// Analysis always reads originals. Normal exports do too; only an explicit
/// draft render may consume proxy pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaUse {
    Playback,
    Analysis,
    Export,
    DraftExport,
}

pub fn source_for(
    use_kind: MediaUse,
    choice: Representation,
    original_online: bool,
) -> Result<Representation> {
    match use_kind {
        MediaUse::Playback => Ok(choice),
        MediaUse::DraftExport if choice == Representation::Proxy => Ok(choice),
        MediaUse::DraftExport | MediaUse::Analysis | MediaUse::Export if original_online => {
            Ok(Representation::Original)
        }
        _ => Err(ProxyError::InvalidMedia("original media is offline".into())),
    }
}
