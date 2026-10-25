//! The passes and draws three.js r180 issues for one 3D layer, as data:
//! the shared prepass (shadow maps of the depth set's casters, then depth
//! of the set's occluding models) and the layer's color pass (opaque
//! items, then transparent items with double-sided ones drawn back face
//! first), each draw with its program parameters, uniform values,
//! textures, and fixed-function state (`WebGLRenderer.renderObjects`,
//! `WebGLShadowMap.render`, `WebGLMaterials`, `WebGLLights`). The GPU
//! executes this list ([`super::gpu`]); tests compare it with the draws
//! three.js recorded.

use crate::three::materials::{self, LightCounts, Side};
use crate::three::math::{self, Mat4};
use crate::three::program::Parameters;

use super::lights::{self, LightState, ShadowView};
use super::model::{Light3d, LightKind, Material3d, MaterialKind, Materials, Scene3d, Texture3d};

/// What a sampler uniform binds.
#[derive(Debug, Clone, PartialEq)]
pub enum TextureRef {
    Map(Texture3d),
    /// The shadow map of `scene.lights[i]`.
    Shadow(usize),
    Ltc1,
    Ltc2,
    /// A glTF texture (its image, sampler, and color space).
    Gltf(super::gltf::GltfMap),
    /// A float data texture (`boneTexture`, `morphTargetsTexture`; RGBA
    /// f32, nearest, `layers` > 1 for an array texture).
    Data {
        key: u64,
        width: u32,
        height: u32,
        layers: u32,
        /// Sampled as an array texture (`sampler2DArray`).
        array: bool,
        data: std::sync::Arc<Vec<f32>>,
    },
    /// The transmission render target of the layer's color pass.
    Transmission,
}

/// What a draw rasterizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Topology {
    #[default]
    Triangles,
    /// `LineSegments` and `LineLoop` (converted to segments).
    Lines,
    LineStrip,
    Points,
}

/// Fixed-function state of a draw (`WebGLState.setMaterial`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawState {
    /// Depth-only shadow caster, with no color attachment.
    pub shadow: bool,
    pub color_write: bool,
    /// An explicit channel mask (viewport selection coverage preserves IDs).
    pub color_mask: Option<u32>,
    /// False only for coverage drawn independently of scene occlusion.
    pub depth_test: bool,
    pub depth_write: bool,
    /// `NormalBlending` of a transparent material (straight alpha).
    pub blend: bool,
    /// Faces culled (`None`: double-sided).
    pub cull: bool,
    /// `frontFace(CW)`: `BackSide` XOR a mirrored world matrix.
    pub front_face_cw: bool,
    /// `material.wireframe`: lines over the wireframe index.
    pub lines: bool,
    /// The primitive's topology (glTF points and lines).
    pub topology: Topology,
}

/// One `renderBufferDirect`.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawDesc {
    pub params: Parameters,
    pub uniforms: Vec<(String, Vec<f64>)>,
    pub textures: Vec<(String, TextureRef)>,
    pub state: DrawState,
    /// `scene.objects[object].meshes[mesh]`.
    pub object: usize,
    pub mesh: usize,
    /// The draw range in the geometry's draw order (before the wireframe
    /// index doubles it): `(start, count)`; `None` for the whole geometry.
    pub range: Option<(u32, u32)>,
}

/// A pass of the layer's render.
#[derive(Debug, Clone, PartialEq)]
pub enum PassDesc {
    /// The transmission pass of the layer's color pass
    /// (`renderTransmissionPass`): its opaque items into the transmission
    /// target (cleared to `clear`, linear, untoned), resolved and
    /// mipmapped; then the back faces of double-sided transmissive items,
    /// resolved and mipmapped again.
    Transmission {
        clear: [f64; 4],
        opaque: Vec<DrawDesc>,
        back: Vec<DrawDesc>,
    },
    /// A light's shadow map: cleared to white, one viewport per camera.
    Shadow {
        light: usize,
        size: (u32, u32),
        views: Vec<(ShadowView, Vec<DrawDesc>)>,
    },
    /// Depth of the set's occluding models (no color writes).
    Prepass(Vec<DrawDesc>),
    /// The layer's linear color, cleared to transparent before the output transform.
    Color {
        clear: [f64; 4],
        draws: Vec<DrawDesc>,
    },
}

/// The layer render [`describe`] lays out.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer3d<'a> {
    pub scene: &'a Scene3d,
    /// `scene.objects` index of the layer's object.
    pub object: usize,
    /// `scene.objects` indices of its depth set.
    pub depth_set: &'a [usize],
    /// The layer draws the scene background.
    pub background: bool,
}

/// `Frustum.setFromProjectionMatrix(projScreen)` (WebGL clip space):
/// normalized planes `[nx, ny, nz, constant]`.
pub fn frustum(m: &Mat4) -> [[f64; 4]; 6] {
    let me = m;
    let plane = |a: [f64; 4]| {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        let inv = 1.0 / l;
        [a[0] * inv, a[1] * inv, a[2] * inv, a[3] * inv]
    };
    [
        plane([
            me[3] - me[0],
            me[7] - me[4],
            me[11] - me[8],
            me[15] - me[12],
        ]),
        plane([
            me[3] + me[0],
            me[7] + me[4],
            me[11] + me[8],
            me[15] + me[12],
        ]),
        plane([
            me[3] + me[1],
            me[7] + me[5],
            me[11] + me[9],
            me[15] + me[13],
        ]),
        plane([
            me[3] - me[1],
            me[7] - me[5],
            me[11] - me[9],
            me[15] - me[13],
        ]),
        plane([
            me[3] - me[2],
            me[7] - me[6],
            me[11] - me[10],
            me[15] - me[14],
        ]),
        plane([
            me[3] + me[2],
            me[7] + me[6],
            me[11] + me[10],
            me[15] + me[14],
        ]),
    ]
}

/// `frustum.intersectsObject(mesh)`: the geometry's bounding sphere moved
/// by the world matrix (radius times the largest axis scale) against the
/// planes. Culled meshes draw nothing either way; culling keeps the draw
/// list three.js's.
fn visible_in(planes: &[[f64; 4]; 6], mesh: &super::model::Mesh3d) -> bool {
    let (c, r) = match &mesh.gltf {
        // Skinned meshes are never culled (three.js keeps the first pose's
        // sphere, which this engine does not reproduce).
        Some(g) => match g.sphere {
            Some(s) => s,
            None => return true,
        },
        None => mesh.geometry.sphere,
    };
    let center = lights::apply(&mesh.world, c);
    let te = &mesh.world;
    let sx = te[0] * te[0] + te[1] * te[1] + te[2] * te[2];
    let sy = te[4] * te[4] + te[5] * te[5] + te[6] * te[6];
    let sz = te[8] * te[8] + te[9] * te[9] + te[10] * te[10];
    let radius = r * sx.max(sy).max(sz).sqrt();
    planes
        .iter()
        .all(|p| p[0] * center[0] + p[1] * center[1] + p[2] * center[2] + p[3] >= -radius)
}

/// One render item (`currentRenderList.push`).
struct Item<'s> {
    object: usize,
    mesh: usize,
    material: &'s Material3d,
    range: Option<(u32, u32)>,
}

fn items<'s>(scene: &'s Scene3d, object: usize, planes: &[[f64; 4]; 6]) -> Vec<Item<'s>> {
    let o = &scene.objects[object];
    let mut out = Vec::new();
    if !o.visible {
        return out;
    }
    for (m, mesh) in o.meshes.iter().enumerate() {
        if !visible_in(planes, mesh) {
            continue;
        }
        match &mesh.materials {
            Materials::Single(material) => out.push(Item {
                object,
                mesh: m,
                material,
                range: None,
            }),
            Materials::Slots(list) => {
                for g in mesh.geometry.geometry.groups() {
                    if let Some(material) = list.get(g.material_index as usize) {
                        out.push(Item {
                            object,
                            mesh: m,
                            material,
                            range: Some((g.start, g.count)),
                        });
                    }
                }
            }
        }
    }
    out
}

/// Whether a mesh is glTF points or lines. Today's per-layer render
/// edits only meshes' materials (`child.isMesh`), so points and lines keep
/// their own visibility and writes: every layer's depth prepass and color
/// pass draws the points and lines of every visible object.
fn loose(mesh: &super::model::Mesh3d) -> bool {
    mesh.gltf
        .as_ref()
        .is_some_and(|g| g.mode != pm_geom3d::gltf::Mode::Triangles)
}

/// The points and lines of every visible object but `except`.
fn loose_items<'s>(
    scene: &'s Scene3d,
    planes: &[[f64; 4]; 6],
    except: Option<usize>,
) -> Vec<Item<'s>> {
    let mut out = Vec::new();
    for o in 0..scene.objects.len() {
        if Some(o) == except || !scene.objects[o].meshes.iter().any(loose) {
            continue;
        }
        out.extend(
            items(scene, o, planes)
                .into_iter()
                .filter(|i| loose(&scene.objects[i.object].meshes[i.mesh])),
        );
    }
    out
}

fn program(
    mesh: &super::model::Mesh3d,
    material: &Material3d,
    side: Side,
    counts: &LightCounts,
    target: super::gltf::material::Target,
) -> Result<Parameters, String> {
    if mesh.gltf.is_some() {
        return super::gltf::material::parameters(mesh, material, side, counts, target);
    }
    match material.kind {
        MaterialKind::Standard => materials::standard(&materials::Standard {
            name: material.name.clone(),
            side,
            transparent: material.transparent,
            maps: material.maps.flags(),
            lights: *counts,
        }),
        MaterialKind::Basic => materials::basic(&material.name, side, material.transparent, counts),
        MaterialKind::Physical | MaterialKind::Points | MaterialKind::Line => {
            Err("a glTF material class on a scene mesh".into())
        }
    }
}

/// Whether a material is lit (`materialNeedsLights`).
fn lit(m: &Material3d) -> bool {
    matches!(m.kind, MaterialKind::Standard | MaterialKind::Physical)
}

fn state(
    material_side: Side,
    world: &Mat4,
    color: bool,
    depth: bool,
    blend: bool,
    lines: bool,
) -> DrawState {
    let mirrored = math::determinant(world) < 0.0;
    DrawState {
        shadow: false,
        color_write: color,
        color_mask: None,
        depth_test: true,
        depth_write: depth,
        blend,
        cull: material_side != Side::Double,
        front_face_cw: (material_side == Side::Back) != mirrored,
        lines,
        topology: Topology::Triangles,
    }
}

/// A glTF primitive's topology.
fn topology(mesh: &super::model::Mesh3d) -> Topology {
    match mesh.gltf.as_ref().map(|g| g.mode) {
        Some(pm_geom3d::gltf::Mode::Points) => Topology::Points,
        Some(pm_geom3d::gltf::Mode::LineSegments | pm_geom3d::gltf::Mode::LineLoop) => {
            Topology::Lines
        }
        Some(pm_geom3d::gltf::Mode::LineStrip) => Topology::LineStrip,
        _ => Topology::Triangles,
    }
}

/// Camera uniforms (`setProgram`).
fn camera_uniforms(
    u: &mut Vec<(String, Vec<f64>)>,
    projection: &Mat4,
    view: &Mat4,
    world: &Mat4,
    ortho: bool,
) {
    u.push(("projectionMatrix".into(), projection.to_vec()));
    u.push(("viewMatrix".into(), view.to_vec()));
    u.push((
        "cameraPosition".into(),
        vec![world[12], world[13], world[14]],
    ));
    u.push(("isOrthographic".into(), vec![f64::from(u8::from(ortho))]));
}

/// Object uniforms (`modelViewMatrix`, `normalMatrix`, `modelMatrix`).
fn object_uniforms(u: &mut Vec<(String, Vec<f64>)>, view: &Mat4, world: &Mat4) {
    let model_view = math::multiply(view, world);
    u.push(("modelViewMatrix".into(), model_view.to_vec()));
    u.push((
        "normalMatrix".into(),
        math::normal_matrix(&model_view).to_vec(),
    ));
    u.push(("modelMatrix".into(), world.to_vec()));
}

/// `refreshUniformsCommon` and `refreshUniformsStandard`.
fn material_uniforms(
    u: &mut Vec<(String, Vec<f64>)>,
    t: &mut Vec<(String, TextureRef)>,
    m: &Material3d,
    side: Side,
) {
    u.push(("opacity".into(), vec![m.opacity]));
    u.push(("diffuse".into(), m.color.to_vec()));
    if m.kind == MaterialKind::Basic {
        return;
    }
    u.push((
        "emissive".into(),
        m.emissive
            .iter()
            .map(|v| v * m.emissive_intensity)
            .collect(),
    ));
    for (name, map) in m.maps.all() {
        if let Some(map) = map {
            t.push((name.into(), TextureRef::Map(map.clone())));
            u.push((format!("{name}Transform"), map.transform.to_vec()));
        }
    }
    if m.maps.normal.is_some() {
        let s = if side == Side::Back { -1.0 } else { 1.0 };
        u.push(("normalScale".into(), vec![s, s]));
    }
    if m.maps.ao.is_some() {
        u.push(("aoMapIntensity".into(), vec![1.0]));
    }
    u.push(("metalness".into(), vec![m.metalness]));
    u.push(("roughness".into(), vec![m.roughness]));
    u.push(("envMapIntensity".into(), vec![1.0]));
}

/// The sampler bindings of the light state (shadow maps, LTC tables).
fn light_textures(scene: &Scene3d, ls: &LightState) -> Vec<(String, TextureRef)> {
    let mut t = Vec::new();
    let mut rect = false;
    for (k, &i) in ls.order.iter().enumerate() {
        let l: &Light3d = &scene.lights[i];
        if l.kind == LightKind::RectArea {
            rect = true;
        }
        if l.cast_shadow && ls.counts.shadow_map {
            t.push((lights::shadow_sampler(l, ls.slot[k]), TextureRef::Shadow(i)));
        }
    }
    if rect {
        t.push(("ltc_1".into(), TextureRef::Ltc1));
        t.push(("ltc_2".into(), TextureRef::Ltc2));
    }
    t
}

fn shading_draw(
    scene: &Scene3d,
    ls: &LightState,
    light_uniforms: &[(String, Vec<f64>)],
    item: &Item<'_>,
    side: Side,
    color: bool,
    depth: bool,
) -> Result<DrawDesc, String> {
    draw_to(
        scene,
        ls,
        light_uniforms,
        item,
        side,
        color,
        depth,
        super::gltf::material::Target::Canvas,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_to(
    scene: &Scene3d,
    ls: &LightState,
    light_uniforms: &[(String, Vec<f64>)],
    item: &Item<'_>,
    side: Side,
    color: bool,
    depth: bool,
    target: super::gltf::material::Target,
) -> Result<DrawDesc, String> {
    let (w, h) = scene.canvas();
    let mesh = &scene.objects[item.object].meshes[item.mesh];
    let m = item.material;
    let mut u = Vec::new();
    let mut t = Vec::new();
    let view = scene.camera.view();
    camera_uniforms(
        &mut u,
        &scene.camera.projection_matrix(w, h),
        &view,
        &scene.camera.world,
        scene.camera.orthographic() && !matches!(m.kind, MaterialKind::Points | MaterialKind::Line),
    );
    object_uniforms(&mut u, &view, &mesh.world);
    if mesh.gltf.is_some() {
        super::gltf::material::uniforms(&mut u, &mut t, m, side, h, (w, h));
        super::gltf::material::object_uniforms(&mut u, &mut t, mesh);
    } else {
        material_uniforms(&mut u, &mut t, m, side);
    }
    u.push(("toneMappingExposure".into(), vec![scene.exposure]));
    if lit(m) {
        u.extend(light_uniforms.iter().cloned());
        t.extend(light_textures(scene, ls));
    }
    if lit(m) || mesh.gltf.is_some() {
        u.push((
            "receiveShadow".into(),
            vec![f64::from(u8::from(mesh.receive_shadow))],
        ));
    }
    let mut st = state(
        side,
        &mesh.world,
        color,
        depth,
        color && m.transparent,
        m.wireframe,
    );
    st.topology = topology(mesh);
    Ok(DrawDesc {
        params: program(mesh, m, side, &ls.counts, target)?,
        uniforms: u,
        textures: t,
        state: st,
        object: item.object,
        mesh: item.mesh,
        range: item.range,
    })
}

fn shadow_draw(
    scene: &Scene3d,
    ls: &LightState,
    light: &Light3d,
    view: &ShadowView,
    item: &Item<'_>,
) -> Result<DrawDesc, String> {
    let mesh = &scene.objects[item.object].meshes[item.mesh];
    let side = item.material.side.shadow();
    let point = light.kind == LightKind::Point;
    let map = item.material.maps.color.as_ref();
    let params = if mesh.gltf.is_some() {
        super::gltf::material::shadow_parameters(mesh, item.material, side, point, &ls.counts)?
    } else if point {
        materials::distance(side, map.is_some(), &ls.counts)?
    } else {
        materials::depth_rgba(side, map.is_some(), &ls.counts)?
    };
    let mut u = Vec::new();
    let v = view.view();
    camera_uniforms(&mut u, &view.projection, &v, &view.world, false);
    let model_view = math::multiply(&v, &mesh.world);
    u.push(("modelViewMatrix".into(), model_view.to_vec()));
    u.push(("modelMatrix".into(), mesh.world.to_vec()));
    u.push(("opacity".into(), vec![1.0]));
    let mut textures = Vec::new();
    if mesh.gltf.is_some() {
        super::gltf::material::shadow_uniforms(&mut u, &mut textures, item.material);
        super::gltf::material::object_uniforms(&mut u, &mut textures, mesh);
    } else if let Some(map) = map {
        textures.push(("map".to_owned(), TextureRef::Map(map.clone())));
        u.push(("mapTransform".into(), map.transform.to_vec()));
    }
    if point {
        u.push(("referencePosition".into(), light.position().to_vec()));
        u.push(("nearDistance".into(), vec![view.near]));
        u.push(("farDistance".into(), vec![view.far]));
    }
    // Scene casters sample no map in the depth pass (it packs depth only);
    // glTF casters bind theirs for the alpha test.
    if mesh.gltf.is_none() {
        textures.clear();
    }
    let mut state = state(
        side,
        &mesh.world,
        false,
        true,
        false,
        item.material.wireframe,
    );
    state.shadow = true;
    Ok(DrawDesc {
        params,
        uniforms: u,
        textures,
        state,
        object: item.object,
        mesh: item.mesh,
        range: item.range,
    })
}

/// The main camera's frustum (`projectObject`).
fn camera_frustum(scene: &Scene3d) -> [[f64; 4]; 6] {
    let (w, h) = scene.canvas();
    frustum(&math::multiply(
        &scene.camera.projection_matrix(w, h),
        &scene.camera.view(),
    ))
}

/// The shared prepass of `layer`'s depth set: shadow maps, then depth.
pub fn prepass(layer: &Layer3d<'_>) -> Result<Vec<PassDesc>, String> {
    let scene = layer.scene;
    let ls = lights::state(scene);
    let mut passes = Vec::new();
    // `WebGLShadowMap.render(shadowsArray)`: visible casting lights in scene
    // order; casters are the set's visible meshes with `castShadow`.
    if ls.counts.shadow_map {
        for (i, light) in scene.lights.iter().enumerate() {
            if !(light.visible && light.cast_shadow) {
                continue;
            }
            let mut views = Vec::new();
            for view in lights::views(light) {
                let mut draws = Vec::new();
                let planes = frustum(&math::multiply(&view.projection, &view.view()));
                for &o in layer.depth_set {
                    for item in items(scene, o, &planes) {
                        let mesh = &scene.objects[o].meshes[item.mesh];
                        if mesh.cast_shadow {
                            draws.push(shadow_draw(scene, &ls, light, &view, &item)?);
                        }
                    }
                }
                views.push((view, draws));
            }
            passes.push(PassDesc::Shadow {
                light: i,
                size: lights::map_size(light.kind),
                views,
            });
        }
    }
    // Depth: opaque items of the set; a model writes depth only at layer
    // opacity ≥ 0.999 without an enabled mask.
    let light_uniforms = lights::uniforms(scene, &ls, &scene.camera.view());
    let planes = camera_frustum(scene);
    let mut draws = Vec::new();
    for &o in layer.depth_set {
        let object = &scene.objects[o];
        if !(object.layer_opacity >= 0.999 && object.occludes) {
            continue;
        }
        for item in items(scene, o, &planes) {
            if item.material.transparent
                || !item.material.depth_write
                || loose(&scene.objects[item.object].meshes[item.mesh])
            {
                continue;
            }
            draws.push(shading_draw(
                scene,
                &ls,
                &light_uniforms,
                &item,
                item.material.side,
                false,
                true,
            )?);
        }
    }
    // Points and lines keep their writes in the prepass too (its color is
    // discarded). With glTF drawables the draws follow three.js's lists:
    // opaque in painter order, then transmissive, then transparent.
    let gltf = scene
        .objects
        .iter()
        .any(|o| o.meshes.iter().any(|m| m.gltf.is_some()));
    if gltf {
        let mut all: Vec<Item<'_>> = Vec::new();
        for &o in layer.depth_set {
            let object = &scene.objects[o];
            if !(object.layer_opacity >= 0.999 && object.occludes) {
                continue;
            }
            all.extend(items(scene, o, &planes).into_iter().filter(|i| {
                !i.material.transparent
                    && i.material.depth_write
                    && !loose(&scene.objects[i.object].meshes[i.mesh])
            }));
        }
        all.extend(loose_items(scene, &planes, None));
        let is_loose = |i: &Item<'_>| loose(&scene.objects[i.object].meshes[i.mesh]);
        draws = list_draws_with(
            scene,
            &ls,
            &light_uniforms,
            &all,
            |i| i.material.depth_write,
            is_loose,
            super::gltf::material::Target::Canvas,
            (true, true, true),
        )?;
    }
    passes.push(PassDesc::Prepass(draws));
    Ok(passes)
}

/// The layer's color pass.
pub fn color(layer: &Layer3d<'_>) -> Result<PassDesc, String> {
    let scene = layer.scene;
    let ls = lights::state(scene);
    let light_uniforms = lights::uniforms(scene, &ls, &scene.camera.view());
    let clear = [0.0; 4];
    let own_depth =
        !scene.objects[layer.object].occludes || scene.objects[layer.object].layer_opacity < 0.999;
    let mut draws = Vec::new();
    if layer.depth_set.contains(&layer.object) {
        let all = items(scene, layer.object, &camera_frustum(scene));
        for item in all.iter().filter(|i| !i.material.transparent) {
            draws.push(shading_draw(
                scene,
                &ls,
                &light_uniforms,
                item,
                item.material.side,
                true,
                own_depth && item.material.depth_write,
            )?);
        }
        for item in all.iter().filter(|i| i.material.transparent) {
            if item.material.side == Side::Double {
                for side in [Side::Back, Side::Front] {
                    draws.push(shading_draw(
                        scene,
                        &ls,
                        &light_uniforms,
                        item,
                        side,
                        true,
                        false,
                    )?);
                }
            } else {
                draws.push(shading_draw(
                    scene,
                    &ls,
                    &light_uniforms,
                    item,
                    item.material.side,
                    true,
                    false,
                )?);
            }
        }
    }
    Ok(PassDesc::Color { clear, draws })
}

/// Whether an item belongs to the transmissive list (`material.transmission > 0`).
fn transmissive(m: &Material3d) -> bool {
    m.kind == MaterialKind::Physical && m.ext.as_ref().is_some_and(|e| e.source.transmission > 0.0)
}

/// The sort keys of a glTF item: material order, clip z of its bounding
/// sphere center, object id.
fn sort_keys(scene: &Scene3d, item: &Item<'_>, proj_screen: &Mat4) -> (u64, f64, u64) {
    let mesh = &scene.objects[item.object].meshes[item.mesh];
    let (center, id) = match &mesh.gltf {
        Some(g) => (g.sphere.map_or(mesh.geometry.sphere.0, |s| s.0), g.id),
        None => (mesh.geometry.sphere.0, item.mesh as u64),
    };
    let p = clip4(&mesh.world, [center[0], center[1], center[2], 1.0]);
    let z = clip4(proj_screen, p)[2];
    // `material.id`: creation order (objects in scene order, a mesh's
    // materials in slot order).
    let order = item.material.ext.as_ref().map_or_else(
        || {
            let slot = mesh
                .materials
                .iter()
                .position(|m| std::ptr::eq(m, item.material))
                .unwrap_or(0);
            ((item.object as u64) << 32) | ((item.mesh as u64) << 16) | slot as u64
        },
        |e| e.order,
    );
    (order, z, id)
}

/// The draws of render items in three.js's lists: opaque (painter order),
/// transmissive, then transparent (reverse painter order), double-sided
/// transparent items back face first.
#[allow(clippy::too_many_arguments)]
fn list_draws(
    scene: &Scene3d,
    ls: &LightState,
    light_uniforms: &[(String, Vec<f64>)],
    all: &[Item<'_>],
    depth: impl Fn(&Item<'_>) -> bool,
    target: super::gltf::material::Target,
    lists: (bool, bool, bool),
) -> Result<Vec<DrawDesc>, String> {
    list_draws_with(
        scene,
        ls,
        light_uniforms,
        all,
        depth,
        |_| true,
        target,
        lists,
    )
}

/// [`list_draws`] with each item's color writes.
#[allow(clippy::too_many_arguments)]
fn list_draws_with(
    scene: &Scene3d,
    ls: &LightState,
    light_uniforms: &[(String, Vec<f64>)],
    all: &[Item<'_>],
    depth: impl Fn(&Item<'_>) -> bool,
    color: impl Fn(&Item<'_>) -> bool,
    target: super::gltf::material::Target,
    lists: (bool, bool, bool),
) -> Result<Vec<DrawDesc>, String> {
    let (w, h) = scene.canvas();
    let proj_screen = math::multiply(&scene.camera.projection_matrix(w, h), &scene.camera.view());
    let mut keyed: Vec<((u64, f64, u64), &Item<'_>)> = all
        .iter()
        .map(|i| (sort_keys(scene, i, &proj_screen), i))
        .collect();
    let painter = |a: &(u64, f64, u64), b: &(u64, f64, u64)| {
        a.0.cmp(&b.0)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.2.cmp(&b.2))
    };
    keyed.sort_by(|a, b| painter(&a.0, &b.0));
    let mut draws = Vec::new();
    let emit = |item: &Item<'_>, draws: &mut Vec<DrawDesc>| -> Result<(), String> {
        let m = item.material;
        let sides: &[Side] = if m.transparent && m.side == Side::Double {
            &[Side::Back, Side::Front]
        } else {
            std::slice::from_ref(&m.side)
        };
        for &side in sides {
            draws.push(draw_to(
                scene,
                ls,
                light_uniforms,
                item,
                side,
                color(item),
                depth(item),
                target,
            )?);
        }
        Ok(())
    };
    if lists.0 {
        for (_, item) in keyed
            .iter()
            .filter(|(_, i)| !transmissive(i.material) && !i.material.transparent)
        {
            emit(item, &mut draws)?;
        }
    }
    // Transmissive and transparent lists: `reversePainterSortStable` (back
    // to front, then object id).
    let reverse = |list: &mut Vec<&((u64, f64, u64), &Item<'_>)>| {
        list.sort_by(|a, b| {
            b.0.1
                .partial_cmp(&a.0.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.2.cmp(&b.0.2))
        });
    };
    if lists.1 {
        let mut list: Vec<&((u64, f64, u64), &Item<'_>)> = keyed
            .iter()
            .filter(|(_, i)| transmissive(i.material))
            .collect();
        reverse(&mut list);
        for (_, item) in list {
            emit(item, &mut draws)?;
        }
    }
    if lists.2 {
        let mut list: Vec<&((u64, f64, u64), &Item<'_>)> = keyed
            .iter()
            .filter(|(_, i)| !transmissive(i.material) && i.material.transparent)
            .collect();
        reverse(&mut list);
        for (_, item) in list {
            emit(item, &mut draws)?;
        }
    }
    Ok(draws)
}

/// The transmission pass (`renderTransmissionPass`) for `items`, cleared
/// to `clear`.
fn transmission_pass(
    scene: &Scene3d,
    ls: &LightState,
    light_uniforms: &[(String, Vec<f64>)],
    items: &[Item<'_>],
    clear: [f64; 4],
    depth: impl Fn(&Item<'_>) -> bool + Copy,
) -> Result<Option<PassDesc>, String> {
    use super::gltf::material::Target;
    if !items.iter().any(|i| transmissive(i.material)) {
        return Ok(None);
    }
    let opaque = list_draws(
        scene,
        ls,
        light_uniforms,
        items,
        depth,
        Target::Transmission,
        (true, false, false),
    )?;
    // Back faces of double-sided transmissive items, in the transmissive
    // list's order (`reversePainterSortStable`).
    let (w, h) = scene.canvas();
    let proj_screen = math::multiply(&scene.camera.projection_matrix(w, h), &scene.camera.view());
    let mut list: Vec<((u64, f64, u64), &Item<'_>)> = items
        .iter()
        .filter(|i| transmissive(i.material) && i.material.side == Side::Double)
        .map(|i| (sort_keys(scene, i, &proj_screen), i))
        .collect();
    list.sort_by(|a, b| {
        b.0.1
            .partial_cmp(&a.0.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.2.cmp(&b.0.2))
    });
    let mut back = Vec::new();
    for (_, item) in list {
        back.push(draw_to(
            scene,
            ls,
            light_uniforms,
            item,
            Side::Back,
            true,
            depth(item),
            Target::Transmission,
        )?);
    }
    Ok(Some(PassDesc::Transmission {
        clear,
        opaque,
        back,
    }))
}

/// The layer's color pass, preceded by its transmission pass when the
/// layer's object has transmissive glTF materials. Objects with glTF
/// drawables use three.js's sorted lists; others [`color`].
pub fn color_passes(layer: &Layer3d<'_>) -> Result<Vec<PassDesc>, String> {
    let scene = layer.scene;
    let gltf = scene.objects[layer.object]
        .meshes
        .iter()
        .any(|m| m.gltf.is_some());
    let any_loose = scene.objects.iter().any(|o| o.meshes.iter().any(loose));
    if !gltf && !any_loose {
        return Ok(vec![color(layer)?]);
    }
    let ls = lights::state(scene);
    let light_uniforms = lights::uniforms(scene, &ls, &scene.camera.view());
    let background = layer.background.then_some(scene.background).flatten();
    let clear = [0.0; 4];
    let planes = camera_frustum(scene);
    let own = items(scene, layer.object, &planes);
    let mut all: Vec<Item<'_>> = if layer.depth_set.contains(&layer.object) {
        own
    } else {
        // The layer's meshes are hidden; its points and lines still draw.
        own.into_iter()
            .filter(|i| loose(&scene.objects[i.object].meshes[i.mesh]))
            .collect()
    };
    all.extend(loose_items(scene, &planes, Some(layer.object)));
    // Points and lines keep their depth writes in the color pass.
    let own_depth =
        !scene.objects[layer.object].occludes || scene.objects[layer.object].layer_opacity < 0.999;
    let depth = |i: &Item<'_>| {
        i.material.depth_write
            && (loose(&scene.objects[i.object].meshes[i.mesh])
                || (own_depth && i.object == layer.object && !i.material.transparent))
    };
    let mut passes = Vec::new();
    // The transmission target clears to the canvas clear color while it is
    // opaque (the GL clear color holds its sRGB-encoded values), else to
    // half-transparent white (premultiplied).
    let t_clear = background.map_or([0.5, 0.5, 0.5, 0.5], |c| [c[0], c[1], c[2], 1.0]);
    if let Some(p) = transmission_pass(scene, &ls, &light_uniforms, &all, t_clear, depth)? {
        passes.push(p);
    }
    let draws = list_draws(
        scene,
        &ls,
        &light_uniforms,
        &all,
        depth,
        super::gltf::material::Target::Canvas,
        (true, true, true),
    )?;
    passes.push(PassDesc::Color { clear, draws });
    Ok(passes)
}

/// `Vector4.applyMatrix4(m)` (no divide).
fn clip4(m: &Mat4, v: [f64; 4]) -> [f64; 4] {
    let (x, y, z, w) = (v[0], v[1], v[2], v[3]);
    [
        m[0] * x + m[4] * y + m[8] * z + m[12] * w,
        m[1] * x + m[5] * y + m[9] * z + m[13] * w,
        m[2] * x + m[6] * y + m[10] * z + m[14] * w,
        m[3] * x + m[7] * y + m[11] * z + m[15] * w,
    ]
}

/// `geometry.computeBoundingSphere().center`: the bounding box center.
fn sphere_center(g: &super::model::GeometryPayload) -> [f64; 3] {
    match g.bounds() {
        Some((lo, hi)) => [
            (f64::from(lo[0]) + f64::from(hi[0])) * 0.5,
            (f64::from(lo[1]) + f64::from(hi[1])) * 0.5,
            (f64::from(lo[2]) + f64::from(hi[2])) * 0.5,
        ],
        None => [0.0; 3],
    }
}

/// A whole-scene render (`renderer.render(scene, camera)` with automatic
/// clears, as a legacy scene layer renders): shadow maps of every caster,
/// then every visible object's items with depth writes, the background
/// cleared first.
pub fn whole_scene(scene: &Scene3d) -> Result<Vec<PassDesc>, String> {
    let all: Vec<usize> = (0..scene.objects.len()).collect();
    let layer = Layer3d {
        scene,
        object: 0,
        depth_set: &all,
        background: true,
    };
    let mut passes: Vec<PassDesc> = prepass(&layer)?
        .into_iter()
        .filter(|p| matches!(p, PassDesc::Shadow { .. }))
        .collect();
    let ls = lights::state(scene);
    let light_uniforms = lights::uniforms(scene, &ls, &scene.camera.view());
    let planes = camera_frustum(scene);
    if scene
        .objects
        .iter()
        .any(|o| o.meshes.iter().any(|m| m.gltf.is_some()))
    {
        let all: Vec<Item<'_>> = (0..scene.objects.len())
            .flat_map(|o| items(scene, o, &planes))
            .collect();
        let clear = [0.0; 4];
        // The background renders into the transmission target too
        // (linear, forced clear); without one it clears to white at 0.5.
        let t_clear = match scene.background {
            Some(c) => [c[0], c[1], c[2], 1.0],
            None => [0.5, 0.5, 0.5, 0.5],
        };
        if let Some(p) = transmission_pass(scene, &ls, &light_uniforms, &all, t_clear, |i| {
            i.material.depth_write
        })? {
            passes.push(p);
        }
        let draws = list_draws(
            scene,
            &ls,
            &light_uniforms,
            &all,
            |i| i.material.depth_write,
            super::gltf::material::Target::Canvas,
            (true, true, true),
        )?;
        passes.push(PassDesc::Color { clear, draws });
        return Ok(passes);
    }
    let mut opaque = Vec::new();
    let mut transparent = Vec::new();
    for o in 0..scene.objects.len() {
        for item in items(scene, o, &planes) {
            if item.material.transparent {
                transparent.push(item);
            } else {
                opaque.push(item);
            }
        }
    }
    // `reversePainterSortStable`: farther (larger clip z of the geometry's
    // bounding sphere center) first.
    let (w, h) = scene.canvas();
    let proj_screen = math::multiply(&scene.camera.projection_matrix(w, h), &scene.camera.view());
    let depth = |item: &Item<'_>| {
        let mesh = &scene.objects[item.object].meshes[item.mesh];
        let c = sphere_center(&mesh.geometry.geometry);
        let p = clip4(&mesh.world, [c[0], c[1], c[2], 1.0]);
        clip4(&proj_screen, p)[2]
    };
    let mut keyed: Vec<(f64, Item<'_>)> = transparent.into_iter().map(|i| (depth(&i), i)).collect();
    keyed.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let transparent: Vec<Item<'_>> = keyed.into_iter().map(|(_, i)| i).collect();
    let mut draws = Vec::new();
    for item in &opaque {
        draws.push(shading_draw(
            scene,
            &ls,
            &light_uniforms,
            item,
            item.material.side,
            true,
            item.material.depth_write,
        )?);
    }
    for item in &transparent {
        let sides: &[Side] = if item.material.side == Side::Double {
            &[Side::Back, Side::Front]
        } else {
            std::slice::from_ref(&item.material.side)
        };
        for &side in sides {
            draws.push(shading_draw(
                scene,
                &ls,
                &light_uniforms,
                item,
                side,
                true,
                item.material.depth_write,
            )?);
        }
    }
    let clear = [0.0; 4];
    passes.push(PassDesc::Color { clear, draws });
    Ok(passes)
}
