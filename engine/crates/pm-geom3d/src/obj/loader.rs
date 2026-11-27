//! three.js r180's `OBJLoader` and `MTLLoader` (what `prepareModelImport`
//! runs): the object list with per-object non-indexed geometry and material
//! groups, and the MTL's Phong material parameters.

use crate::js;

/// `geometry.type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mesh,
    Line,
    Points,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialUse {
    pub name: String,
    pub group_start: i64,
    pub group_end: i64,
    pub group_count: i64,
    pub inherited: bool,
}

impl MaterialUse {
    fn new(name: &str, group_start: i64) -> MaterialUse {
        MaterialUse {
            name: name.to_string(),
            group_start,
            group_end: -1,
            group_count: -1,
            inherited: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Object {
    pub name: String,
    from_declaration: bool,
    pub kind: Kind,
    /// Flat arrays as OBJLoader pushes them (NaN where JS reads `undefined`).
    pub vertices: Vec<f64>,
    pub normals: Vec<f64>,
    pub colors: Vec<f64>,
    pub uvs: Vec<f64>,
    pub has_uv_indices: bool,
    pub materials: Vec<MaterialUse>,
}

impl Object {
    fn new(name: &str, from_declaration: bool) -> Object {
        Object {
            name: name.to_string(),
            from_declaration,
            kind: Kind::Mesh,
            vertices: vec![],
            normals: vec![],
            colors: vec![],
            uvs: vec![],
            has_uv_indices: false,
            materials: vec![],
        }
    }

    /// `_finalize(end)`: closes the current material's group and returns it.
    fn finalize(&mut self, end: bool) -> Option<MaterialUse> {
        let vertex_count = (self.vertices.len() / 3) as i64;
        let last = self.materials.last_mut().map(|m| {
            if m.group_end == -1 {
                m.group_end = vertex_count;
                m.group_count = m.group_end - m.group_start;
                m.inherited = false;
            }
            m.clone()
        });
        if end && self.materials.len() > 1 {
            self.materials.retain(|m| m.group_count > 0);
        }
        if end && self.materials.is_empty() {
            self.materials.push(MaterialUse::new("", 0));
        }
        last
    }

    /// `startMaterial(name)`.
    fn start_material(&mut self, name: &str) {
        let previous = self.finalize(false);
        if let Some(p) = &previous
            && (p.inherited || p.group_count <= 0)
        {
            self.materials.pop();
        }
        let start = previous.map_or(0, |p| p.group_end);
        self.materials.push(MaterialUse::new(name, start));
    }
}

struct State {
    objects: Vec<Object>,
    vertices: Vec<f64>,
    normals: Vec<f64>,
    /// Per vertex: its color (JS pushes `undefined` for none).
    colors: Vec<Option<[f64; 3]>>,
    uvs: Vec<f64>,
}

fn at(src: &[f64], i: f64) -> f64 {
    if i >= 0.0 && i.fract() == 0.0 {
        src.get(i as usize).copied().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

impl State {
    fn object(&mut self) -> &mut Object {
        self.objects.last_mut().expect("an object")
    }

    /// `startObject(name, fromDeclaration)`.
    fn start_object(&mut self, name: &str, from_declaration: bool) {
        if let Some(o) = self.objects.last_mut()
            && !o.from_declaration
        {
            o.name = name.to_string();
            o.from_declaration = from_declaration;
            return;
        }
        let previous = self
            .objects
            .last()
            .and_then(|o| o.materials.last().cloned());
        if let Some(o) = self.objects.last_mut() {
            o.finalize(true);
        }
        let mut object = Object::new(name, from_declaration);
        if let Some(p) = previous
            && !p.name.is_empty()
        {
            let mut declared = MaterialUse::new(&p.name, 0);
            declared.inherited = true;
            object.materials.push(declared);
        }
        self.objects.push(object);
    }

    fn vertex_index(&self, value: &str) -> f64 {
        let i = js::parse_int(value);
        (if i >= 0.0 {
            i - 1.0
        } else {
            i + (self.vertices.len() / 3) as f64
        }) * 3.0
    }

    fn normal_index(&self, value: &str) -> f64 {
        let i = js::parse_int(value);
        (if i >= 0.0 {
            i - 1.0
        } else {
            i + (self.normals.len() / 3) as f64
        }) * 3.0
    }

    fn uv_index(&self, value: &str) -> f64 {
        let i = js::parse_int(value);
        (if i >= 0.0 {
            i - 1.0
        } else {
            i + (self.uvs.len() / 2) as f64
        }) * 2.0
    }

    fn push3(&mut self, which: u8, src_index: f64) {
        let src = if which == 0 {
            &self.vertices
        } else {
            &self.normals
        };
        let v = [
            at(src, src_index),
            at(src, src_index + 1.0),
            at(src, src_index + 2.0),
        ];
        let o = self.objects.last_mut().expect("an object");
        let dst = if which == 0 {
            &mut o.vertices
        } else {
            &mut o.normals
        };
        dst.extend(v);
    }

    fn push_color(&mut self, index: f64) {
        // `src[a] !== undefined`, with `a` the vertex index times 3.
        let color = if index >= 0.0 && index.fract() == 0.0 && (index as usize).is_multiple_of(3) {
            self.colors.get(index as usize / 3).copied().flatten()
        } else {
            None
        };
        if let Some(c) = color {
            self.object().colors.extend(c);
        }
    }

    fn push_uv(&mut self, index: f64) {
        let v = [at(&self.uvs, index), at(&self.uvs, index + 1.0)];
        self.object().uvs.extend(v);
    }

    fn add_face(&mut self, a: &str, b: &str, c: &str, uv: [Option<&str>; 3], n: [Option<&str>; 3]) {
        let (ia, ib, ic) = (
            self.vertex_index(a),
            self.vertex_index(b),
            self.vertex_index(c),
        );
        for i in [ia, ib, ic] {
            self.push3(0, i);
        }
        for i in [ia, ib, ic] {
            self.push_color(i);
        }
        if let Some(na) = n[0].filter(|s| !s.is_empty()) {
            let idx = [
                self.normal_index(na),
                self.normal_index(n[1].unwrap_or("")),
                self.normal_index(n[2].unwrap_or("")),
            ];
            for i in idx {
                self.push3(1, i);
            }
        } else {
            // addFaceNormal: (C − B) × (A − B), normalized.
            let v = |i: f64| {
                [
                    at(&self.vertices, i),
                    at(&self.vertices, i + 1.0),
                    at(&self.vertices, i + 2.0),
                ]
            };
            let (va, vb, vc) = (v(ia), v(ib), v(ic));
            let cb = [vc[0] - vb[0], vc[1] - vb[1], vc[2] - vb[2]];
            let ab = [va[0] - vb[0], va[1] - vb[1], va[2] - vb[2]];
            let x = cb[1] * ab[2] - cb[2] * ab[1];
            let y = cb[2] * ab[0] - cb[0] * ab[2];
            let z = cb[0] * ab[1] - cb[1] * ab[0];
            let len = (x * x + y * y + z * z).sqrt();
            let s = 1.0 / if len == 0.0 || len.is_nan() { 1.0 } else { len };
            let normal = [x * s, y * s, z * s];
            let o = self.object();
            for _ in 0..3 {
                o.normals.extend(normal);
            }
        }
        if let Some(ua) = uv[0].filter(|s| !s.is_empty()) {
            let idx = [
                self.uv_index(ua),
                self.uv_index(uv[1].unwrap_or("")),
                self.uv_index(uv[2].unwrap_or("")),
            ];
            for i in idx {
                self.push_uv(i);
            }
            self.object().has_uv_indices = true;
        } else {
            self.object().uvs.extend([0.0; 6]);
        }
    }
}

/// `OBJLoader.parse(text)`: the objects with vertices (empty ones are
/// skipped, as the loader skips them).
pub fn parse_obj(text: &str) -> Vec<Object> {
    let mut text = text.to_string();
    if text.contains("\r\n") {
        text = text.replace("\r\n", "\n");
    }
    if text.contains("\\\n") {
        text = text.replace("\\\n", "");
    }
    let mut state = State {
        objects: vec![],
        vertices: vec![],
        normals: vec![],
        colors: vec![],
        uvs: vec![],
    };
    state.start_object("", false);
    for raw in text.split('\n') {
        let line = js::trim_start(raw);
        let Some(first) = line.chars().next() else {
            continue;
        };
        match first {
            '#' => {}
            'v' => {
                let data = js::split_ws(line);
                let f = |i: usize| data.get(i).map_or(f64::NAN, |s| js::parse_float(s));
                match data[0] {
                    "v" => {
                        state.vertices.extend([f(1), f(2), f(3)]);
                        state.colors.push((data.len() >= 7).then(|| {
                            // setRGB(r, g, b, SRGBColorSpace): to linear working space.
                            [
                                srgb_to_linear(f(4)),
                                srgb_to_linear(f(5)),
                                srgb_to_linear(f(6)),
                            ]
                        }));
                    }
                    "vn" => state.normals.extend([f(1), f(2), f(3)]),
                    "vt" => state.uvs.extend([f(1), f(2)]),
                    _ => {}
                }
            }
            'f' => {
                let data = js::trim(&line[1..]);
                let corners: Vec<Vec<&str>> = js::split_ws(data)
                    .into_iter()
                    .filter(|v| !v.is_empty())
                    .map(|v| v.split('/').collect())
                    .collect();
                if corners.is_empty() {
                    continue;
                }
                fn part<'a>(c: &[&'a str], i: usize) -> Option<&'a str> {
                    c.get(i).copied()
                }
                let v1 = corners[0].clone();
                for j in 1..corners.len().saturating_sub(1) {
                    let (v2, v3) = (&corners[j], &corners[j + 1]);
                    let (a, b, c) = (
                        part(&v1, 0).unwrap_or(""),
                        part(v2, 0).unwrap_or(""),
                        part(v3, 0).unwrap_or(""),
                    );
                    let uv = [part(&v1, 1), part(v2, 1), part(v3, 1)];
                    let n = [part(&v1, 2), part(v2, 2), part(v3, 2)];
                    state.add_face(a, b, c, uv, n);
                }
            }
            'l' => {
                let parts: Vec<&str> = js::trim(&line[1..]).split(' ').collect();
                let mut verts: Vec<&str> = vec![];
                let mut uvs: Vec<Option<&str>> = vec![];
                if !line.contains('/') {
                    verts = parts;
                } else {
                    for p in parts {
                        let mut s = p.split('/');
                        let (v, t) = (s.next().unwrap_or(""), s.next());
                        if !v.is_empty() {
                            verts.push(v);
                        }
                        if t != Some("") {
                            uvs.push(t);
                        }
                    }
                }
                state.object().kind = Kind::Line;
                for v in verts {
                    let i = state.vertex_index(v);
                    state.push3(0, i);
                }
                for t in uvs {
                    let i = t.map_or(f64::NAN, |t| state.uv_index(t));
                    state.push_uv(i);
                }
            }
            'p' => {
                state.object().kind = Kind::Points;
                for v in js::trim(&line[1..]).split(' ') {
                    let i = state.vertex_index(v);
                    state.push3(0, i);
                    state.push_color(i);
                }
            }
            'o' | 'g' => {
                // /^[og]\s*(.+)?/: the match stops at a line terminator.
                let end = line
                    .find(['\r', '\u{2028}', '\u{2029}'])
                    .unwrap_or(line.len());
                let name = js::trim(&line[1..end]).to_string();
                state.start_object(&name, true);
            }
            _ => {
                if let Some(name) = line.strip_prefix("usemtl ") {
                    let name = js::trim(name).to_string();
                    state.object().start_material(&name);
                }
                // mtllib (every selected MTL is used), usemap, and s change
                // nothing prepareModelImport keeps.
            }
        }
    }
    if let Some(o) = state.objects.last_mut() {
        o.finalize(true);
    }
    state.objects.retain(|o| !o.vertices.is_empty());
    state.objects
}

/// `SRGBToLinear`.
pub fn srgb_to_linear(c: f64) -> f64 {
    if c < 0.04045 {
        c * 0.0773993808
    } else {
        (c * 0.9478672986 + 0.0521327014).powf(2.4)
    }
}

/// A texture reference (`getTextureParams`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextureParams {
    pub url: String,
    pub scale: [f64; 2],
    pub offset: [f64; 2],
}

/// MeshPhongMaterial parameters from the MTL (`createMaterial_`).
#[derive(Clone, Debug, PartialEq)]
pub struct Phong {
    pub name: String,
    pub color: [f64; 3],
    pub emissive: [f64; 3],
    pub opacity: f64,
    pub transparent: bool,
    pub shininess: f64,
    pub bump_scale: f64,
    /// By three.js property name, first texture per slot.
    pub maps: Vec<(&'static str, TextureParams)>,
}

/// `MTLLoader.parse`: materials by name, each property in first-occurrence
/// order with its last value.
pub fn parse_mtl(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut materials: Vec<(String, Vec<(String, String)>)> = vec![];
    let mut current: Option<usize> = None;
    for raw in text.split('\n') {
        let line = js::trim(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.find(' ') {
            Some(p) => (&line[..p], js::trim(&line[p + 1..])),
            None => (line, ""),
        };
        let key = key.to_lowercase();
        if key == "newmtl" {
            let info = vec![("name".to_string(), value.to_string())];
            match materials.iter().position(|(n, _)| n == value) {
                Some(i) => materials[i].1 = info,
                None => materials.push((value.to_string(), info)),
            }
            current = materials.iter().position(|(n, _)| n == value);
        } else {
            // Before any newmtl, properties go to a throwaway object.
            let Some(i) = current else { continue };
            let props = &mut materials[i].1;
            match props.iter_mut().find(|(k, _)| *k == key) {
                Some(p) => p.1 = value.to_string(),
                None => props.push((key, value.to_string())),
            }
        }
    }
    materials
}

fn color3(value: &str) -> [f64; 3] {
    // value.split(/\s+/, 3).map(parseFloat)
    let parts = js::split_ws(value);
    std::array::from_fn(|i| parts.get(i).map_or(f64::NAN, |s| js::parse_float(s)))
}

/// `getTextureParams(value, params)` (sets `bumpScale` on -bm).
fn texture_params(value: &str, bump_scale: &mut f64) -> TextureParams {
    let mut items: Vec<String> = js::split_ws(value).into_iter().map(String::from).collect();
    let num = |items: &[String], i: usize| items.get(i).map_or(f64::NAN, |s| js::parse_float(s));
    let mut out = TextureParams {
        url: String::new(),
        scale: [1.0, 1.0],
        offset: [0.0, 0.0],
    };
    let splice = |items: &mut Vec<String>, pos: usize, n: usize| {
        let end = (pos + n).min(items.len());
        items.drain(pos..end);
    };
    if let Some(pos) = items.iter().position(|s| s == "-bm") {
        *bump_scale = num(&items, pos + 1);
        splice(&mut items, pos, 2);
    }
    if let Some(pos) = items.iter().position(|s| s == "-mm") {
        splice(&mut items, pos, 3);
    }
    if let Some(pos) = items.iter().position(|s| s == "-s") {
        out.scale = [num(&items, pos + 1), num(&items, pos + 2)];
        splice(&mut items, pos, 4);
    }
    if let Some(pos) = items.iter().position(|s| s == "-o") {
        out.offset = [num(&items, pos + 1), num(&items, pos + 2)];
        splice(&mut items, pos, 4);
    }
    out.url = js::trim(&items.join(" ")).to_string();
    out
}

/// `MaterialCreator.createMaterial_(name)` (an unknown name gets the
/// defaults).
pub fn phong(name: &str, props: Option<&[(String, String)]>) -> Phong {
    let mut m = Phong {
        name: name.to_string(),
        color: [1.0; 3],
        emissive: [0.0; 3],
        opacity: 1.0,
        transparent: false,
        shininess: 30.0,
        bump_scale: 1.0,
        maps: vec![],
    };
    let set_map = |m: &mut Phong, slot: &'static str, value: &str| {
        if m.maps.iter().any(|(s, _)| *s == slot) {
            return;
        }
        let mut bump = m.bump_scale;
        let t = texture_params(value, &mut bump);
        m.bump_scale = bump;
        m.maps.push((slot, t));
    };
    for (key, value) in props.into_iter().flatten() {
        if value.is_empty() && !matches!(key.as_str(), "ka" | "kd" | "ks" | "ke") {
            continue;
        }
        match key.as_str() {
            "kd" => m.color = color3(value).map(srgb_to_linear),
            "ke" => m.emissive = color3(value).map(srgb_to_linear),
            "map_kd" => set_map(&mut m, "map", value),
            "map_ks" => set_map(&mut m, "specularMap", value),
            "map_ke" => set_map(&mut m, "emissiveMap", value),
            "norm" => set_map(&mut m, "normalMap", value),
            "map_bump" | "bump" => set_map(&mut m, "bumpMap", value),
            "disp" => set_map(&mut m, "displacementMap", value),
            "map_d" => {
                set_map(&mut m, "alphaMap", value);
                m.transparent = true;
            }
            "ns" => m.shininess = js::parse_float(value),
            "d" => {
                let n = js::parse_float(value);
                if n < 1.0 {
                    m.opacity = n;
                    m.transparent = true;
                }
            }
            "tr" => {
                let n = js::parse_float(value);
                if n > 0.0 {
                    m.opacity = 1.0 - n;
                    m.transparent = true;
                }
            }
            _ => {}
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_and_groups_follow_obj_loader() {
        let objects = parse_obj(
            "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0 1 0.5 0\nf 1 2 3\no A\nusemtl Red\nf 1 2 3 4\nusemtl Blue\nf 1 3 4\no B\nf 1 2 3\nl 1 2\n",
        );
        // Faces before the first `o` belong to it (the default object is renamed).
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].name, "A");
        assert_eq!(objects[0].vertices.len(), 3 * 3 * 4);
        let groups: Vec<_> = objects[0]
            .materials
            .iter()
            .map(|m| (m.name.as_str(), m.group_start, m.group_count))
            .collect();
        // The first usemtl opens at 0 (no previous material), so the face
        // before it joins Red.
        assert_eq!(groups, vec![("Red", 0, 9), ("Blue", 9, 3)]);
        // B inherits Blue and becomes a line (its type is the last set).
        assert_eq!(objects[1].kind, Kind::Line);
        assert_eq!(objects[1].materials[0].name, "Blue");
        assert_eq!(objects[1].vertices.len(), 3 * 3 + 2 * 3);
        // Only vertex 4 has a color; faces using it push it alone.
        assert_eq!(objects[0].colors.len(), 3 * 2);
        assert!((objects[0].colors[1] - srgb_to_linear(0.5)).abs() < 1e-15);
    }

    #[test]
    fn mtl_materials_follow_mtl_loader() {
        let mtl = parse_mtl(
            "newmtl A\nKd 0.5 0.5 0.5\nNs 10\nd 0.4\nTr 0.25\nmap_Kd -s 2 2 1 -bm 0.5 wood.png\nbump b1.png\nmap_bump b2.png\n",
        );
        let m = phong("A", Some(&mtl[0].1));
        assert_eq!(m.color[0], srgb_to_linear(0.5));
        assert_eq!(m.shininess, 10.0);
        assert_eq!((m.opacity, m.transparent), (0.75, true));
        assert_eq!(m.bump_scale, 0.5);
        assert_eq!(m.maps.len(), 2);
        assert_eq!(m.maps[0].1.url, "wood.png");
        assert_eq!(m.maps[0].1.scale, [2.0, 2.0]);
        // The first key filling a slot wins.
        assert_eq!(
            m.maps[1],
            (
                "bumpMap",
                TextureParams {
                    url: "b1.png".into(),
                    scale: [1.0; 2],
                    offset: [0.0; 2]
                }
            )
        );
        // A repeated key keeps its first position and its last value.
        let again = parse_mtl("newmtl B\nmap_Kd a.png\nKd 1 0 0\nmap_Kd b.png\n");
        assert_eq!(again[0].1[1], ("map_kd".to_string(), "b.png".to_string()));
        let unknown = phong("Missing", None);
        assert_eq!((unknown.color, unknown.shininess), ([1.0; 3], 30.0));
    }
}
