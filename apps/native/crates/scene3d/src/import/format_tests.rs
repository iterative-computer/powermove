use super::{ImportFile, Model, cad, load, load_with_cad, prepare, triangle_glb};
use std::io::{Cursor, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const TRIANGLE: [[f32; 3]; 3] = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
const ASCII_STL: &[u8] = b"solid test\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid test\n";

fn fixture(name: &str, bytes: &[u8]) -> ImportFile {
    ImportFile {
        name: name.into(),
        mime: String::new(),
        bytes: bytes.to_vec(),
    }
}

fn round_trip(name: &str, bytes: &[u8]) -> pm_geom3d::gltf::Model {
    let prepared =
        prepare(vec![fixture(name, bytes)], None).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(prepared.name.ends_with(".glb"), "{name}");
    assert_eq!(&prepared.bytes[..4], b"glTF");
    assert_eq!(prepared.original[0].bytes, bytes);
    let parsed = pm_geom3d::gltf::load(&prepared.bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(!parsed.geometries.is_empty(), "{name}");
    assert_eq!(parsed.geometries[0].vertex_count(), 3, "{name}");
    assert!(matches!(load(name, bytes), Ok(Model::Gltf(_))), "{name}");
    parsed
}

fn archive(files: &[(&str, &[u8])], options: SimpleFileOptions) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for &(name, bytes) in files {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn ascii_and_binary_stl_round_trip() {
    round_trip("triangle.STL", ASCII_STL);
    let mut bytes = vec![0; 80];
    bytes[..5].copy_from_slice(b"solid"); // Binary headers can also start with solid.
    bytes.extend(1_u32.to_le_bytes());
    bytes.extend([0; 12]);
    bytes.extend(TRIANGLE.into_iter().flatten().flat_map(f32::to_le_bytes));
    bytes.extend([0; 2]);
    round_trip("triangle.stl", &bytes);
    assert!(load("triangle.stl", &bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn ply_mesh_round_trips_in_all_three_encodings() {
    let header = |format| {
        format!("ply\r\nformat {format} 1.0\r\ncomment test\r\nelement vertex 3\r\nproperty float x\r\nproperty float y\r\nproperty float z\r\nelement face 1\r\nproperty list uchar int vertex_indices\r\nend_header\r\n").into_bytes()
    };
    let mut ascii = header("ascii");
    ascii.extend(b"0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n");
    round_trip("triangle.ply", &ascii);
    for big in [false, true] {
        let mut binary = header(if big {
            "binary_big_endian"
        } else {
            "binary_little_endian"
        });
        binary.extend(TRIANGLE.into_iter().flatten().flat_map(|v| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        }));
        binary.push(3);
        binary.extend([0, 1, 2].into_iter().flat_map(|v: i32| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        }));
        round_trip("triangle.ply", &binary);
        assert!(load("triangle.ply", &binary[..binary.len() - 1]).is_err());
    }
}

#[test]
fn gaussian_and_point_ply_remain_splats() {
    let mut p = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n".to_vec();
    p.extend([0; 12]);
    assert!(matches!(load("cloud.ply", &p), Ok(Model::Splats(_))));
    let mut p = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\n".to_vec();
    for field in [
        "x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "scale_0", "scale_1", "scale_2", "rot_0",
        "rot_1", "rot_2", "rot_3",
    ] {
        writeln!(&mut p, "property float {field}").unwrap();
    }
    p.extend(b"element face 1\nproperty list uchar int vertex_indices\nend_header\n");
    p.extend([0; 48]);
    // A nonzero first quaternion component is in rot_0.
    let field = p.len() - 12;
    p[field..field + 4].copy_from_slice(&1_f32.to_le_bytes());
    p.extend([0; 4]);
    p.extend([3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(matches!(load("cloud.ply", &p), Ok(Model::Splats(_))));
}

#[test]
fn off_round_trips_and_keeps_inline_counts() {
    round_trip(
        "triangle.off",
        b"OFF 3 1 0\n# positions\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n",
    );
    round_trip(
        "triangle.off",
        b"OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n",
    );
    assert!(load("invalid.off", b"OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 9\n").is_err());
}

#[test]
fn three_mf_reads_the_primary_part_and_material() {
    let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Target="/3D/triangle.model" Id="model" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
    </Relationships>"#;
    let model = br##"<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
        <resources>
            <basematerials id="2"><base name="Red" displaycolor="#FF0000"/></basematerials>
            <object id="1" pid="2" pindex="0" type="model"><mesh><vertices>
                <vertex x="0" y="0" z="0"/><vertex x="1000" y="0" z="0"/><vertex x="0" y="1000" z="0"/>
            </vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>
            <object id="3"><components><component objectid="1" transform="1 0 0 0 1 0 0 0 1 2000 0 0"/></components></object>
        </resources>
        <build><item objectid="3"/></build>
    </model>"##;
    let bytes = archive(
        &[("_rels/.rels", rels), ("3D/triangle.model", model)],
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    );
    let parsed = round_trip("triangle.3mf", &bytes);
    assert!(parsed.materials.iter().any(|m| m.name == "Red"));
    let bounds =
        pm_geom3d::gltf::pose::scene_box(&parsed, &pm_geom3d::gltf::pose::Pose::rest(&parsed))
            .unwrap();
    assert_eq!([bounds.0[0], bounds.1[0]], [2., 3.]);
}

#[test]
fn collada_reads_indexed_geometry_material_and_transforms() {
    let bytes = br##"<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
        <asset><unit meter="0.01"/><up_axis>Z_UP</up_axis></asset>
        <library_geometries><geometry id="g"><mesh>
            <source id="p"><float_array id="p-data" count="9">0 0 0 100 0 0 0 100 0</float_array>
                <technique_common><accessor source="#p-data" count="3" stride="3"/></technique_common></source>
            <vertices id="v"><input semantic="POSITION" source="#p"/></vertices>
            <polylist count="1" material="Blue"><input semantic="VERTEX" source="#v" offset="0"/><vcount>3</vcount><p>0 1 2</p></polylist>
        </mesh></geometry></library_geometries>
        <library_visual_scenes><visual_scene id="stage"><node id="item"><translate>200 0 0</translate>
            <instance_geometry url="#g"/>
        </node></visual_scene></library_visual_scenes>
        <scene><instance_visual_scene url="#stage"/></scene>
    </COLLADA>"##;
    let parsed = round_trip("triangle.dae", bytes);
    assert!(parsed.materials.iter().any(|m| m.name == "Blue"));
    let bounds =
        pm_geom3d::gltf::pose::scene_box(&parsed, &pm_geom3d::gltf::pose::Pose::rest(&parsed))
            .unwrap();
    assert_eq!([bounds.0[0], bounds.1[0]], [2., 3.]);
}

fn chunk(id: u16, body: &[u8]) -> Vec<u8> {
    [
        id.to_le_bytes().as_slice(),
        &((body.len() + 6) as u32).to_le_bytes(),
        body,
    ]
    .concat()
}

#[test]
fn three_ds_reads_nested_geometry() {
    let mut points = vec![3, 0];
    points.extend(TRIANGLE.into_iter().flatten().flat_map(f32::to_le_bytes));
    let vertices = chunk(0x4110, &points);
    let faces = chunk(0x4120, &[1, 0, 0, 0, 1, 0, 2, 0, 0, 0]);
    let object = chunk(0x4100, &[vertices, faces].concat());
    let object = chunk(0x4000, &[b"Triangle\0".as_slice(), &object].concat());
    let bytes = chunk(0x4d4d, &chunk(0x3d3d, &object));
    round_trip("triangle.3ds", &bytes);
    assert!(load("triangle.3ds", &bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn vrm_accepts_binary_and_embedded_json_gltf() {
    let bytes = triangle_glb(&TRIANGLE, &[0, 1, 2]);
    round_trip("triangle.vrm", &bytes);
    let size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut doc: serde_json::Value = serde_json::from_slice(&bytes[20..20 + size]).unwrap();
    use base64::Engine;
    let uri = base64::engine::general_purpose::STANDARD.encode(&bytes[28 + size..]);
    doc["buffers"][0]["uri"] = format!("data:application/octet-stream;base64,{uri}").into();
    round_trip("triangle.vrm", &serde_json::to_vec(&doc).unwrap());
}

#[test]
fn zip_selects_one_model_and_embeds_its_companions() {
    let imported = archive(
        &[("directory/triangle.stl", ASCII_STL)],
        SimpleFileOptions::default(),
    );
    round_trip("triangle.zip", &imported);
    let glb = triangle_glb(&TRIANGLE, &[0, 1, 2]);
    let size = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let mut doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + size]).unwrap();
    doc["buffers"][0]["uri"] = "sidecar.bin".into();
    let doc = serde_json::to_vec(&doc).unwrap();
    let imported = archive(
        &[
            ("models/triangle.gltf", &doc),
            ("models/sidecar.bin", &glb[28 + size..]),
        ],
        SimpleFileOptions::default(),
    );
    let prepared = prepare(vec![fixture("packed.zip", &imported)], None).unwrap();
    assert_eq!(prepared.name, "triangle.gltf");
    assert!(pm_geom3d::gltf::load(&prepared.bytes).is_ok());
}

#[test]
fn all_cad_extensions_use_the_supplied_tessellator() {
    struct Cad(cad::Format);
    impl cad::Tessellator for Cad {
        fn convert(
            &self,
            format: cad::Format,
            input: &[u8],
            settings: cad::Settings,
        ) -> crate::Result<Vec<u8>> {
            assert_eq!(format, self.0);
            assert_eq!(input, b"CAD fixture");
            assert_eq!(settings, cad::Settings::default());
            Ok(triangle_glb(&TRIANGLE, &[0, 1, 2]))
        }
    }
    for (ext, format) in [
        ("step", cad::Format::Step),
        ("stp", cad::Format::Step),
        ("iges", cad::Format::Iges),
        ("igs", cad::Format::Iges),
        ("brep", cad::Format::Brep),
    ] {
        let name = format!("triangle.{ext}");
        let prepared = prepare(vec![fixture(&name, b"CAD fixture")], Some(&Cad(format))).unwrap();
        assert_eq!(prepared.name, "triangle.glb");
        assert!(pm_geom3d::gltf::load(&prepared.bytes).is_ok());
        assert!(matches!(
            load_with_cad(&name, b"CAD fixture", Some(&Cad(format))),
            Ok(Model::Gltf(_))
        ));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn modelio_reads_usd_variants_usdz_and_alembic() {
    let ascii = include_bytes!("fixtures/triangle.usda");
    let binary = include_bytes!("fixtures/triangle.usdc");
    let alembic = include_bytes!("fixtures/triangle.abc");
    let usdz = archive(
        &[("triangle.usdc", binary)],
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .with_alignment(64),
    );
    round_trip("triangle.usda", ascii);
    round_trip("triangle.usd", binary);
    round_trip("triangle.usdc", binary);
    round_trip("triangle.usdz", &usdz);
    round_trip("triangle.abc", alembic);
}

/// Opt-in, read-only check against a local source that is not part of the repo.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires PM_TEST_USDZ pointing at a local USDZ"]
fn local_usdz_round_trip() {
    let source = std::fs::read(std::env::var_os("PM_TEST_USDZ").unwrap()).unwrap();
    assert!(matches!(load("local.usdz", &source), Ok(Model::Gltf(_))));
    let glb = prepare(vec![fixture("local.usdz", &source)], None)
        .unwrap()
        .bytes;
    let doc = pm_geom3d::model_import::json_of(&glb).unwrap();
    let len = |key: &str| doc[key].as_array().map_or(0, Vec::len);
    let mut positions = std::collections::HashSet::new();
    let mut triangles = 0;
    let mut primitives = 0;
    for mesh in doc["meshes"].as_array().unwrap() {
        for primitive in mesh["primitives"].as_array().unwrap() {
            positions.insert(primitive["attributes"]["POSITION"].as_u64().unwrap() as usize);
            triangles += doc["accessors"][primitive["indices"].as_u64().unwrap() as usize]["count"]
                .as_u64()
                .unwrap()
                / 3;
            primitives += 1;
        }
    }
    let vertices: u64 = positions
        .into_iter()
        .map(|i| doc["accessors"][i]["count"].as_u64().unwrap())
        .sum();
    let Model::Gltf(model) = super::load("converted.glb", &glb).unwrap() else {
        panic!("expected glTF model");
    };
    eprintln!(
        "bytes={} vertices={vertices} triangles={triangles} meshes={} primitives={primitives} nodes={} materials={} textures={} images={} loaded_drawables={}",
        glb.len(),
        len("meshes"),
        len("nodes"),
        len("materials"),
        len("textures"),
        len("images"),
        model.geometries.len(),
    );
    assert!(
        !model.images.is_empty(),
        "USDZ should contain textured materials"
    );
    for image in &model.images {
        let raster = super::texture::decode(&image.bytes).unwrap();
        assert!(raster.width().max(raster.height()) <= super::texture::MAX_EDGE);
    }
    if let Some(path) = std::env::var_os("PM_TEST_GLB_OUTPUT") {
        std::fs::write(path, glb).unwrap();
    }
}

#[test]
fn shared_mesh_compacts_material_parts_and_reuses_instances() {
    use super::mesh::{Mesh, Part, Scene};
    let mut scene = Scene::default();
    let red = scene.material("Red", [1., 0., 0., 1.]);
    let blue = scene.material("Blue", [0., 0., 1., 1.]);
    assert_eq!(red, scene.material("Red", [1., 0., 0., 1.]));
    let mesh = scene
        .partitioned_mesh(
            "Shared",
            Mesh {
                positions: vec![
                    [0., 0., 0.],
                    [1., 0., 0.],
                    [0., 1., 0.],
                    [1., 1., 0.],
                    [1000., 1000., 1000.],
                ],
                ..Mesh::default()
            },
            vec![
                Part {
                    indices: vec![0, 1, 2],
                    material: Some(red),
                },
                Part {
                    indices: vec![0, 1, 2],
                    material: Some(red),
                },
                Part {
                    indices: vec![1, 3, 2],
                    material: Some(blue),
                },
            ],
        )
        .unwrap();
    let a = scene.node("First", Some(mesh), None, vec![]).unwrap();
    let b = scene.node("Second", Some(mesh), None, vec![]).unwrap();
    let glb = scene.finish(vec![a, b]).unwrap();
    let doc = pm_geom3d::model_import::json_of(&glb).unwrap();
    assert_eq!(doc["nodes"][0]["mesh"], doc["nodes"][1]["mesh"]);
    assert_eq!(doc["meshes"].as_array().unwrap().len(), 1);
    assert_eq!(doc["meshes"][0]["primitives"].as_array().unwrap().len(), 2);
    let Model::Gltf(model) = load("parts.glb", &glb).unwrap() else {
        panic!("expected glTF")
    };
    assert_eq!(model.geometries.len(), 2);
    assert!(model.geometries.iter().all(|g| g.vertex_count() == 3));
}

#[test]
fn pbr_maps_preserve_alpha_channels_and_reuse_embedded_image_bytes() {
    use super::{
        mesh::{Alpha, Material, Mesh, Sampler, Scene},
        texture,
    };
    use image::{Rgba, RgbaImage};
    let solid = |rgba| RgbaImage::from_pixel(1, 1, Rgba(rgba));
    let mut scene = Scene::default();
    let map = texture::metallic_roughness(Some(solid([87; 4])), Some(solid([231; 4]))).unwrap();
    let orm = scene
        .texture(map.clone(), false, Sampler::default())
        .unwrap();
    assert_eq!(orm, scene.texture(map, false, Sampler::default()).unwrap());
    let base = scene
        .texture(
            texture::with_opacity(Some(solid([255, 128, 0, 200])), solid([128; 4])),
            true,
            Sampler::default(),
        )
        .unwrap();
    let material = scene.pbr_material(
        "PBR",
        Material {
            base_map: Some(base),
            metallic_roughness_map: Some(orm),
            metallic: 0.7,
            roughness: 0.5,
            normal_map: Some(orm),
            occlusion_map: Some(orm),
            emissive: [0.2, 0.3, 0.4],
            emissive_map: Some(base),
            double_sided: true,
            alpha: Alpha::Mask(0.27),
            ..Material::default()
        },
    );
    let mesh = scene
        .mesh(
            "Triangle",
            [Mesh {
                positions: TRIANGLE.to_vec(),
                indices: vec![0, 1, 2],
                uv: vec![[0., 0.], [1., 0.], [0., 1.]],
                material: Some(material),
                ..Mesh::default()
            }],
        )
        .unwrap();
    let root = scene.node("Triangle", Some(mesh), None, vec![]).unwrap();
    let glb = scene.finish(vec![root]).unwrap();
    let doc = pm_geom3d::model_import::json_of(&glb).unwrap();
    assert_eq!(doc["images"].as_array().unwrap().len(), 2);
    assert_eq!(doc["samplers"].as_array().unwrap().len(), 1);
    assert_eq!(doc["materials"][0]["alphaMode"], "MASK");
    assert_eq!(doc["materials"][0]["doubleSided"], true);
    let model = pm_geom3d::gltf::load(&glb).unwrap();
    assert_eq!(
        texture::decode(&model.images[0].bytes)
            .unwrap()
            .get_pixel(0, 0)
            .0,
        [255, 87, 231, 255]
    );
    assert_eq!(
        texture::decode(&model.images[1].bytes)
            .unwrap()
            .get_pixel(0, 0)
            .0,
        [255, 128, 0, 100]
    );
}

fn test_texture() -> Vec<u8> {
    use image::{Rgba, RgbaImage};
    super::texture::encode(
        RgbaImage::from_fn(4, 4, |_, y| {
            if y < 2 {
                Rgba([255, 0, 0, 240])
            } else {
                Rgba([0, 0, 255, 255])
            }
        }),
        false,
    )
    .unwrap()
    .1
}

#[test]
fn collada_texture_resolves_from_selected_files_and_nested_zip() {
    let xml = br##"<COLLADA>
        <library_images><image id="paper"><init_from>textures/paper.png</init_from></image></library_images>
        <library_effects><effect id="effect"><profile_COMMON>
          <newparam sid="surface"><surface type="2D"><init_from>paper</init_from></surface></newparam>
          <newparam sid="sampler"><sampler2D><source>surface</source></sampler2D></newparam>
          <technique sid="common"><lambert><diffuse><texture texture="sampler" texcoord="UV"/></diffuse></lambert></technique>
        </profile_COMMON></effect></library_effects>
        <library_materials><material id="mat"><instance_effect url="#effect"/></material></library_materials>
        <library_geometries><geometry id="g"><mesh>
          <source id="p"><float_array id="xyz" count="9">0 0 0 1 0 0 0 1 0</float_array>
            <technique_common><accessor source="#xyz" count="3" stride="3"/></technique_common></source>
          <source id="u"><float_array id="uv" count="6">0 0 1 0 0 1</float_array>
            <technique_common><accessor source="#uv" count="3" stride="2"/></technique_common></source>
          <vertices id="v"><input semantic="POSITION" source="#p"/></vertices>
          <triangles count="1" material="surface"><input semantic="VERTEX" source="#v" offset="0"/>
            <input semantic="TEXCOORD" source="#u" offset="1"/><p>0 0 1 1 2 2</p></triangles>
        </mesh></geometry></library_geometries>
        <library_visual_scenes><visual_scene id="scene"><node><instance_geometry url="#g"><bind_material>
          <technique_common><instance_material symbol="surface" target="#mat"/></technique_common>
        </bind_material></instance_geometry></node></visual_scene></library_visual_scenes>
        <scene><instance_visual_scene url="#scene"/></scene>
    </COLLADA>"##;
    let tex = test_texture();
    let bundle = archive(
        &[
            ("bundle/card.dae", xml),
            ("bundle/textures/paper.png", &tex),
        ],
        SimpleFileOptions::default(),
    );
    for files in [
        vec![fixture("card.dae", xml), fixture("paper.png", &tex)],
        vec![fixture("card.zip", &bundle)],
    ] {
        let glb = prepare(files, None).unwrap().bytes;
        let model = pm_geom3d::gltf::load(&glb).unwrap();
        assert_eq!(model.images.len(), 1);
        assert!(
            model
                .materials
                .iter()
                .any(|material| material.map.is_some())
        );
        assert!(model.geometries[0].uv[0].is_some());
    }
}

#[test]
fn three_mf_texture_group_keeps_corner_uvs() {
    let rels = br#"<Relationships><Relationship Type="urn:3dmodel/3dmodel" Target="/3D/model.model"/></Relationships>"#;
    let model = br#"<model xmlns:m="http://schemas.microsoft.com/3dmanufacturing/material/2015/02"><resources>
        <m:texture2d id="1" path="/3D/Textures/paper.png" tilestyleu="mirror"/>
        <m:texture2dgroup id="2" texid="1"><m:tex2coord u="0" v="0"/><m:tex2coord u="1" v="0"/><m:tex2coord u="0" v="1"/></m:texture2dgroup>
        <object id="3"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
          <triangles><triangle v1="0" v2="1" v3="2" pid="2" p1="0" p2="1" p3="2"/></triangles></mesh></object>
        </resources><build><item objectid="3"/></build></model>"#;
    let bytes = archive(
        &[
            ("_rels/.rels", rels),
            ("3D/model.model", model),
            ("3D/Textures/paper.png", &test_texture()),
        ],
        SimpleFileOptions::default(),
    );
    let model = round_trip("textured.3mf", &bytes);
    assert_eq!(model.images.len(), 1);
    assert_eq!(
        model.geometries[0].uv[0].as_ref().unwrap().data.as_slice(),
        &[0., 1., 1., 1., 0., 0.]
    );
}

#[test]
fn fbx_reads_a_selected_external_color_texture() {
    let source =
        include_str!("../../../../../../engine/crates/pm-import-fbx/fixtures/tiny-static.fbx");
    let source = source
        .replace(
            "Objects: {",
            r#"Objects: {
  Texture: 800, "Texture::Paper", "" {
    Type: "TextureVideoClip"
    Version: 202
    TextureName: "Texture::Paper"
    FileName: "paper.png"
    RelativeFilename: "textures/paper.png"
  }
"#,
        )
        .replace(
            "Connections: {",
            r#"Connections: {
  C: "OP",800,300,"DiffuseColor"
"#,
        );
    let prepared = prepare(
        vec![
            fixture("card.fbx", source.as_bytes()),
            fixture("paper.png", &test_texture()),
        ],
        None,
    )
    .unwrap();
    let model = pm_geom3d::gltf::load(&prepared.bytes).unwrap();
    assert_eq!(model.images.len(), 1);
    assert!(
        model
            .materials
            .iter()
            .any(|material| material.map.is_some())
    );
}

#[test]
fn three_ds_zip_reads_a_diffuse_texture_from_its_model_directory() {
    let mut points = vec![3, 0];
    points.extend(TRIANGLE.into_iter().flatten().flat_map(f32::to_le_bytes));
    let mut uv = vec![3, 0];
    uv.extend(
        [[0_f32, 0.], [1., 0.], [0., 1.]]
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes),
    );
    let mut faces = vec![1, 0, 0, 0, 1, 0, 2, 0, 0, 0];
    faces.extend(chunk(0x4130, b"Paper\0\x01\0\0\0"));
    let geometry = chunk(
        0x4100,
        &[
            chunk(0x4110, &points),
            chunk(0x4120, &faces),
            chunk(0x4140, &uv),
        ]
        .concat(),
    );
    let object = chunk(0x4000, &[b"Triangle\0".as_slice(), &geometry].concat());
    let material = chunk(
        0xafff,
        &[
            chunk(0xa000, b"Paper\0"),
            chunk(0xa200, &chunk(0xa300, b"paper.png\0")),
        ]
        .concat(),
    );
    let model = chunk(0x4d4d, &chunk(0x3d3d, &[material, object].concat()));
    let bytes = archive(
        &[
            ("models/triangle.3ds", &model),
            ("models/paper.png", &test_texture()),
        ],
        SimpleFileOptions::default(),
    );
    let model = round_trip("textured.zip", &bytes);
    assert_eq!(model.images.len(), 1);
    assert!(
        model
            .materials
            .iter()
            .any(|material| material.map.is_some())
    );
}

#[cfg(target_os = "macos")]
#[test]
fn usdz_preserves_top_image_row_and_flips_v_once() {
    let usda = br#"#usda 1.0
(
    defaultPrim = "Card"
    upAxis = "Y"
)
def Xform "Card" {
    def Mesh "Paper" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    ) {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        texCoord2f[] primvars:st = [(0, 0), (1, 0), (0, 1)] (interpolation = "vertex")
        uniform token subdivisionScheme = "none"
        rel material:binding = </Card/Ink>
    }
    def Material "Ink" {
        token outputs:surface.connect = </Card/Ink/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor.connect = </Card/Ink/Image.outputs:rgb>
            token outputs:surface
        }
        def Shader "Image" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            float2 inputs:st.connect = </Card/Ink/UV.outputs:result>
            float3 outputs:rgb
        }
        def Shader "UV" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st"
            float2 outputs:result
        }
    }
}
"#;
    let bytes = archive(
        &[("card.usda", usda), ("paper.png", &test_texture())],
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .with_alignment(64),
    );
    let model = round_trip("card.usdz", &bytes);
    let map = model
        .materials
        .iter()
        .find_map(|material| material.map.as_ref())
        .unwrap();
    let image =
        super::texture::decode(&model.images[model.textures[map.texture].image].bytes).unwrap();
    assert!(image.get_pixel(0, 0)[0] > 235);
    assert!(image.get_pixel(0, image.height() - 1)[2] > 235);
    let geometry = &model.geometries[0];
    let uv = geometry.uv[0].as_ref().unwrap();
    for (point, coord) in geometry
        .position
        .data
        .chunks_exact(3)
        .zip(uv.data.chunks_exact(2))
    {
        assert_eq!(coord[1], 1. - point[1]);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn usdz_prefers_authored_colors_second_uv_set_and_authored_wrap() {
    let usda = br#"#usda 1.0
(
    defaultPrim = "Deck"
    upAxis = "Y"
)
def Xform "Deck" {
    def Mesh "Key" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    ) {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        texCoord2f[] primvars:st = [(0, 0), (10, 0), (0, 10)] (interpolation = "vertex")
        texCoord2f[] primvars:st1 = [(0, 0), (1, 0), (0, 1)] (interpolation = "vertex")
        uniform token subdivisionScheme = "none"
        rel material:binding = </Deck/DarkKey>
    }
    def Mesh "Legend" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    ) {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0.1), (1, 0, 0.1), (0, 1, 0.1)]
        uniform token subdivisionScheme = "none"
        rel material:binding = </Deck/WhiteLegend>
    }
    def Mesh "Speaker" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    ) {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(2, 0, 0), (3, 0, 0), (2, 1, 0)]
        texCoord2f[] primvars:st = [(0.1, 0.1), (2.7, 0.1), (0.1, 22.2)] (interpolation = "vertex")
        uniform token subdivisionScheme = "none"
        rel material:binding = </Deck/Grille>
    }
    def Mesh "Panel" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    ) {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(4, 0, 0), (5, 0, 0), (4, 1, 0)]
        texCoord2f[] primvars:st = [(0, 0), (6, 0), (0, 6)] (interpolation = "vertex")
        uniform token subdivisionScheme = "none"
        rel material:binding = </Deck/Tiled>
    }
    def Material "Tiled" {
        token outputs:surface.connect = </Deck/Tiled/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor = (0, 0, 0)
            float inputs:opacity.connect = </Deck/Tiled/Mask.outputs:r>
            token outputs:surface
        }
        def Shader "Mask" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            token inputs:wrapS = "repeat"
            token inputs:wrapT = "repeat"
            float2 inputs:st.connect = </Deck/Tiled/UV0.outputs:result>
            float outputs:r
        }
        def Shader "UV0" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st"
            float2 outputs:result
        }
    }
    def Material "DarkKey" {
        token outputs:surface.connect = </Deck/DarkKey/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor = (0.0035, 0.0036, 0.0040)
            float inputs:metallic = 0.2
            float inputs:roughness = 0.55
            float inputs:occlusion.connect = </Deck/DarkKey/AO.outputs:r>
            token outputs:surface
        }
        def Shader "AO" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            float2 inputs:st.connect = </Deck/DarkKey/UV1.outputs:result>
            float outputs:r
        }
        def Shader "UV1" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st1"
            float2 outputs:result
        }
    }
    def Material "WhiteLegend" {
        token outputs:surface.connect = </Deck/WhiteLegend/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor = (0.8, 0.8, 0.8)
            color3f inputs:emissiveColor = (0.5, 0.5, 0.5)
            token outputs:surface
        }
    }
    def Material "Grille" {
        token outputs:surface.connect = </Deck/Grille/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor = (0, 0, 0)
            float inputs:opacity.connect = </Deck/Grille/Mask.outputs:r>
            token outputs:surface
        }
        def Shader "Mask" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            token inputs:wrapS = "clamp"
            token inputs:wrapT = "clamp"
            float2 inputs:st.connect = </Deck/Grille/UV0.outputs:result>
            float outputs:r
        }
        def Shader "UV0" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st"
            float2 outputs:result
        }
    }
}
"#;
    let bytes = archive(
        &[("deck.usda", usda), ("paper.png", &test_texture())],
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .with_alignment(64),
    );
    let prepared = prepare(vec![fixture("deck.usdz", &bytes)], None).unwrap();
    let doc = pm_geom3d::model_import::json_of(&prepared.bytes).unwrap();
    let materials = doc["materials"].as_array().unwrap();
    let find = |name: &str| {
        materials
            .iter()
            .find(|m| m["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("missing material {name}: {materials:?}"))
    };
    let dark = find("DarkKey");
    let dark_rgb = dark["pbrMetallicRoughness"]["baseColorFactor"]
        .as_array()
        .unwrap();
    assert!(
        (dark_rgb[0].as_f64().unwrap() - 0.0035).abs() < 1e-4,
        "DarkKey must keep authored diffuseColor, not 0.18 default: {dark_rgb:?}"
    );
    assert_eq!(dark["occlusionTexture"]["texCoord"].as_u64(), Some(1));

    let legend = find("WhiteLegend");
    let legend_rgb = legend["pbrMetallicRoughness"]["baseColorFactor"]
        .as_array()
        .unwrap();
    assert!(
        (legend_rgb[0].as_f64().unwrap() - 0.8).abs() < 1e-4,
        "WhiteLegend must keep authored diffuseColor: {legend_rgb:?}"
    );
    assert!(
        (legend["emissiveFactor"][0].as_f64().unwrap() - 0.5).abs() < 1e-4,
        "WhiteLegend must keep authored emissiveColor"
    );

    let grille = find("Grille");
    let grille_rgb = grille["pbrMetallicRoughness"]["baseColorFactor"]
        .as_array()
        .unwrap();
    assert!(
        grille_rgb[0].as_f64().unwrap() < 1e-4,
        "Grille must keep authored black diffuseColor: {grille_rgb:?}"
    );
    // Authored wrap modes stay (w83 review P0-2): the Grille's mask is
    // clamped even though its UVs tile, and an authored repeat stays repeat.
    let wraps = |material: &serde_json::Value| {
        let texture = material["pbrMetallicRoughness"]["baseColorTexture"]["index"]
            .as_u64()
            .unwrap() as usize;
        let sampler = &doc["samplers"][doc["textures"][texture]["sampler"].as_u64().unwrap() as usize];
        (sampler["wrapS"].as_u64(), sampler["wrapT"].as_u64())
    };
    assert_eq!(wraps(grille), (Some(33071), Some(33071)), "authored clamp across tiled UVs");
    assert_eq!(wraps(find("Tiled")), (Some(10497), Some(10497)), "authored repeat");

    let model = pm_geom3d::gltf::load(&prepared.bytes).unwrap();
    assert!(
        model.geometries.iter().any(|g| g.uv[1].is_some()),
        "Key mesh must preserve TEXCOORD_1"
    );
}

/// w83 review P0-3: two maps packed into one texture share one UV set. When
/// theirs differ, both use UV0 (as before second sets were read), so the
/// opacity and roughness maps authored on UV0 are not moved onto UV1; maps
/// that agree on UV1 keep it.
#[cfg(target_os = "macos")]
#[test]
fn usdz_packed_maps_never_move_onto_another_uv_set() {
    let usda = br#"#usda 1.0
(
    defaultPrim = "Deck"
    upAxis = "Y"
)
def Xform "Deck" {
    def Mesh "A" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    ) {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        texCoord2f[] primvars:st = [(0, 0), (1, 0), (0, 1)] (interpolation = "vertex")
        texCoord2f[] primvars:st1 = [(0, 1), (1, 1), (1, 0)] (interpolation = "vertex")
        uniform token subdivisionScheme = "none"
        rel material:binding = </Deck/MixedAlpha>
    }
    def Mesh "B" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    ) {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(2, 0, 0), (3, 0, 0), (2, 1, 0)]
        texCoord2f[] primvars:st = [(0, 0), (1, 0), (0, 1)] (interpolation = "vertex")
        texCoord2f[] primvars:st1 = [(0, 1), (1, 1), (1, 0)] (interpolation = "vertex")
        uniform token subdivisionScheme = "none"
        rel material:binding = </Deck/MixedPbr>
    }
    def Mesh "C" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    ) {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(4, 0, 0), (5, 0, 0), (4, 1, 0)]
        texCoord2f[] primvars:st = [(0, 0), (1, 0), (0, 1)] (interpolation = "vertex")
        texCoord2f[] primvars:st1 = [(0, 1), (1, 1), (1, 0)] (interpolation = "vertex")
        uniform token subdivisionScheme = "none"
        rel material:binding = </Deck/AgreedAlpha>
    }
    def Material "MixedAlpha" {
        token outputs:surface.connect = </Deck/MixedAlpha/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor.connect = </Deck/MixedAlpha/A.outputs:rgb>
            float inputs:opacity.connect = </Deck/MixedAlpha/B.outputs:r>
            token outputs:surface
        }
        def Shader "A" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            float2 inputs:st.connect = </Deck/MixedAlpha/UV1.outputs:result>
            float3 outputs:rgb
            float outputs:r
        }
        def Shader "B" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            float2 inputs:st.connect = </Deck/MixedAlpha/UV0.outputs:result>
            float outputs:r
        }
        def Shader "UV0" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st"
            float2 outputs:result
        }
        def Shader "UV1" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st1"
            float2 outputs:result
        }
    }
    def Material "MixedPbr" {
        token outputs:surface.connect = </Deck/MixedPbr/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            float inputs:metallic.connect = </Deck/MixedPbr/B.outputs:r>
            float inputs:roughness.connect = </Deck/MixedPbr/A.outputs:r>
            token outputs:surface
        }
        def Shader "A" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            float2 inputs:st.connect = </Deck/MixedPbr/UV1.outputs:result>
            float3 outputs:rgb
            float outputs:r
        }
        def Shader "B" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            float2 inputs:st.connect = </Deck/MixedPbr/UV0.outputs:result>
            float outputs:r
        }
        def Shader "UV0" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st"
            float2 outputs:result
        }
        def Shader "UV1" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st1"
            float2 outputs:result
        }
    }
    def Material "AgreedAlpha" {
        token outputs:surface.connect = </Deck/AgreedAlpha/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor.connect = </Deck/AgreedAlpha/A.outputs:rgb>
            float inputs:opacity.connect = </Deck/AgreedAlpha/B.outputs:r>
            token outputs:surface
        }
        def Shader "A" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            float2 inputs:st.connect = </Deck/AgreedAlpha/UV1.outputs:result>
            float3 outputs:rgb
            float outputs:r
        }
        def Shader "B" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paper.png@
            float2 inputs:st.connect = </Deck/AgreedAlpha/UV1.outputs:result>
            float outputs:r
        }
        def Shader "UV0" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st"
            float2 outputs:result
        }
        def Shader "UV1" {
            uniform token info:id = "UsdPrimvarReader_float2"
            token inputs:varname = "st1"
            float2 outputs:result
        }
    }
}
"#;
    let bytes = archive(
        &[("deck.usda", usda), ("paper.png", &test_texture())],
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .with_alignment(64),
    );
    let prepared = prepare(vec![fixture("deck.usdz", &bytes)], None).unwrap();
    let doc = pm_geom3d::model_import::json_of(&prepared.bytes).unwrap();
    let materials = doc["materials"].as_array().unwrap();
    let find = |name: &str| {
        materials
            .iter()
            .find(|m| m["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("missing material {name}: {materials:?}"))
    };
    let coord = |texture: &serde_json::Value| {
        assert!(texture.is_object(), "the map is imported");
        texture["texCoord"].as_u64().unwrap_or(0)
    };
    let pbr = |name: &str| find(name)["pbrMetallicRoughness"].clone();
    assert_eq!(coord(&pbr("MixedAlpha")["baseColorTexture"]), 0);
    assert_eq!(coord(&pbr("MixedPbr")["metallicRoughnessTexture"]), 0);
    assert_eq!(coord(&pbr("AgreedAlpha")["baseColorTexture"]), 1);
}

#[test]
fn gltf_selected_and_zip_companions_cap_large_images() {
    let mut writer = pm_geom3d::model_import::glb::GlbWriter::default();
    let position = writer.floats(&TRIANGLE, true);
    let uv = writer.floats(&[[0., 0.], [1., 0.], [0., 1.]], false);
    let glb = writer.finish(serde_json::json!({
        "asset":{"version":"2.0"}, "scene":0, "scenes":[{"nodes":[0]}], "nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":position,"TEXCOORD_0":uv},"material":0}]}],
        "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}],
        "textures":[{"source":0}], "images":[{"uri":"textures/wide.png"}],
    }).as_object().unwrap().clone());
    let size = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let mut doc = pm_geom3d::model_import::json_of(&glb).unwrap();
    doc["buffers"][0]["uri"] = "mesh.bin".into();
    let json = serde_json::to_vec(&doc).unwrap();
    let bin = &glb[28 + size..];
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4096, 128, image::Rgba([21, 87, 231, 240]))
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let zip = archive(
        &[
            ("model/card.gltf", &json),
            ("model/mesh.bin", bin),
            ("model/textures/wide.png", &png),
        ],
        SimpleFileOptions::default(),
    );
    let selected = vec![
        fixture("card.gltf", &json),
        fixture("mesh.bin", bin),
        fixture("wide.png", &png),
    ];
    for files in [selected, vec![fixture("card.zip", &zip)]] {
        let prepared = prepare(files, None).unwrap();
        let model = pm_geom3d::gltf::load(&prepared.bytes).unwrap();
        let image = super::texture::decode(&model.images[0].bytes).unwrap();
        assert_eq!(image.dimensions(), (2048, 64));
        assert_eq!(image.get_pixel(0, 0).0, [21, 87, 231, 240]);
    }
}
