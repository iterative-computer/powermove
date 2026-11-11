use pm_scene3d::{
    geometry::{self, AssetGeometry, LoadedAssets},
    import::{
        self, ImportFile,
        cad::{self, Tessellator},
        splats,
    },
    navigation::Bounds,
};
use serde_json::json;
fn triangle() -> Vec<u8> {
    import::triangle_glb(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[0, 1, 2])
}
#[test]
fn self_contained_glb_loads_and_original_bytes_are_retained() {
    let bytes = triangle();
    let p = import::prepare(
        vec![ImportFile {
            name: "model.glb".into(),
            mime: "model/gltf-binary".into(),
            bytes: bytes.clone(),
        }],
        None,
    )
    .unwrap();
    assert_eq!(p.bytes, bytes);
    assert_eq!(p.original[0].bytes, bytes);
    let import::Model::Gltf(m) = import::load(&p.name, &p.bytes).unwrap() else {
        panic!()
    };
    assert_eq!(m.triangles, 1);
}

#[test]
fn fbx_import_keeps_the_source_and_uses_the_shared_glb_converter() {
    let original =
        include_bytes!("../../../../../engine/crates/pm-import-fbx/fixtures/tiny-static.fbx");
    let prepared = import::prepare(
        vec![ImportFile {
            name: "product.FBX".into(),
            mime: "model/fbx".into(),
            bytes: original.to_vec(),
        }],
        None,
    )
    .unwrap();
    assert_eq!(prepared.name, "product.glb");
    assert_eq!(prepared.original[0].bytes, original);
    assert_eq!(&prepared.bytes[..4], b"glTF");
    let import::Model::Gltf(model) = import::load(&prepared.name, &prepared.bytes).unwrap() else {
        panic!("converted FBX")
    };
    assert_eq!(model.triangles, 1);
    assert!(!model.materials.is_empty());
    assert_eq!(model.lights.len(), 1);
}

#[test]
fn persisted_fbx_assets_keep_skins_and_baked_animation() {
    let original =
        include_bytes!("../../../../../engine/crates/pm-import-fbx/fixtures/tiny-animated.fbx");
    let import::Model::Gltf(model) = import::load("animation.fbx", original).unwrap() else {
        panic!("FBX asset")
    };
    assert_eq!(model.triangles, 1);
    assert_eq!(model.skins.len(), 1);
    assert_eq!(model.clips.len(), 1);
}

#[test]
fn malformed_fbx_and_multiple_models_are_rejected() {
    assert!(import::load("bad.fbx", b"not an FBX").is_err());
    let original =
        include_bytes!("../../../../../engine/crates/pm-import-fbx/fixtures/tiny-static.fbx");
    let file = |name: &str| ImportFile {
        name: name.into(),
        mime: "model/fbx".into(),
        bytes: original.to_vec(),
    };
    assert!(import::prepare(vec![file("first.fbx"), file("second.fbx")], None).is_err());
}
#[test]
fn rejects_external_gltf_dependencies_without_network() {
    let v = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"https://example.invalid/geometry.bin","byteLength":36}]});
    assert!(import::load("model.gltf", v.to_string().as_bytes()).is_err());
}
#[test]
fn rejects_malformed_containers_and_blender() {
    let mut b = triangle();
    b[8..12].copy_from_slice(&0u32.to_le_bytes());
    assert!(import::load("model.glb", &b).is_err());
    assert!(import::load("model.blend", b"BLENDER").is_err());
}
#[test]
fn obj_bundle_retains_materials_and_selected_sources() {
    let files = vec![
        ImportFile {
            name: "model.obj".into(),
            mime: "".into(),
            bytes: b"mtllib model.mtl\no Body\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl Finish\nf 1 2 3\n"
                .to_vec(),
        },
        ImportFile {
            name: "model.mtl".into(),
            mime: "".into(),
            bytes: b"newmtl Finish\nKd 0.2 0.4 0.8\nNs 30\n".to_vec(),
        },
    ];
    let p = import::prepare(files, None).unwrap();
    assert_eq!(p.name, "model.glb");
    assert_eq!(p.original.len(), 2);
    let import::Model::Gltf(m) = import::load(&p.name, &p.bytes).unwrap() else {
        panic!()
    };
    assert_eq!(m.triangles, 1);
    assert!(!m.materials.is_empty());
}
#[test]
fn obj_texture_references_must_be_explicitly_selected() {
    let files = vec![
        ImportFile {
            name: "model.obj".into(),
            mime: "".into(),
            bytes: b"mtllib model.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl F\nf 1 2 3".to_vec(),
        },
        ImportFile {
            name: "model.mtl".into(),
            mime: "".into(),
            bytes: b"newmtl F\nmap_Kd https://example.invalid/texture.png".to_vec(),
        },
    ];
    assert!(import::prepare(files, None).is_err());
}
#[test]
fn duplicate_selected_names_and_multiple_models_are_refused() {
    for names in [["a.obj", "a.OBJ"], ["a.obj", "b.obj"]] {
        assert!(
            import::prepare(
                names
                    .into_iter()
                    .map(|name| ImportFile {
                        name: name.into(),
                        mime: "".into(),
                        bytes: vec![]
                    })
                    .collect(),
                None
            )
            .is_err()
        );
    }
}
struct CadDouble;
impl Tessellator for CadDouble {
    fn convert(
        &self,
        _: cad::Format,
        bytes: &[u8],
        settings: cad::Settings,
    ) -> pm_scene3d::Result<Vec<u8>> {
        assert_eq!(bytes, b"synthetic");
        assert_eq!(
            settings,
            cad::Settings {
                linear_deflection: 0.0006,
                angular_deflection: 0.25
            }
        );
        Ok(triangle())
    }
}
#[test]
fn native_cad_contract_never_needs_js() {
    let files = || {
        vec![ImportFile {
            name: "model.step".into(),
            mime: "".into(),
            bytes: b"synthetic".to_vec(),
        }]
    };
    assert!(import::prepare(files(), None).is_err());
    let p = import::prepare(files(), Some(&CadDouble)).unwrap();
    assert_eq!(p.name, "model.glb");
    assert_eq!(p.original[0].bytes, b"synthetic");
}
fn splat(position: [f32; 3], scale: [f32; 3]) -> Vec<u8> {
    let mut b = position
        .into_iter()
        .chain(scale)
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    b.extend([255, 128, 0, 200, 255, 128, 128, 128]);
    b
}
#[test]
fn splat_decodes_covariance_color_and_stable_order() {
    let mut b = splat([0., 0., -1.], [1., 2., 3.]);
    b.extend(splat([0., 0., -5.], [1., 1., 1.]));
    let d = splats::parse_splat(&b).unwrap();
    assert_eq!(d.splats[0].covariance, [1., 0., 0., 4., 0., 9.]);
    assert_eq!(d.splats[0].rgba, [255, 128, 0, 200]);
    let mut m = [0.; 16];
    for i in [0, 5, 10, 15] {
        m[i] = 1.;
    }
    assert_eq!(d.order(m).unwrap(), vec![1, 0]);
}
#[test]
fn splat_rejects_truncation_nonfinite_and_negative_scale() {
    assert!(splats::parse_splat(&[0; 31]).is_err());
    assert!(splats::parse_splat(&[]).is_err());
    assert!(splats::parse_splat(&splat([f32::NAN, 0., 0.], [1.; 3])).is_err());
    assert!(splats::parse_splat(&splat([0.; 3], [-1., 1., 1.])).is_err());
}
fn ply(crlf: bool) -> Vec<u8> {
    let newline = if crlf { "\r\n" } else { "\n" };
    let header = [
        "ply",
        "format binary_little_endian 1.0",
        "element vertex 2",
        "property float x",
        "property float y",
        "property float z",
        "property uchar red",
        "property uchar green",
        "property uchar blue",
        "end_header",
        "",
    ]
    .join(newline);
    let mut b = header.into_bytes();
    for p in [[0f32, 0., 0.], [1., 2., 3.]] {
        b.extend(p.into_iter().flat_map(f32::to_le_bytes));
        b.extend([10, 20, 30]);
    }
    b
}
#[test]
fn binary_ply_accepts_lf_and_crlf() {
    for crlf in [false, true] {
        let d = splats::parse_ply(&ply(crlf)).unwrap();
        assert_eq!(d.splats.len(), 2);
        assert_eq!(d.splats[1].center, [1., 2., 3.]);
        assert_eq!(d.splats[1].rgba, [10, 20, 30, 255]);
    }
}
#[test]
fn ply_rejects_truncation_lists_and_huge_declared_count() {
    let mut b = ply(false);
    b.pop();
    assert!(splats::parse_ply(&b).is_err());
    for text in [
        "ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty list uchar float points\nend_header\n",
        "ply\nformat binary_little_endian 1.0\nelement vertex 16000001\nend_header\n",
    ] {
        assert!(splats::parse_ply(text.as_bytes()).is_err());
    }
}
#[test]
fn binary_gaussian_ply_decodes_sh_log_scales_and_opacity() {
    let mut b = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\n".to_vec();
    let names = [
        "x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "scale_2",
        "rot_0", "rot_1", "rot_2", "rot_3",
    ];
    for n in names {
        b.extend(format!("property float {n}\n").bytes());
    }
    b.extend(b"end_header\n");
    for n in [1f32, 2., 3., 0., 0., 0., 0., 0., 0., 0., 1., 0., 0., 0.] {
        b.extend(n.to_le_bytes());
    }
    let d = splats::parse_ply(&b).unwrap();
    assert_eq!(d.splats[0].rgba, [128; 4]);
    assert_eq!(d.splats[0].covariance, [1., 0., 0., 1., 0., 1.]);
}
#[test]
fn primitive_bounds_use_generated_geometry_including_float_indices() {
    let source = json!({"mesh":{"positions":[0,0,0,2,0,0,0,4,0],"indices":[0.0,1.0,2.0]}});
    let b = geometry::local_bounds(&source, &geometry::NoAssets, &json!({}), 0.).unwrap();
    assert_eq!(
        b,
        Bounds {
            min: [0.; 3],
            max: [2., 4., 0.]
        }
    );
}
#[test]
fn loaded_asset_bounds_refuse_stale_source_metadata() {
    let mut registry = LoadedAssets::default();
    let record = json!({"revision":"one"});
    registry.insert(
        "model".into(),
        record.clone(),
        import::load("model.glb", &triangle()).unwrap(),
    );
    let b = registry.bounds("model", &record, 0.).unwrap();
    assert!(b.radius() > 0.);
    assert!(
        registry
            .bounds("model", &json!({"revision":"two"}), 0.)
            .is_err()
    );
    assert!(registry.bounds("missing", &record, 0.).is_err());
}
#[test]
fn archive_never_overwrites_conflicting_content_and_rejects_path_injection() {
    use pm_scene3d::import::archive::Archive;
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "pm-scene-archive-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _temp = Temp(root.clone());
    let archive = Archive::open(&root).unwrap();
    let prepare = || {
        import::prepare(
            vec![ImportFile {
                name: "model.glb".into(),
                mime: "model/gltf-binary".into(),
                bytes: triangle(),
            }],
            None,
        )
        .unwrap()
    };
    let receipt = archive.admit(prepare()).unwrap();
    let retained = receipt.primary().path();
    std::fs::write(retained, b"conflicting bytes").unwrap();
    assert!(archive.admit(prepare()).is_err());
    assert_eq!(std::fs::read(retained).unwrap(), b"conflicting bytes");
    assert!(archive.recover(&json!({"name":"model.glb","revision":"../../outside","size":1,"native3dSources":[]})).is_err());
}
#[test]
fn malformed_gltf_indices_are_refused_instead_of_blank_webgl_draws() {
    let bytes = import::triangle_glb(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[0, 1, 99]);
    assert!(import::load("bad.glb", &bytes).is_err());
}

#[test]
fn splat_framing_uses_dense_core_without_changing_outlier_bytes() {
    let mut bytes = Vec::new();
    for i in 0..100 {
        bytes.extend(splat(
            [if i == 99 { 1_000_000. } else { i as f32 }, 0., 0.],
            [1.; 3],
        ));
    }
    let model = splats::parse_splat(&bytes).unwrap();
    let core = model.dense_core().unwrap();
    assert_eq!(core.min, [2., 0., 0.]);
    assert_eq!(core.max, [98., 0., 0.]);
    assert_eq!(model.normalization().unwrap(), ([50., 0., 0.], 48.));
    let mut registry = LoadedAssets::default();
    registry.insert("a".into(), json!({}), import::Model::Splats(model.clone()));
    let bounds = registry.bounds("a", &json!({}), 0.).unwrap();
    assert_eq!(bounds.min, [-1., 0., 0.]);
    assert_eq!(bounds.max, [1., 0., 0.]);
    assert_eq!(model.splats[99].center[0], 1_000_000.);
    assert!(splats::Data { splats: vec![] }.normalization().is_err());
}
#[test]
fn a_live_model_resource_is_shared_and_a_dropped_one_reloads() {
    use pm_scene3d::render::Resource;
    use std::sync::Arc;
    let bytes = triangle();
    let other = import::triangle_glb(&[[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]], &[0, 1, 2]);
    let loads = [bytes.clone(), other.clone(), bytes.clone()]
        .map(|b| std::thread::spawn(move || Resource::load("model.glb", &b).unwrap()));
    let [a, b, c] = loads.map(|t| t.join().unwrap());
    assert!(Arc::ptr_eq(&a, &c));
    assert!(!Arc::ptr_eq(&a, &b));
    let weak = Arc::downgrade(&a);
    drop((a, c));
    assert!(weak.upgrade().is_none());
    assert!(Resource::load("model.glb", &bytes).is_ok());
}
