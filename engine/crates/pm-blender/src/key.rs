//! The surface cache key (section 14.7, contract 11 invariant 6): snapshot
//! hash, worker-script hash, sorted asset hashes, Blender version and
//! settings, resolved device, preview flag, camera, size, layer, and shutter
//! sample. Equal keys are the same pixels (Cycles on a GPU is the one
//! nondeterministic case S7 measured, ±1 of 65,535; its device is in the
//! key, and a job never renders a surface twice).

use pm_protocol::blender::{BlenderSettings, ResolvedDevice, ShutterSample, SurfaceKey};
use pm_protocol::common::{ContentHash, LayerId, Size};

use crate::snapshot::Snapshot;

/// The key inputs of one layer's surface from `snapshot`.
pub fn surface_key(
    snapshot: &Snapshot,
    layer: &str,
    worker_script_hash: ContentHash,
    blender_version: &str,
    settings: &BlenderSettings,
    device: &ResolvedDevice,
) -> SurfaceKey {
    let (w, h) = snapshot.size();
    SurfaceKey {
        snapshot_hash: snapshot.hash(),
        worker_script_hash,
        asset_hashes: snapshot.assets.iter().map(|a| a.hash()).collect(),
        blender_version: blender_version.to_owned(),
        settings: settings.clone(),
        device: device.clone(),
        preview: snapshot.frame.preview,
        camera: snapshot.camera_hash(),
        size: Size::new(w, h),
        layer: LayerId::new(layer),
        shutter: snapshot
            .frame
            .shutter
            .map(|(index, count)| ShutterSample { index, count }),
    }
}

/// BLAKE3 of [`SurfaceKey::key_bytes`].
pub fn key_hash(key: &SurfaceKey) -> ContentHash {
    ContentHash(*blake3::hash(&key.key_bytes()).as_bytes())
}

/// Lowercase hex of a hash (file names).
pub fn hex(hash: &ContentHash) -> String {
    hash.0.iter().map(|b| format!("{b:02x}")).collect()
}

/// The device a job's surfaces are keyed under before Blender has run:
/// EEVEE renders on the GPU Blender itself picks, Cycles on the CPU when
/// asked for the CPU; anything else is resolved by Blender per job and
/// recorded ([`crate::cache::SurfaceCache::resolved_device`]).
pub fn predicted_device(settings: &BlenderSettings) -> Option<ResolvedDevice> {
    use pm_protocol::blender::{BlenderEngine, RequestedDevice};
    match (settings.engine, settings.device) {
        (BlenderEngine::Cycles, RequestedDevice::Cpu) => Some(ResolvedDevice::Cpu),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{AssetBytes, BlenderFrame, SnapshotAsset};
    use pm_protocol::blender::{BlenderEngine, RequestedDevice};

    fn snapshot(fov: f64, assets: Vec<SnapshotAsset>, shutter: Option<(u16, u16)>) -> Snapshot {
        Snapshot {
            frame: BlenderFrame {
                composition_path: vec!["root".into()],
                time: 1.0,
                width: 64,
                height: 32,
                shutter,
                preview: false,
            },
            json: pm_model::json::parse(&format!(
                r#"{{"width":64,"height":32,"objects":[{{"id":"a"}}],"camera":{{"fov":{fov}}}}}"#
            ))
            .unwrap(),
            assets,
        }
    }

    fn asset(id: &str, bytes: &[u8]) -> SnapshotAsset {
        SnapshotAsset::new(
            id.into(),
            format!("{id}.png"),
            AssetBytes::Bytes(std::sync::Arc::new(bytes.to_vec())),
        )
        .unwrap()
    }

    fn settings() -> BlenderSettings {
        BlenderSettings {
            engine: BlenderEngine::Eevee,
            samples: 16,
            preview: false,
            preview_scale: 0.5,
            denoise: true,
            device: RequestedDevice::Auto,
        }
    }

    #[test]
    fn every_listed_input_changes_the_key_and_asset_order_does_not() {
        let worker = ContentHash([7; 32]);
        let device = ResolvedDevice::Metal {
            name: "Apple M4 Max".into(),
        };
        let base = snapshot(45.0, vec![asset("x", b"1"), asset("y", b"2")], None);
        let k =
            |s: &Snapshot,
             layer: &str,
             w: ContentHash,
             v: &str,
             st: &BlenderSettings,
             d: &ResolvedDevice| { key_hash(&surface_key(s, layer, w, v, st, d)) };
        let reference = k(&base, "a", worker, "4.5.14", &settings(), &device);
        let reordered = snapshot(45.0, vec![asset("y", b"2"), asset("x", b"1")], None);
        // The snapshot's own bytes do not list assets, so reordering them
        // changes nothing.
        assert_eq!(
            reference,
            k(&reordered, "a", worker, "4.5.14", &settings(), &device)
        );
        let changed = [
            k(
                &snapshot(46.0, vec![asset("x", b"1"), asset("y", b"2")], None),
                "a",
                worker,
                "4.5.14",
                &settings(),
                &device,
            ),
            k(
                &snapshot(45.0, vec![asset("x", b"1"), asset("y", b"3")], None),
                "a",
                worker,
                "4.5.14",
                &settings(),
                &device,
            ),
            k(
                &snapshot(
                    45.0,
                    vec![asset("x", b"1"), asset("y", b"2")],
                    Some((1, 16)),
                ),
                "a",
                worker,
                "4.5.14",
                &settings(),
                &device,
            ),
            k(&base, "b", worker, "4.5.14", &settings(), &device),
            k(
                &base,
                "a",
                ContentHash([8; 32]),
                "4.5.14",
                &settings(),
                &device,
            ),
            k(&base, "a", worker, "4.5.15", &settings(), &device),
            k(
                &base,
                "a",
                worker,
                "4.5.14",
                &BlenderSettings {
                    samples: 17,
                    ..settings()
                },
                &device,
            ),
            k(
                &base,
                "a",
                worker,
                "4.5.14",
                &settings(),
                &ResolvedDevice::Cpu,
            ),
        ];
        for (i, c) in changed.iter().enumerate() {
            assert_ne!(reference, *c, "input {i}");
        }
        let mut preview = base.clone();
        preview.frame.preview = true;
        assert_ne!(
            reference,
            k(&preview, "a", worker, "4.5.14", &settings(), &device)
        );
    }

    #[test]
    fn only_cpu_cycles_is_known_before_blender_runs() {
        assert_eq!(predicted_device(&settings()), None);
        let cpu = BlenderSettings {
            engine: BlenderEngine::Cycles,
            device: RequestedDevice::Cpu,
            ..settings()
        };
        assert_eq!(predicted_device(&cpu), Some(ResolvedDevice::Cpu));
    }
}
