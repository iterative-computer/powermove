use super::{
    ResolveCompanion,
    mesh::{self, Alpha, MAX_FACE_VERTICES, MAX_VERTICES, Material, Mesh, Sampler, Scene, number},
    texture,
    xml::{self, child, children, reference},
};
use crate::{Error, Result, fail};
use pm_geom3d::gltf::math::{IDENTITY, Mat4, compose, multiply, scale, translation};
use roxmltree::Node;
use std::collections::HashMap;

type Nodes<'a, 'input> = HashMap<&'a str, Node<'a, 'input>>;

pub fn convert(bytes: &[u8], resolve: &mut ResolveCompanion<'_>) -> Result<Vec<u8>> {
    let doc = xml::parse(bytes)?;
    let root = doc.root_element();
    if !root.has_tag_name("COLLADA") {
        return fail("Invalid COLLADA root element");
    }
    let meta = children(root, "asset").next();
    let unit = meta
        .and_then(|a| children(a, "unit").next())
        .and_then(|u| u.attribute("meter"))
        .map(|v| number::<f64>(Some(v)))
        .transpose()?
        .unwrap_or(1.);
    if !unit.is_finite() || unit <= 0. {
        return fail("Invalid COLLADA unit");
    }
    let up = match meta
        .and_then(|a| children(a, "up_axis").next())
        .and_then(|n| n.text())
        .map(str::trim)
    {
        Some("Z_UP") => [
            1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.,
        ],
        Some("X_UP") => [
            0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ],
        _ => IDENTITY,
    };
    let ids: Nodes<'_, '_> = root
        .descendants()
        .filter_map(|n| Some((n.attribute("id")?, n)))
        .collect();
    let mut importer = Importer {
        ids,
        resolve,
        scene: Scene::default(),
        meshes: HashMap::new(),
        materials: HashMap::new(),
    };
    let mut nodes = Vec::new();
    if let Some(instance) = children(root, "scene")
        .flat_map(|n| children(n, "instance_visual_scene"))
        .next()
    {
        let scene = importer.find(instance.attribute("url"), "visual_scene")?;
        for node in children(scene, "node") {
            nodes.push(importer.node(node, 0)?);
        }
    } else {
        for geometry in
            children(root, "library_geometries").flat_map(|lib| children(lib, "geometry"))
        {
            let name = geometry
                .attribute("name")
                .or_else(|| geometry.attribute("id"))
                .unwrap_or("COLLADA mesh");
            let parts = importer.parts(geometry, &[])?;
            if !parts.is_empty() {
                let mesh = importer.scene.mesh(name, parts)?;
                nodes.push(importer.scene.node(name, Some(mesh), None, vec![])?);
            }
        }
    }
    let root = importer.scene.node(
        "COLLADA model",
        None,
        Some(multiply(&up, &scale(unit))),
        nodes,
    )?;
    importer.scene.finish(vec![root])
}

type Bindings<'input> = Vec<(&'input str, &'input str)>;

struct Importer<'a, 'input, 'files, 'resolver> {
    resolve: &'files mut ResolveCompanion<'resolver>,
    scene: Scene,
    ids: Nodes<'a, 'input>,
    meshes: HashMap<(&'a str, Bindings<'a>), usize>,
    materials: HashMap<&'a str, usize>,
}

impl<'a, 'input> Importer<'a, 'input, '_, '_> {
    fn find(&self, url: Option<&str>, tag: &str) -> Result<Node<'a, 'input>> {
        self.ids
            .get(reference(url)?)
            .copied()
            .filter(|n| n.has_tag_name(tag))
            .ok_or_else(|| Error(format!("COLLADA {tag} reference is missing")))
    }

    fn node(&mut self, node: Node<'a, 'input>, depth: usize) -> Result<usize> {
        if depth >= 64 {
            return fail("COLLADA hierarchy is too deep or cyclic");
        }
        let name = node
            .attribute("name")
            .or_else(|| node.attribute("id"))
            .unwrap_or("COLLADA node");
        let mut matrix = IDENTITY;
        let mut nodes = Vec::new();
        for entry in node.children().filter(Node::is_element) {
            let transform = match entry.tag_name().name() {
                "matrix" | "translate" | "scale" | "rotate" => transform(entry)?,
                "instance_geometry" => {
                    let id = reference(entry.attribute("url"))?;
                    let geometry = self.find(entry.attribute("url"), "geometry")?;
                    let mut bindings = entry
                        .descendants()
                        .filter(|n| n.has_tag_name("instance_material"))
                        .filter_map(|n| Some((n.attribute("symbol")?, n.attribute("target")?)))
                        .collect::<Vec<_>>();
                    bindings.sort_unstable();
                    let key = (id, bindings);
                    let name = geometry.attribute("name").unwrap_or(id);
                    let mesh = if let Some(&mesh) = self.meshes.get(&key) {
                        mesh
                    } else {
                        let parts = self.parts(geometry, &key.1)?;
                        if parts.is_empty() {
                            continue;
                        }
                        let mesh = self.scene.mesh(name, parts)?;
                        self.meshes.insert(key, mesh);
                        mesh
                    };
                    nodes.push(self.scene.node(name, Some(mesh), None, vec![])?);
                    continue;
                }
                "node" => {
                    nodes.push(self.node(entry, depth + 1)?);
                    continue;
                }
                "instance_node" => {
                    let node = self.find(entry.attribute("url"), "node")?;
                    nodes.push(self.node(node, depth + 1)?);
                    continue;
                }
                _ => continue,
            };
            matrix = multiply(&matrix, &transform);
        }
        self.scene.node(name, None, Some(matrix), nodes)
    }

    fn parts(
        &mut self,
        geometry: Node<'a, 'input>,
        bindings: &[(&'a str, &'a str)],
    ) -> Result<Vec<Mesh>> {
        let mesh = child(geometry, "mesh")?;
        let mut sources = HashMap::new();
        let mut parts = Vec::new();
        for primitive in mesh.children().filter(|n| {
            matches!(
                n.tag_name().name(),
                "triangles" | "polylist" | "polygons" | "trifans" | "tristrips"
            )
        }) {
            let mut part = self.primitive(primitive, &mut sources)?;
            if let Some(symbol) = primitive.attribute("material") {
                let target = bindings
                    .iter()
                    .find(|(key, _)| *key == symbol)
                    .map(|(_, value)| *value);
                part.material = Some(self.material(symbol, target)?);
            }
            parts.push(part);
        }
        Ok(parts)
    }

    fn material(&mut self, symbol: &'a str, target: Option<&'a str>) -> Result<usize> {
        let key = target.unwrap_or(symbol);
        if let Some(&material) = self.materials.get(key) {
            return Ok(material);
        }
        let mut pbr = Material::default();
        let mut name = symbol;
        if let Some(target) = target {
            let material = self.find(Some(target), "material")?;
            name = material.attribute("name").unwrap_or(symbol);
            if let Some(effect) = children(material, "instance_effect").next() {
                let effect = self.find(effect.attribute("url"), "effect")?;
                if let Some(profile) = children(effect, "profile_COMMON").next()
                    && let Some(shader) = profile.descendants().find(|node| {
                        matches!(
                            node.tag_name().name(),
                            "lambert" | "phong" | "blinn" | "constant"
                        )
                    })
                {
                    let input = |name| children(shader, name).next();
                    if let Some(diffuse) = input("diffuse") {
                        parse_color(diffuse, &mut pbr.base_color)?;
                    }
                    let mut base = self.map(profile, input("diffuse"))?;
                    if let Some(emission) = input("emission") {
                        parse_color(emission, &mut pbr.emissive)?;
                        if let Some(image) = self.map(profile, Some(emission))? {
                            if children(emission, "color").next().is_none() {
                                pbr.emissive = [1.; 3];
                            }
                            pbr.emissive_map =
                                Some(self.scene.texture(image, true, Sampler::default())?);
                        }
                    }
                    if let Some(gloss) = input("shininess")
                        .and_then(|node| children(node, "float").next()?.text())
                        .and_then(|value| value.trim().parse::<f32>().ok())
                        .filter(|value| value.is_finite() && *value >= 0.)
                    {
                        pbr.roughness = (2. / (gloss + 2.)).sqrt().clamp(0.02, 1.);
                    }
                    if let Some(mut image) = self.map(profile, input("specular"))? {
                        for pixel in image.pixels_mut() {
                            // Approximate a legacy gloss/specular map as roughness.
                            pixel[0] = 255 - pixel[0];
                        }
                        let image = texture::metallic_roughness(Some(image), None).unwrap();
                        pbr.metallic_roughness_map =
                            Some(self.scene.texture(image, false, Sampler::default())?);
                    }
                    if let Some(normal) = profile.descendants().find(|node| {
                        matches!(node.tag_name().name(), "normal" | "bump")
                            && children(*node, "texture").next().is_some()
                    }) && let Some(image) = self.map(profile, Some(normal))?
                    {
                        let image = if normal.has_tag_name("bump") {
                            super::obj::bump_normals(&image, 1.)
                        } else {
                            image
                        };
                        pbr.normal_map =
                            Some(self.scene.texture(image, false, Sampler::default())?);
                    }
                    if let Some(transparent) = input("transparent") {
                        let factor = input("transparency")
                            .and_then(|node| children(node, "float").next()?.text())
                            .and_then(|value| value.trim().parse::<f32>().ok())
                            .filter(|value| value.is_finite())
                            .unwrap_or(1.)
                            .clamp(0., 1.);
                        let mode = transparent.attribute("opaque").unwrap_or("A_ONE");
                        let opacity = |rgba: [f32; 4]| {
                            let value = if mode.starts_with("RGB") {
                                0.2126 * rgba[0] + 0.7152 * rgba[1] + 0.0722 * rgba[2]
                            } else {
                                rgba[3]
                            };
                            if mode.ends_with("ZERO") {
                                1. - value * factor
                            } else {
                                value * factor
                            }
                        };
                        if let Some(mut image) = self.map(profile, Some(transparent))? {
                            for pixel in image.pixels_mut() {
                                pixel[0] =
                                    (opacity(pixel.0.map(|channel| f32::from(channel) / 255.))
                                        * 255.)
                                        .round() as u8;
                            }
                            base = Some(texture::with_opacity(base, image));
                        } else {
                            let mut color = [1.; 4];
                            parse_color(transparent, &mut color)?;
                            pbr.base_color[3] *= opacity(color);
                        }
                    }
                    if let Some(image) = base {
                        if image.pixels().any(|p| p[3] < 255) {
                            pbr.alpha = Alpha::Blend;
                        }
                        pbr.base_map = Some(self.scene.texture(image, true, Sampler::default())?);
                    }
                    pbr.double_sided = effect.descendants().any(|node| {
                        node.has_tag_name("double_sided")
                            && node
                                .text()
                                .is_some_and(|v| matches!(v.trim(), "1" | "true"))
                    });
                }
            }
        }
        if pbr.base_color[3] < 1. {
            pbr.alpha = Alpha::Blend;
        }
        let index = self.scene.pbr_material(name, pbr);
        self.materials.insert(key, index);
        Ok(index)
    }

    fn map(
        &mut self,
        profile: Node<'a, 'input>,
        input: Option<Node<'a, 'input>>,
    ) -> Result<Option<image::RgbaImage>> {
        let Some(id) =
            input.and_then(|node| children(node, "texture").next()?.attribute("texture"))
        else {
            return Ok(None);
        };
        let param = |sid| children(profile, "newparam").find(|n| n.attribute("sid") == Some(sid));
        let id = param(id)
            .and_then(|sampler| {
                let source = sampler
                    .descendants()
                    .find(|node| node.has_tag_name("source"))?
                    .text()?
                    .trim();
                children(param(source)?, "surface")
                    .next()
                    .and_then(|surface| children(surface, "init_from").next()?.text())
                    .map(str::trim)
            })
            .unwrap_or(id);
        let Some(uri) = self
            .ids
            .get(id.strip_prefix('#').unwrap_or(id))
            .filter(|node| node.has_tag_name("image"))
            .and_then(|image| children(*image, "init_from").next())
            .and_then(|node| {
                children(node, "ref")
                    .next()
                    .and_then(|n| n.text())
                    .or_else(|| node.text())
            })
        else {
            return Ok(None);
        };
        texture::resolve(uri.trim(), self.resolve)
    }

    fn primitive(
        &self,
        primitive: Node<'a, 'input>,
        sources: &mut HashMap<&'a str, Source>,
    ) -> Result<Mesh> {
        let mut inputs = [None; 3];
        let mut stride = 0;
        for input in children(primitive, "input") {
            let offset: usize = number(input.attribute("offset"))?;
            if offset >= 64 {
                return fail("COLLADA input offset exceeds import limits");
            }
            stride = stride.max(offset + 1);
            let semantic = input.attribute("semantic").unwrap_or("");
            if semantic == "VERTEX" {
                for nested in children(self.find(input.attribute("source"), "vertices")?, "input") {
                    read_input(nested, offset, &mut inputs)?;
                }
            } else {
                read_input(input, offset, &mut inputs)?;
            }
        }
        if inputs[0].is_none() {
            return fail("COLLADA primitive has no positions");
        }
        for (id, _) in inputs.iter().flatten() {
            if !sources.contains_key(id) {
                let node = *self
                    .ids
                    .get(id)
                    .filter(|n| n.has_tag_name("source"))
                    .ok_or_else(|| Error("Missing COLLADA mesh source".into()))?;
                sources.insert(*id, Source::read(node, self)?);
            }
        }
        let mut mesh = Mesh::default();
        let mut vertices = HashMap::new();
        let mut vcount = children(primitive, "vcount")
            .next()
            .and_then(|n| n.text())
            .unwrap_or("")
            .split_whitespace();
        for p in children(primitive, "p") {
            let mut values = p.text().unwrap_or("").split_whitespace().peekable();
            let kind = primitive.tag_name().name();
            let mut face = Vec::new();
            while values.peek().is_some() {
                let count = match kind {
                    "triangles" => 3,
                    "polylist" => number(vcount.next())?,
                    _ => MAX_FACE_VERTICES,
                };
                if !(3..=MAX_FACE_VERTICES).contains(&count) {
                    return fail("Invalid COLLADA face size");
                }
                face.clear();
                for _ in 0..count {
                    if !matches!(kind, "triangles" | "polylist") && values.peek().is_none() {
                        break;
                    }
                    let mut tuple = [0; 64];
                    for i in &mut tuple[..stride] {
                        *i = number(values.next())?;
                    }
                    let key = inputs.map(|pair| pair.map_or(0, |(_, i)| tuple[i]));
                    let index = if let Some(&index) = vertices.get(&key) {
                        index
                    } else {
                        let index = mesh.positions.len() as u32;
                        if index as usize == MAX_VERTICES {
                            return fail("COLLADA exceeds the vertex limit");
                        }
                        let get = |i: usize| inputs[i].map(|(id, _)| &sources[id]);
                        mesh.positions.push(get(0).unwrap().at(key[0])?);
                        if let Some(source) = get(1) {
                            mesh.normals.push(source.at(key[1])?);
                        }
                        if let Some(source) = get(2) {
                            let [u, v] = source.at(key[2])?;
                            mesh.uv.push([u, 1. - v]);
                        }
                        vertices.insert(key, index);
                        index
                    };
                    face.push(index);
                }
                match kind {
                    "tristrips" | "trifans" => {
                        for i in 2..face.len() {
                            let mut triangle = if kind == "trifans" {
                                [face[0], face[i - 1], face[i]]
                            } else {
                                [face[i - 2], face[i - 1], face[i]]
                            };
                            if kind == "tristrips" && i % 2 != 0 {
                                triangle.swap(0, 1);
                            }
                            mesh::polygon(&mesh.positions, &triangle, &mut mesh.indices)?;
                        }
                    }
                    _ => mesh::polygon(&mesh.positions, &face, &mut mesh.indices)?,
                }
                if !matches!(kind, "triangles" | "polylist") && values.peek().is_some() {
                    return fail("COLLADA polygon exceeds the vertex limit");
                }
            }
        }
        if vcount.next().is_some() {
            return fail("COLLADA polygon counts do not match the indices");
        }
        Ok(mesh)
    }
}

fn read_input<'a>(
    input: Node<'a, '_>,
    offset: usize,
    inputs: &mut [Option<(&'a str, usize)>; 3],
) -> Result<()> {
    let channel = match input.attribute("semantic") {
        Some("POSITION") => 0,
        Some("NORMAL") => 1,
        Some("TEXCOORD") if matches!(input.attribute("set"), None | Some("0")) => 2,
        _ => return Ok(()),
    };
    inputs[channel] = Some((reference(input.attribute("source"))?, offset));
    Ok(())
}

struct Source {
    data: Vec<f32>,
    offset: usize,
    stride: usize,
    count: usize,
}
impl Source {
    fn read(node: Node<'_, '_>, importer: &Importer<'_, '_, '_, '_>) -> Result<Self> {
        let accessor = child(child(node, "technique_common")?, "accessor")?;
        let array = importer.find(accessor.attribute("source"), "float_array")?;
        let data = array
            .text()
            .unwrap_or("")
            .split_whitespace()
            .take(MAX_VERTICES * 4 + 1)
            .map(|s| number(Some(s)))
            .collect::<Result<Vec<_>>>()?;
        let offset = number(accessor.attribute("offset").or(Some("0")))?;
        let stride = number(accessor.attribute("stride").or(Some("1")))?;
        let count = number(accessor.attribute("count"))?;
        if stride == 0 || count > MAX_VERTICES || data.len() > MAX_VERTICES * 4 {
            return fail("Invalid COLLADA accessor size");
        }
        Ok(Self {
            data,
            offset,
            stride,
            count,
        })
    }
    fn at<const N: usize>(&self, index: usize) -> Result<[f32; N]> {
        let start = index
            .checked_mul(self.stride)
            .and_then(|i| i.checked_add(self.offset));
        let range = start.and_then(|i| Some(i..i.checked_add(N)?));
        if index >= self.count || self.stride < N {
            return fail("Invalid COLLADA attribute index");
        }
        range
            .and_then(|r| self.data.get(r))
            .map(|v| v.try_into().unwrap())
            .ok_or_else(|| Error("Truncated COLLADA attribute array".into()))
    }
}

fn transform(node: Node<'_, '_>) -> Result<Mat4> {
    let mut words = node.text().unwrap_or("").split_whitespace();
    let mut vector = || {
        Ok::<_, Error>([
            number::<f64>(words.next())?,
            number(words.next())?,
            number(words.next())?,
        ])
    };
    let matrix = match node.tag_name().name() {
        "translate" => translation(vector()?),
        "scale" => {
            let mut m = IDENTITY;
            [m[0], m[5], m[10]] = vector()?;
            m
        }
        "rotate" => {
            let axis = vector()?;
            let angle = number::<f64>(words.next())?.to_radians() / 2.;
            let length = axis.iter().map(|v| v * v).sum::<f64>().sqrt();
            if !length.is_finite() || length <= 0. {
                return fail("Invalid COLLADA rotation axis");
            }
            let [x, y, z] = axis.map(|v| v * angle.sin() / length);
            compose([0.; 3], [x, y, z, angle.cos()], [1.; 3])
        }
        _ => {
            let mut m = IDENTITY;
            for row in 0..4 {
                for col in 0..4 {
                    m[col * 4 + row] = number(words.next())?;
                }
            }
            m
        }
    };
    if words.next().is_some() {
        return fail("Invalid COLLADA transform");
    }
    Ok(matrix)
}

fn parse_color<const N: usize>(node: Node<'_, '_>, channels: &mut [f32; N]) -> Result<()> {
    if let Some(color) = children(node, "color").next() {
        let mut words = color.text().unwrap_or("").split_whitespace();
        for channel in channels {
            let value: f32 = number(words.next())?;
            if !value.is_finite() {
                return fail("Invalid COLLADA material color");
            }
            *channel = value.clamp(0., 1.);
        }
    }
    Ok(())
}
