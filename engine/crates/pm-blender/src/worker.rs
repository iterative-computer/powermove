//! The pinned worker script (section 17, contract 11 invariants 1 and 2).
//!
//! `worker/worker.py` is today's worker (`apps/desktop/src/main/blender-
//! worker.ts`) with the output pinned: 16-bit RGBA PNG, AgX with look None,
//! sRGB display, gamma 1, dither 0, image color management following the
//! scene, compression 15 (S7's "fast compression"), and metadata stamps off
//! (S7 defect 7: date and render-time text chunks made files differ). It
//! renders the layers a request lists in `outputs` (one invocation per
//! layer, or every layer of a frame when batching) and records the device
//! it rendered on. Shading, framing, and texture orientation are today's
//! (appendix A items 38 and 39 stay at `compat` level 1).
//!
//! Electron main ships the same file (`main/engine/blender.ts` imports it),
//! and both sides hash it with SHA-256; the hash is in every surface key and
//! host services refuse a request whose hash is not their script's.

use pm_protocol::blender::ResolvedDevice;
use pm_protocol::common::ContentHash;

/// The worker script.
pub const WORKER_SCRIPT: &str = include_str!("../worker/worker.py");

/// SHA-256 of [`WORKER_SCRIPT`].
pub fn worker_script_hash() -> ContentHash {
    use sha2::{Digest, Sha256};
    ContentHash(Sha256::digest(WORKER_SCRIPT.as_bytes()).into())
}

/// The command-line flags after the executable: contract 11's
/// [`pm_protocol::blender::WORKER_FLAGS`], a Python failure as the exit
/// status, then the script.
pub fn worker_args(script: &std::path::Path) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = pm_protocol::blender::WORKER_FLAGS
        .iter()
        .map(Into::into)
        .collect();
    args.extend(["--python-exit-code".into(), "1".into(), "--python".into()]);
    args.push(script.as_os_str().to_owned());
    args
}

/// The device a manifest's `resolvedDevice` names (`{kind, name}`).
pub fn resolved_device(manifest: &serde_json::Value) -> Option<ResolvedDevice> {
    let d = manifest.get("resolvedDevice")?;
    let kind = d.get("kind")?.as_str()?;
    let name = d.get("name").and_then(|n| n.as_str()).unwrap_or("").to_owned();
    Some(match kind {
        "cpu" => ResolvedDevice::Cpu,
        "metal" => ResolvedDevice::Metal { name },
        "optix" => ResolvedDevice::Optix { name },
        "cuda" => ResolvedDevice::Cuda { name },
        "hip" => ResolvedDevice::Hip { name },
        "oneapi" => ResolvedDevice::OneApi { name },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_pins_section_17s_output() {
        let s = WORKER_SCRIPT;
        for line in [
            "image.color_depth='16'",
            "image.compression=15",
            "image.color_management='FOLLOW_SCENE'",
            "scene.render.dither_intensity=0",
            "scene.display_settings.display_device='sRGB'",
            "scene.view_settings.view_transform='AgX';scene.view_settings.look='None';scene.view_settings.gamma=1",
            "'use_stamp_date'",
            "'use_stamp_render_time'",
        ] {
            assert!(s.contains(line), "{line}");
        }
        assert_eq!(
            pm_protocol::blender::PINNED_OUTPUT.view_transform,
            "AgX",
            "contract 11 pins AgX"
        );
        // No shell, no subprocess, no network from the worker: it imports
        // only Blender's modules and the standard library's basics.
        for line in s.lines().filter(|l| l.trim_start().starts_with("import ") || l.contains(" import ")) {
            for banned in ["subprocess", "urllib", "socket", "http", "ctypes"] {
                assert!(!line.contains(banned), "{line}");
            }
        }
        assert!(!s.contains("os.system") && !s.contains("exec(") && !s.contains("eval("));
        assert_ne!(worker_script_hash(), ContentHash([0; 32]));
    }

    #[test]
    fn flags_are_contract_11s() {
        let args = worker_args(std::path::Path::new("/tmp/w.py"));
        let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            [
                "--background",
                "--factory-startup",
                "--disable-autoexec",
                "--offline-mode",
                "--python-exit-code",
                "1",
                "--python",
                "/tmp/w.py"
            ]
        );
    }

    #[test]
    fn manifests_name_their_device() {
        let m = serde_json::json!({"resolvedDevice": {"kind": "metal", "name": "Apple M4 Max"}});
        assert_eq!(
            resolved_device(&m),
            Some(ResolvedDevice::Metal {
                name: "Apple M4 Max".into()
            })
        );
        assert_eq!(
            resolved_device(&serde_json::json!({"resolvedDevice": {"kind": "cpu"}})),
            Some(ResolvedDevice::Cpu)
        );
        assert_eq!(resolved_device(&serde_json::json!({})), None);
    }
}
