use super::{
    archive::Zip,
    mesh::{Mesh, Scene, number, srgb},
    xml::{self, child, children},
};
use crate::{Error, Result, fail};
use pm_geom3d::gltf::math::{IDENTITY, Mat4, multiply, scale};
use roxmltree::Node;
use std::collections::{HashMap, HashSet};

pub fn convert(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut zip = Zip::open(bytes)?;
    let relations = zip.read_named("_rels/.rels", 1024 * 1024)?;
    let relations = xml::parse(&relations)?;
    let target = children(relations.root_element(), "Relationship")
        .find(|n| {
            n.attribute("Type").is_some_and(|v| v.ends_with("/3dmodel"))
                && n.attribute("TargetMode")
                    .is_none_or(|mode| mode == "Internal")
        })
        .and_then(|n| n.attribute("Target"))
        .ok_or_else(|| Error("3MF package has no primary model relationship".into()))?;
    let target = super::path::relative_reference(target.strip_prefix('/').unwrap_or(target))?;
    let model = zip.read_named(&target, 128 * 1024 * 1024)?;
    let model = xml::parse(&model)?;
    let model = model.root_element();
    if !model.has_tag_name("model") {
        return fail("Invalid 3MF model element");
    }
    let metres = match model.attribute("unit").unwrap_or("millimeter") {
        "micron" => 1e-6,
        "millimeter" => 0.001,
        "centimeter" => 0.01,
        "inch" => 0.0254,
        "foot" => 0.3048,
        "meter" => 1.,
        _ => return fail("Unknown 3MF measurement unit"),
    };
    let resources = child(model, "resources")?;
    let mut importer = Importer::default();
    for entry in resources.children().filter(Node::is_element) {
        match entry.tag_name().name() {
            "object" => {
                let id = number(entry.attribute("id"))?;
                if importer.objects.insert(id, entry).is_some() {
                    return fail("Duplicate 3MF object ID");
                }
            }
            "basematerials" => {
                let id = number(entry.attribute("id"))?;
                for (i, base) in children(entry, "base").enumerate() {
                    let rgba = base
                        .attribute("displaycolor")
                        .and_then(parse_color)
                        .ok_or_else(|| Error("Invalid 3MF material color".into()))?;
                    let index = importer
                        .scene
                        .material(base.attribute("name").unwrap_or("3MF material"), rgba);
                    importer.materials.insert((id, i), index);
                }
            }
            _ => {}
        }
    }
    let mut nodes = Vec::new();
    for item in children(child(model, "build")?, "item") {
        nodes.push(importer.instance(item, &mut HashSet::new())?);
    }
    let z_up = [
        1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.,
    ];
    let root = importer.scene.node(
        "3MF model",
        None,
        Some(multiply(&z_up, &scale(metres))),
        nodes,
    )?;
    importer.scene.finish(vec![root])
}

#[derive(Default)]
struct Importer<'a, 'input> {
    scene: Scene,
    objects: HashMap<u32, Node<'a, 'input>>,
    meshes: HashMap<u32, usize>,
    materials: HashMap<(u32, usize), usize>,
}

impl Importer<'_, '_> {
    fn instance(&mut self, reference: Node<'_, '_>, ancestors: &mut HashSet<u32>) -> Result<usize> {
        if reference.attributes().any(|a| a.name() == "path") {
            return fail("3MF model parts outside the primary model are unsupported");
        }
        let id = number(reference.attribute("objectid"))?;
        let object = *self
            .objects
            .get(&id)
            .ok_or_else(|| Error("3MF object is missing".into()))?;
        if ancestors.len() >= 64 || !ancestors.insert(id) {
            return fail("Invalid or overly deep 3MF component hierarchy");
        }
        let name = object.attribute("name").unwrap_or("3MF object");
        let matrix = reference
            .attribute("transform")
            .map(transform)
            .transpose()?;
        let shape = object
            .children()
            .find(|n| n.has_tag_name("mesh") || n.has_tag_name("components"))
            .ok_or_else(|| Error("3MF object has no mesh or components".into()))?;
        let mut nodes = Vec::new();
        let mesh = match shape.tag_name().name() {
            "mesh" => {
                if let Some(&index) = self.meshes.get(&id) {
                    Some(index)
                } else {
                    let index = self.mesh(object, shape, name)?;
                    self.meshes.insert(id, index);
                    Some(index)
                }
            }
            "components" => {
                for component in children(shape, "component") {
                    nodes.push(self.instance(component, ancestors)?);
                }
                None
            }
            _ => return fail("3MF object has no supported geometry"),
        };
        ancestors.remove(&id);
        self.scene.node(name, mesh, matrix, nodes)
    }

    fn mesh(&mut self, object: Node<'_, '_>, mesh: Node<'_, '_>, name: &str) -> Result<usize> {
        let positions = children(child(mesh, "vertices")?, "vertex")
            .map(|v| {
                Ok([
                    number(v.attribute("x"))?,
                    number(v.attribute("y"))?,
                    number(v.attribute("z"))?,
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        let mut parts = std::collections::BTreeMap::<_, (Mesh, HashMap<u32, u32>)>::new();
        for tri in children(child(mesh, "triangles")?, "triangle") {
            let material = tri
                .attribute("pid")
                .or_else(|| object.attribute("pid"))
                .and_then(|id| id.parse().ok())
                .zip(
                    tri.attribute("p1")
                        .or_else(|| object.attribute("pindex"))
                        .and_then(|i| i.parse().ok()),
                )
                .and_then(|key| self.materials.get(&key).copied());
            let (part, remap) = parts.entry(material).or_default();
            part.material = material;
            for attribute in ["v1", "v2", "v3"] {
                let i: u32 = number(tri.attribute(attribute))?;
                let p = *positions
                    .get(i as usize)
                    .ok_or_else(|| Error("Invalid 3MF vertex index".into()))?;
                let index = *remap.entry(i).or_insert_with(|| {
                    let next = part.positions.len() as u32;
                    part.positions.push(p);
                    next
                });
                part.indices.push(index);
            }
        }
        self.scene
            .mesh(name, parts.into_values().map(|(part, _)| part))
    }
}

fn transform(value: &str) -> Result<Mat4> {
    let mut words = value.split_whitespace();
    let mut matrix = IDENTITY;
    for (i, slot) in matrix.iter_mut().enumerate() {
        if i % 4 != 3 {
            *slot = number(words.next())?;
        }
    }
    if words.next().is_some() {
        return fail("Invalid 3MF transform");
    }
    Ok(matrix)
}

fn parse_color(value: &str) -> Option<[f32; 4]> {
    let text = value.strip_prefix('#')?;
    if !matches!(text.len(), 6 | 8) || !text.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut color = [1.; 4];
    for (i, channel) in text.as_bytes().chunks_exact(2).enumerate() {
        let s = std::str::from_utf8(channel).ok()?;
        let value = u8::from_str_radix(s, 16).ok()? as f32 / 255.;
        color[i] = if i < 3 { srgb(value) } else { value };
    }
    Some(color)
}
