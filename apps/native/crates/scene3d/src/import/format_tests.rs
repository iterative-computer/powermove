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
