//! `WebGLLights` and the light shadows of three.js r180 as the runtime
//! configures them (`sync`: map size 1024², bias −0.0002, normal bias
//! 0.025; the sun's orthographic frustum ±10, near 0.01, far 200; spot and
//! point cameras near 0.5, far `distance || 500`; the point light's 4:2
//! cube atlas).

use crate::three::materials::LightCounts;
use crate::three::math::{self, Mat4, Vec3};

use super::model::{Light3d, LightKind, Scene3d};

/// `shadow.mapSize`.
pub const SHADOW_MAP_SIZE: u32 = 1024;
pub const SHADOW_BIAS: f64 = -0.0002;
pub const SHADOW_NORMAL_BIAS: f64 = 0.025;
/// `shadow.radius` and `shadow.intensity` (defaults).
pub const SHADOW_RADIUS: f64 = 1.0;
pub const SHADOW_INTENSITY: f64 = 1.0;

/// The lights a render sees (visible ones), sorted as `WebGLLights.setup`
/// sorts them (shadow-casting first, stable), with their uniform indices.
#[derive(Debug, Clone, PartialEq)]
pub struct LightState {
    /// Indices into `scene.lights`, in uniform order.
    pub order: Vec<usize>,
    pub counts: LightCounts,
    /// Per light in `order`: its index within its class's uniform array.
    pub slot: Vec<u32>,
}

/// The light state of `scene` (the hemisphere light always counts).
pub fn state(scene: &Scene3d) -> LightState {
    let mut order: Vec<usize> = (0..scene.lights.len())
        .filter(|&i| scene.lights[i].visible)
        .collect();
    // `shadowCastingAndTexturingLightsFirst` (no light maps): stable.
    order.sort_by_key(|&i| u8::from(!scene.lights[i].cast_shadow));
    let mut counts = LightCounts {
        hemi: 1,
        ..LightCounts::default()
    };
    let mut slot = Vec::with_capacity(order.len());
    let mut shadows = 0;
    for &i in &order {
        let l = &scene.lights[i];
        let cast = l.cast_shadow;
        let index = match l.kind {
            LightKind::Directional => {
                counts.directional += 1;
                counts.directional_shadows += u32::from(cast);
                counts.directional - 1
            }
            LightKind::Point => {
                counts.point += 1;
                counts.point_shadows += u32::from(cast);
                counts.point - 1
            }
            LightKind::Spot => {
                counts.spot += 1;
                counts.spot_shadows += u32::from(cast);
                counts.spot - 1
            }
            LightKind::RectArea => {
                counts.rect_area += 1;
                counts.rect_area - 1
            }
        };
        if cast {
            shadows += 1;
        }
        slot.push(index);
    }
    counts.shadow_map = scene.shadows && shadows > 0;
    LightState {
        order,
        counts,
        slot,
    }
}

/// A shadow camera for one render of a shadow map (one viewport).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowView {
    /// `shadow.camera.matrixWorld`.
    pub world: Mat4,
    pub projection: Mat4,
    /// Viewport in shadow-map pixels (GL convention: from the bottom).
    pub viewport: [u32; 4],
    pub near: f64,
    pub far: f64,
}

impl ShadowView {
    pub fn view(&self) -> Mat4 {
        math::invert(&self.world)
    }
}

/// A shadow map's size (`mapSize × frameExtents`).
pub fn map_size(kind: LightKind) -> (u32, u32) {
    match kind {
        LightKind::Point => (SHADOW_MAP_SIZE * 4, SHADOW_MAP_SIZE * 2),
        _ => (SHADOW_MAP_SIZE, SHADOW_MAP_SIZE),
    }
}

fn camera_far(light: &Light3d) -> f64 {
    // `light.distance || camera.far` with the camera's initial far of 500.
    if light.distance != 0.0 && !light.distance.is_nan() {
        light.distance
    } else {
        500.0
    }
}

/// `Object3D.lookAt` for a camera with `up`, then `updateMatrixWorld`.
fn look(position: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    math::camera_world(position, target, up)
}

/// The shadow camera renders of `light` (`shadow.updateMatrices` per
/// viewport), in viewport order.
pub fn views(light: &Light3d) -> Vec<ShadowView> {
    let position = light.position();
    match light.kind {
        LightKind::Directional => {
            // `OrthographicCamera(-10, 10, 10, -10, 0.01, 200)`, zoom 1.
            let (l, r, t, b, near, far) = (-10.0f64, 10.0f64, 10.0f64, -10.0f64, 0.01, 200.0);
            let dx = (r - l) / 2.0;
            let dy = (t - b) / 2.0;
            let cx = (r + l) / 2.0;
            let cy = (t + b) / 2.0;
            vec![ShadowView {
                world: look(position, light.target, [0.0, 1.0, 0.0]),
                projection: math::make_orthographic(cx - dx, cx + dx, cy + dy, cy - dy, near, far),
                viewport: [0, 0, SHADOW_MAP_SIZE, SHADOW_MAP_SIZE],
                near,
                far,
            }]
        }
        LightKind::Spot => {
            // `SpotLightShadow`: fov = RAD2DEG · 2 · angle · focus, aspect 1.
            let fov = (180.0 / std::f64::consts::PI) * 2.0 * light.angle * 1.0;
            let (near, far) = (0.5, camera_far(light));
            vec![ShadowView {
                world: look(position, light.target, [0.0, 1.0, 0.0]),
                projection: math::perspective(fov, 1.0, 1.0, near, far),
                viewport: [0, 0, SHADOW_MAP_SIZE, SHADOW_MAP_SIZE],
                near,
                far,
            }]
        }
        LightKind::Point => {
            let (near, far) = (0.5, camera_far(light));
            let projection = math::perspective(90.0, 1.0, 1.0, near, far);
            let directions: [Vec3; 6] = [
                [1.0, 0.0, 0.0],
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, -1.0],
                [0.0, 1.0, 0.0],
                [0.0, -1.0, 0.0],
            ];
            let ups: [Vec3; 6] = [
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, -1.0],
            ];
            let viewports: [[u32; 2]; 6] = [[2, 1], [0, 1], [3, 1], [1, 1], [3, 0], [1, 0]];
            (0..6)
                .map(|i| {
                    let d = directions[i];
                    let target = [position[0] + d[0], position[1] + d[1], position[2] + d[2]];
                    ShadowView {
                        world: look(position, target, ups[i]),
                        projection,
                        viewport: [
                            SHADOW_MAP_SIZE * viewports[i][0],
                            SHADOW_MAP_SIZE * viewports[i][1],
                            SHADOW_MAP_SIZE,
                            SHADOW_MAP_SIZE,
                        ],
                        near,
                        far,
                    }
                })
                .collect()
        }
        LightKind::RectArea => Vec::new(),
    }
}

/// `shadow.matrix` after the last `updateMatrices`: the biased
/// projection-view (sun, spot), or the light's negative translation
/// (point).
pub fn shadow_matrix(light: &Light3d) -> Mat4 {
    let p = light.position();
    match light.kind {
        LightKind::Point => {
            let mut m = math::IDENTITY;
            m[12] = -p[0];
            m[13] = -p[1];
            m[14] = -p[2];
            m
        }
        _ => {
            let v = views(light);
            let Some(v) = v.first() else {
                return math::IDENTITY;
            };
            let proj_screen = math::multiply(&v.projection, &v.view());
            // `set(0.5, 0, 0, 0.5, 0, 0.5, 0, 0.5, 0, 0, 0.5, 0.5, 0, 0, 0, 1)`
            // (row-major arguments).
            let bias = [
                0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 0.5, 1.0,
            ];
            math::multiply(&bias, &proj_screen)
        }
    }
}

/// `Vector3.applyMatrix4(m)` (with the perspective divide).
pub fn apply(m: &Mat4, v: Vec3) -> Vec3 {
    let (x, y, z) = (v[0], v[1], v[2]);
    let w = 1.0 / (m[3] * x + m[7] * y + m[11] * z + m[15]);
    [
        (m[0] * x + m[4] * y + m[8] * z + m[12]) * w,
        (m[1] * x + m[5] * y + m[9] * z + m[13]) * w,
        (m[2] * x + m[6] * y + m[10] * z + m[14]) * w,
    ]
}

/// `Matrix4.extractRotation(m)`.
fn extract_rotation(m: &Mat4) -> Mat4 {
    let len = |i: usize| (m[i] * m[i] + m[i + 1] * m[i + 1] + m[i + 2] * m[i + 2]).sqrt();
    let (sx, sy, sz) = (1.0 / len(0), 1.0 / len(4), 1.0 / len(8));
    [
        m[0] * sx,
        m[1] * sx,
        m[2] * sx,
        0.0,
        m[4] * sy,
        m[5] * sy,
        m[6] * sy,
        0.0,
        m[8] * sz,
        m[9] * sz,
        m[10] * sz,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

/// The light and shadow uniforms of a render with `view`
/// (`camera.matrixWorldInverse`), by GL name.
pub fn uniforms(scene: &Scene3d, state: &LightState, view: &Mat4) -> Vec<(String, Vec<f64>)> {
    let mut u: Vec<(String, Vec<f64>)> = Vec::new();
    let (sky, ground, intensity) = scene.hemisphere;
    u.push((
        "hemisphereLights[0].skyColor".into(),
        sky.iter().map(|v| v * intensity).collect(),
    ));
    u.push((
        "hemisphereLights[0].groundColor".into(),
        ground.iter().map(|v| v * intensity).collect(),
    ));
    u.push((
        "hemisphereLights[0].direction".into(),
        math::transform_direction([0.0, 1.0, 0.0], view).to_vec(),
    ));
    u.push(("ambientLightColor".into(), vec![0.0; 3]));
    u.push(("lightProbe".into(), vec![0.0; 27]));
    let shadow_struct = |prefix: String, u: &mut Vec<(String, Vec<f64>)>| {
        u.push((format!("{prefix}.shadowIntensity"), vec![SHADOW_INTENSITY]));
        u.push((format!("{prefix}.shadowBias"), vec![SHADOW_BIAS]));
        u.push((
            format!("{prefix}.shadowNormalBias"),
            vec![SHADOW_NORMAL_BIAS],
        ));
        u.push((format!("{prefix}.shadowRadius"), vec![SHADOW_RADIUS]));
        u.push((
            format!("{prefix}.shadowMapSize"),
            vec![f64::from(SHADOW_MAP_SIZE), f64::from(SHADOW_MAP_SIZE)],
        ));
    };
    for (k, &i) in state.order.iter().enumerate() {
        let l = &scene.lights[i];
        let n = state.slot[k];
        let color: Vec<f64> = l.color.iter().map(|c| c * l.intensity).collect();
        let position = l.position();
        let direction = || {
            math::transform_direction(
                [
                    position[0] - l.target[0],
                    position[1] - l.target[1],
                    position[2] - l.target[2],
                ],
                view,
            )
            .to_vec()
        };
        match l.kind {
            LightKind::Directional => {
                u.push((format!("directionalLights[{n}].direction"), direction()));
                u.push((format!("directionalLights[{n}].color"), color));
                if l.cast_shadow {
                    shadow_struct(format!("directionalLightShadows[{n}]"), &mut u);
                    u.push((
                        format!("directionalShadowMatrix[{n}]"),
                        shadow_matrix(l).to_vec(),
                    ));
                }
            }
            LightKind::Spot => {
                u.push((
                    format!("spotLights[{n}].position"),
                    apply(view, position).to_vec(),
                ));
                u.push((format!("spotLights[{n}].direction"), direction()));
                u.push((format!("spotLights[{n}].color"), color));
                u.push((format!("spotLights[{n}].distance"), vec![l.distance]));
                u.push((format!("spotLights[{n}].coneCos"), vec![libm::cos(l.angle)]));
                u.push((
                    format!("spotLights[{n}].penumbraCos"),
                    vec![libm::cos(l.angle * (1.0 - l.penumbra))],
                ));
                u.push((format!("spotLights[{n}].decay"), vec![l.decay]));
                if l.cast_shadow {
                    shadow_struct(format!("spotLightShadows[{n}]"), &mut u);
                    u.push((format!("spotLightMatrix[{n}]"), shadow_matrix(l).to_vec()));
                }
            }
            LightKind::Point => {
                u.push((
                    format!("pointLights[{n}].position"),
                    apply(view, position).to_vec(),
                ));
                u.push((format!("pointLights[{n}].color"), color));
                u.push((format!("pointLights[{n}].distance"), vec![l.distance]));
                u.push((format!("pointLights[{n}].decay"), vec![l.decay]));
                if l.cast_shadow {
                    let prefix = format!("pointLightShadows[{n}]");
                    shadow_struct(prefix.clone(), &mut u);
                    u.push((format!("{prefix}.shadowCameraNear"), vec![0.5]));
                    u.push((format!("{prefix}.shadowCameraFar"), vec![camera_far(l)]));
                    u.push((format!("pointShadowMatrix[{n}]"), shadow_matrix(l).to_vec()));
                }
            }
            LightKind::RectArea => {
                u.push((format!("rectAreaLights[{n}].color"), color));
                u.push((
                    format!("rectAreaLights[{n}].position"),
                    apply(view, position).to_vec(),
                ));
                let rotation = extract_rotation(&math::multiply(view, &l.world));
                u.push((
                    format!("rectAreaLights[{n}].halfWidth"),
                    apply(&rotation, [l.width * 0.5, 0.0, 0.0]).to_vec(),
                ));
                u.push((
                    format!("rectAreaLights[{n}].halfHeight"),
                    apply(&rotation, [0.0, l.height * 0.5, 0.0]).to_vec(),
                ));
            }
        }
    }
    u
}

/// The sampler uniform of a shadow-casting light (`directionalShadowMap[n]`
/// and the like).
pub fn shadow_sampler(light: &Light3d, slot: u32) -> String {
    match light.kind {
        LightKind::Directional => format!("directionalShadowMap[{slot}]"),
        LightKind::Spot => format!("spotShadowMap[{slot}]"),
        LightKind::Point => format!("pointShadowMap[{slot}]"),
        LightKind::RectArea => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::three::materials::Side;

    fn light(kind: LightKind, cast: bool) -> Light3d {
        let mut world = math::IDENTITY;
        world[12] = -3.0;
        world[13] = 5.0;
        world[14] = 4.0;
        Light3d {
            id: String::new(),
            kind,
            world,
            target: [0.0; 3],
            color: [1.0; 3],
            intensity: 3.0,
            distance: 0.0,
            decay: 2.0,
            angle: std::f64::consts::FRAC_PI_4,
            penumbra: 0.4,
            width: 1.0,
            height: 1.0,
            visible: true,
            cast_shadow: cast,
        }
    }

    #[test]
    fn casting_lights_sort_first_and_point_atlas_is_four_by_two() {
        let _ = Side::Front;
        let scene = Scene3d {
            width: 4,
            height: 4,
            camera: super::super::model::Camera3d {
                world: math::IDENTITY,
                projection: super::super::model::Projection::Perspective {
                    fov: 45.0,
                    zoom: 1.0,
                    near: 0.1,
                    far: 10.0,
                },
            },
            exposure: 1.0,
            hemisphere: ([1.0; 3], [0.35; 3], 0.5),
            lights: vec![
                light(LightKind::Directional, false),
                light(LightKind::Point, true),
                light(LightKind::Directional, true),
            ],
            shadows: true,
            background: None,
            objects: Vec::new(),
            look: Default::default(),
            warnings: Vec::new(),
        };
        let s = state(&scene);
        assert_eq!(s.order, vec![1, 2, 0]);
        assert_eq!(s.slot, vec![0, 0, 1]);
        assert_eq!(s.counts.directional, 2);
        assert_eq!(s.counts.directional_shadows, 1);
        assert!(s.counts.shadow_map);
        assert_eq!(map_size(LightKind::Point), (4096, 2048));
        let v = views(&scene.lights[1]);
        assert_eq!(v.len(), 6);
        assert_eq!(v[0].viewport, [2048, 1024, 1024, 1024]);
        assert_eq!(v[1].far, 500.0);
    }
}
