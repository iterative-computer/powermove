//! Legacy 3D layer kinds (plan sections 8 and 10):
//!
//! - **`scene3d` layers** (`powermove.3d.scene`, read-only): the layer's
//!   own scene rendered whole (`SceneRuntime.render(targetW, targetH)`) at
//!   its size, at most 4,096 pixels a side, with its own background, then
//!   drawn through the layer's 2D transform.
//! - **OBJ mesh layers** (extension definitions with
//!   `renderer.kind === 'mesh'`): the compositor's fixed shading
//!   (`meshExtensionQuad`: its own camera from yaw, pitch, and distance, a
//!   key and a rim light, Reinhard and gamma 2.2) into a half-float target
//!   with a 16-bit depth buffer, cleared to the layer's background; without
//!   a mesh, the checkerboard placeholder of a missing definition.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use pm_eval::node;
use pm_eval::{JsResult, Scope};
use pm_geom3d::obj::{ObjMesh, parse_obj};
use pm_gpu::{Target, TargetPool};
use pm_model::Node;
use pm_model::json::Value;

use crate::assets::AssetResolver;
use crate::js;

/// The built-in legacy scene definition (`SCENE3D_DEFINITION`).
pub const SCENE3D_DEFINITION: &str = "powermove.3d.scene";

/// The largest legacy target edge (`Math.min(4096, …)`).
pub const MAX_TARGET: f64 = 4096.0;

/// What a mesh layer's definition snapshot holds for rendering
/// (`renderer.assetField` and the parameter defaults).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshDefinition {
    pub asset_field: String,
    /// `params[].k` to `params[].def`.
    pub defaults: HashMap<String, Value>,
}

/// The uniforms of the fixed mesh shading.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjMeshPlan {
    pub width: u32,
    pub height: u32,
    /// `None`: the asset has no mesh (the checkerboard placeholder).
    pub mesh: Option<Arc<ObjMesh>>,
    /// `u_yaw`, `u_pitch`, `u_distance`, `u_rotationX`, `u_rotationY`,
    /// `u_rotationZ`, `u_scale`, `u_roughness`, `u_metalness`.
    pub values: [f64; 9],
    pub object_color: [f64; 3],
    pub light_color: [f64; 3],
    pub background: [f64; 3],
}

/// The target size of an extension layer: `min(4096, max(2, round(w · s)))`
/// with `w = max(1, Number(d.w || comp.w || W))`.
pub fn target_size(d_size: f64, comp_size: f64, full: f64, scale: f64) -> (f64, f64) {
    let or = |v: f64, f: f64| if v.is_nan() || v == 0.0 { f } else { v };
    let w = js::max(1.0, or(or(d_size, comp_size), full));
    (w, js::min(MAX_TARGET, js::max(2.0, js::round(w * scale))))
}

type ObjKey = (PathBuf, u64, Option<std::time::SystemTime>);

fn obj_cache() -> &'static Mutex<HashMap<ObjKey, Arc<ObjMesh>>> {
    static C: OnceLock<Mutex<HashMap<ObjKey, Arc<ObjMesh>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The parsed OBJ mesh of a file (`asset.mesh`), cached per file version.
pub fn obj_mesh(path: &std::path::Path) -> Result<Arc<ObjMesh>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(m) = obj_cache().lock().unwrap().get(&key) {
        return Ok(m.clone());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mesh = Arc::new(parse_obj(&text)?);
    let mut c = obj_cache().lock().unwrap();
    c.retain(|k, _| k.0 != key.0);
    c.insert(key, mesh.clone());
    Ok(mesh)
}

/// `meshExtensionQuad`'s parameters and mesh for a mesh layer at `time`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_mesh_layer<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    definition: &MeshDefinition,
    time: f64,
    width: u32,
    height: u32,
    assets: &dyn AssetResolver,
) -> JsResult<Result<ObjMeshPlan, String>> {
    let d = node::member(Some(layer), "d");
    let params = node::member(d, "params");
    // `extensionParam`: the layer's property, the definition's default, or
    // the fallback.
    let param = |scope: &mut Scope<'_, 'd>, key: &str, fallback: Value| -> JsResult<Value> {
        if let Some(prop) = node::member(params, key).filter(|p| node::truthy(Some(p))) {
            let v = scope.channel_p(layer, Some(prop), time, key)?;
            return Ok(v.to_json().unwrap_or(Value::Null));
        }
        Ok(definition.defaults.get(key).cloned().unwrap_or(fallback))
    };
    let number = |v: &Value| -> f64 {
        let n = pm_eval::JsValue::from(v.clone()).to_number();
        if n.is_nan() { 0.0 } else { n }
    };
    let color = |v: &Value| -> [f64; 3] {
        let s = pm_eval::JsValue::from(v.clone())
            .to_js_string()
            .to_string_lossy();
        pm_color::standard::parse_hex(&s).rgb
    };
    let names: [(&str, f64); 9] = [
        ("yaw", 28.0),
        ("pitch", 18.0),
        ("distance", 4.8),
        ("rotationX", 0.0),
        ("rotationY", 25.0),
        ("rotationZ", 0.0),
        ("size", 1.35),
        ("roughness", 0.25),
        ("metalness", 0.7),
    ];
    let mut values = [0.0; 9];
    for (i, (k, fallback)) in names.iter().enumerate() {
        let mut v = number(&param(scope, k, Value::Number(*fallback))?);
        if *k == "rotationY" {
            let auto = param(scope, "autoRotate", Value::Bool(false))?;
            if pm_eval::JsValue::from(auto).truthy() {
                let speed = number(&param(scope, "speed", Value::Number(25.0))?);
                let from = node::to_number(node::member(Some(layer), "from"));
                v += (time - from) * speed;
            }
        }
        values[i] = v;
    }
    let s = |v: &str| Value::String(v.into());
    let object_color = color(&param(scope, "objectColor", s("#C7C4FF"))?);
    let light_color = color(&param(scope, "lightColor", s("#FFB36B"))?);
    let background = color(&param(scope, "background", s("#0C0D12"))?);
    // The mesh asset.
    let asset = node::string(node::member(
        node::member(d, "data"),
        &definition.asset_field,
    ))
    .map(|s| s.to_string_lossy())
    .unwrap_or_default();
    let root = scope.comp.root();
    let record = node::member(node::member(Some(root), "assets"), &asset);
    let mesh = match (asset.is_empty(), record) {
        (false, Some(record)) => match assets.path(&asset, Some(record)) {
            Some(path) => match obj_mesh(&path) {
                Ok(m) => Some(m),
                Err(e) => return Ok(Err(format!("the OBJ mesh {asset}: {e}"))),
            },
            None => None,
        },
        _ => None,
    };
    Ok(Ok(ObjMeshPlan {
        width,
        height,
        mesh,
        values,
        object_color,
        light_color,
        background,
    }))
}

const MESH_WGSL: &str = r#"
struct U {
    resolution: vec2<f32>,
    yaw: f32,
    pitch: f32,
    distance: f32,
    rotation_x: f32,
    rotation_y: f32,
    rotation_z: f32,
    scale: f32,
    roughness: f32,
    metalness: f32,
    _pad: f32,
    object_color: vec4<f32>,
    light_color: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world: vec3<f32>,
};
fn rx(a: f32) -> mat3x3<f32> { let c = cos(a); let s = sin(a); return mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, c, s), vec3<f32>(0.0, -s, c)); }
fn ry(a: f32) -> mat3x3<f32> { let c = cos(a); let s = sin(a); return mat3x3<f32>(vec3<f32>(c, 0.0, -s), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(s, 0.0, c)); }
fn rz(a: f32) -> mat3x3<f32> { let c = cos(a); let s = sin(a); return mat3x3<f32>(vec3<f32>(c, s, 0.0), vec3<f32>(-s, c, 0.0), vec3<f32>(0.0, 0.0, 1.0)); }
@vertex
fn vs(@location(0) a_position: vec3<f32>, @location(1) a_normal: vec3<f32>) -> Out {
    let model = rz(radians(u.rotation_z)) * rx(radians(u.rotation_x)) * ry(radians(u.rotation_y));
    let world = model * a_position * u.scale;
    let normal = normalize(model * a_normal);
    let yaw = radians(u.yaw);
    let pitch = radians(u.pitch);
    let camera = u.distance * vec3<f32>(cos(pitch) * sin(yaw), sin(pitch), cos(pitch) * cos(yaw));
    let forward = normalize(-camera);
    let right = normalize(cross(forward, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, forward);
    let rel = world - camera;
    let z = dot(rel, forward);
    let near = 0.05;
    let far = 100.0;
    let focal = 1.8;
    let aspect = u.resolution.x / max(u.resolution.y, 1.0);
    var p = vec4<f32>(dot(rel, right) * focal / aspect, dot(rel, up) * focal,
        ((far + near) / (far - near)) * z - (2.0 * far * near / (far - near)), z);
    // GL row order and depth range (pm_gpu::glsl::VERTEX_EPILOGUE).
    p.y = -p.y;
    p.z = (p.z + p.w) * 0.5;
    var o: Out;
    o.pos = p;
    o.normal = normal;
    o.world = world;
    return o;
}
@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let yaw = radians(u.yaw);
    let pitch = radians(u.pitch);
    let camera = u.distance * vec3<f32>(cos(pitch) * sin(yaw), sin(pitch), cos(pitch) * cos(yaw));
    let view = normalize(camera - in.world);
    let key = normalize(vec3<f32>(-3.5, 5.0, 4.0) - in.world);
    let rim = normalize(vec3<f32>(4.0, 2.5, -3.0) - in.world);
    let diffuse = max(dot(n, key), 0.0);
    let rim_light = pow(max(dot(n, rim), 0.0), 1.5);
    let half_vector = normalize(key + view);
    let power = mix(18.0, 180.0, 1.0 - clamp(u.roughness, 0.0, 1.0));
    let spec = pow(max(dot(n, half_vector), 0.0), power) * (0.12 + 2.4 * clamp(u.metalness, 0.0, 1.0));
    let fresnel = pow(1.0 - max(dot(n, view), 0.0), 4.0);
    let oc = u.object_color.rgb;
    let lc = u.light_color.rgb;
    var color = oc * (0.14 + diffuse * (1.0 - 0.65 * u.metalness));
    color = color + lc * spec + mix(oc, lc, vec3<f32>(0.45)) * fresnel * (0.25 + 0.7 * u.metalness);
    color = color + vec3<f32>(0.35, 0.42, 0.65) * rim_light * 0.2;
    color = color / (color + vec3<f32>(1.0));
    color = pow(color, vec3<f32>(1.0 / 2.2));
    return vec4<f32>(color, 1.0);
}
struct Quad {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
// `PM.VERT` over the unit quad (a triangle strip): `v_uv = a_pos`, GL clip
// y = 1 − 2·a.y, negated for GL row order. The varying is interpolated as
// WebGL interpolates it, so cell edges fall on the same pixels.
@vertex
fn vs_quad(@builtin(vertex_index) i: u32) -> Quad {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    var o: Quad;
    o.pos = vec4<f32>(a.x * 2.0 - 1.0, a.y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = a;
    return o;
}
// The missing-definition checkerboard (`void main(){ vec2 q=floor(uv*24.0); … }`).
@fragment
fn fs_missing(in: Quad) -> @location(0) vec4<f32> {
    let q = floor(in.uv * 24.0);
    let checker = (q.x + q.y) - 2.0 * floor((q.x + q.y) / 2.0);
    let a = vec3<f32>(0.055, 0.05, 0.07);
    let b = vec3<f32>(0.16, 0.09, 0.15);
    return vec4<f32>(mix(a, b, vec3<f32>(checker)), 1.0);
}
"#;

struct Pipelines {
    mesh: wgpu::RenderPipeline,
    missing: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

/// GPU state of the OBJ mesh layers.
#[derive(Default)]
pub struct MeshRenderer {
    pipelines: Option<Pipelines>,
    buffers: HashMap<usize, (Arc<ObjMesh>, wgpu::Buffer, wgpu::Buffer)>,
}

impl MeshRenderer {
    fn pipelines(&mut self, device: &wgpu::Device) -> &Pipelines {
        self.pipelines.get_or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("legacy mesh"),
                source: wgpu::ShaderSource::Wgsl(MESH_WGSL.into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("legacy mesh"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("legacy mesh"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let attr = |loc: u32| {
                [wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: loc,
                }]
            };
            let (a0, a1) = (attr(0), attr(1));
            let depth = wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth16Unorm,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            };
            let target = [Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })];
            let mesh = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("legacy mesh"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 12,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &a0,
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 12,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &a1,
                        }),
                    ],
                },
                // No culling (the compositor never enables CULL_FACE).
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(depth.clone()),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &target,
                }),
                multiview_mask: None,
                cache: None,
            });
            let missing = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("legacy mesh missing"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_quad"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    ..depth
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_missing"),
                    compilation_options: Default::default(),
                    targets: &target,
                }),
                multiview_mask: None,
                cache: None,
            });
            Pipelines {
                mesh,
                missing,
                layout,
            }
        })
    }

    /// Renders a mesh layer into a half-float target in GL row order.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        plan: &ObjMeshPlan,
        _samples: u32,
    ) -> Result<Target, String> {
        let (w, h) = (plan.width.max(1), plan.height.max(1));
        let mut data: Vec<f32> = vec![w as f32, h as f32];
        data.extend(plan.values.iter().map(|v| *v as f32));
        data.push(0.0);
        data.extend(plan.object_color.iter().map(|v| *v as f32));
        data.push(0.0);
        data.extend(plan.light_color.iter().map(|v| *v as f32));
        data.push(0.0);
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let ubuf = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("legacy mesh"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let buffers = plan.mesh.as_ref().map(|m| {
            let key = Arc::as_ptr(m) as usize;
            self.buffers
                .retain(|_, (mesh, _, _)| Arc::strong_count(mesh) > 1);
            let entry = self.buffers.entry(key).or_insert_with(|| {
                let make = |v: &[f32], label: &str| {
                    wgpu::util::DeviceExt::create_buffer_init(
                        device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some(label),
                            contents: &v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>(),
                            usage: wgpu::BufferUsages::VERTEX,
                        },
                    )
                };
                (
                    m.clone(),
                    make(&m.positions, "legacy mesh positions"),
                    make(&m.normals, "legacy mesh normals"),
                )
            });
            (entry.1.clone(), entry.2.clone(), m.vertex_count as u32)
        });
        let p = self.pipelines(device);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("legacy mesh"),
            layout: &p.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            }],
        });
        let target = pool.take(
            device,
            w,
            h,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = pool.take(
            device,
            w,
            h,
            wgpu::TextureFormat::Depth16Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let clear = match &buffers {
            Some(_) => wgpu::Color {
                r: plan.background[0],
                g: plan.background[1],
                b: plan.background[2],
                a: 1.0,
            },
            None => wgpu::Color::TRANSPARENT,
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("legacy mesh"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &bind, &[]);
            match &buffers {
                Some((positions, normals, count)) => {
                    pass.set_pipeline(&p.mesh);
                    pass.set_vertex_buffer(0, positions.slice(..));
                    pass.set_vertex_buffer(1, normals.slice(..));
                    pass.draw(0..*count, 0..1);
                }
                None => {
                    pass.set_pipeline(&p.missing);
                    pass.draw(0..4, 0..1);
                }
            }
        }
        let _ = queue;
        pool.give(depth);
        Ok(target)
    }
}

/// An extension layer's source and its quad size.
pub(crate) type ExtensionQuad = (crate::plan::Source, f64, f64);

/// Plans an extension layer that is not a 3D role: a legacy `scene3d`
/// layer, or a mesh layer whose definition snapshot is known. `None` for
/// other definitions. The result is the source, drawn as a `w` × `h` quad
/// through the layer's transform (`fromFbo`, anchor at the origin).
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_extension<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    d: &pm_eval::Content,
    definition: &str,
    time: f64,
    output: (u32, u32),
    scale: (f64, f64),
    assets: &dyn AssetResolver,
    options: &super::Options3d,
) -> JsResult<Option<Result<ExtensionQuad, String>>> {
    let mesh = options.mesh_definitions.get(definition);
    if definition != SCENE3D_DEFINITION && mesh.is_none() {
        return Ok(None);
    }
    let (cw, ch) = scope.comp_size();
    let (w, tw) = target_size(
        js::get(d, "w").to_number(),
        cw,
        f64::from(output.0),
        scale.0,
    );
    let (h, th) = target_size(
        js::get(d, "h").to_number(),
        ch,
        f64::from(output.1),
        scale.1,
    );
    let (tw, th) = (tw as u32, th as u32);
    let id = js::layer_id(layer);
    let plan = if let Some(definition) = mesh {
        match plan_mesh_layer(scope, layer, definition, time, tw, th, assets)? {
            Ok(m) => super::Object3dPlan {
                layer: id,
                scene: Arc::new(super::model::Scene3d::empty(tw, th)),
                kind: super::Render3d::ObjMesh(m),
            },
            Err(e) => return Ok(Some(Err(e))),
        }
    } else {
        match super::layers::legacy_scene(scope, layer, time, tw, th, assets)? {
            Ok(scene) => super::Object3dPlan {
                layer: id,
                scene: Arc::new(scene),
                kind: super::Render3d::Whole,
            },
            Err(e) => return Ok(Some(Err(e))),
        }
    };
    Ok(Some(Ok((
        crate::plan::Source::Object3d(Box::new(plan)),
        w,
        h,
    ))))
}
