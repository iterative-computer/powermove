//! The reads behind parenting and grouping edits (`legacy/core/parenting.ts`):
//! world aims of cameras and lights, and the decomposition of a world pose
//! into a layer's editable channels in a new parent space.
//!
//! The edits themselves (`preserveParentPose`, `preserveWorldPose`,
//! `preserveWorldAim`, `applyPoseValues`) write channels and belong to
//! `pm-edit`; they call these functions for the values to write.
//!
//! Aims are stored in the layer's parent space (`targetX`, `targetY`,
//! `targetZ` of a camera's or light's content), so reparenting keeps the
//! world aim by rewriting them with [`Scope::aim_values3d`].

use pm_model::Node;

use crate::js_sem::{JsResult, JsThrow};
use crate::math;
use crate::node;
use crate::scope::{Scope, js_hypot};
use crate::space3d::{Mat4, inverse3d};
use crate::transform::{Mat2d, mul};

/// The 3D layer roles (`layer3DRole`), by `d.definition`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role3d {
    Object,
    Light,
    Camera,
}

impl Role3d {
    pub const OBJECT: &'static str = "powermove.3d.object";
    pub const LIGHT: &'static str = "powermove.3d.light";
    pub const CAMERA: &'static str = "powermove.3d.camera";

    /// `layer3DRole(layer)`.
    pub fn of(layer: &Node) -> Option<Role3d> {
        let def = node::member(node::member(Some(layer), "d"), "definition");
        if node::is_str(def, Self::OBJECT) {
            Some(Role3d::Object)
        } else if node::is_str(def, Self::LIGHT) {
            Some(Role3d::Light)
        } else if node::is_str(def, Self::CAMERA) {
            Some(Role3d::Camera)
        } else {
            None
        }
    }

    /// The role's name (`object`, `light`, `camera`): the content key and the
    /// channel prefix.
    pub fn name(self) -> &'static str {
        match self {
            Role3d::Object => "object",
            Role3d::Light => "light",
            Role3d::Camera => "camera",
        }
    }
}

/// A camera's or light's role and its three aim channels.
pub type AimProperties<'a> = (Role3d, [Option<&'a Node>; 3]);

/// The aim channel keys, in order.
pub const AIM_KEYS: [&str; 3] = ["targetX", "targetY", "targetZ"];

/// `aimProperties3D(layer)`: the aim channels of a camera or light
/// (`layer.d.data[role].p.target{X,Y,Z}`), empty for other layers. Throws when
/// a camera or light has no `d.data[role]` (reading `p` of `undefined`).
pub fn aim_properties3d(layer: &Node) -> JsResult<Option<AimProperties<'_>>> {
    let role = match Role3d::of(layer) {
        Some(r @ (Role3d::Camera | Role3d::Light)) => r,
        _ => return Ok(None),
    };
    let d = node::get_checked(Some(layer), b"d")?;
    let data = node::get_checked(d.node(), b"data")?;
    let content = node::get_checked(data.node(), role.name().as_bytes())?;
    let p = node::get_checked(content.node(), b"p")?;
    let prop = |key: &str| match &p {
        node::Got::Node(p) => node::member(Some(p), key),
        _ => None,
    };
    let props = [prop(AIM_KEYS[0]), prop(AIM_KEYS[1]), prop(AIM_KEYS[2])];
    // Reading a member of a primitive `p` gives `undefined`; of `undefined`, a throw.
    if p.is_nullish() {
        return Err(JsThrow::type_error(
            "Cannot read properties of undefined (reading 'targetX')",
        ));
    }
    Ok(Some((role, props)))
}

/// `transformAim(matrix, point)`.
pub fn transform_aim(m: &Mat4, p: &[f64; 3]) -> [f64; 3] {
    let at = |i: usize| m[i] * p[0] + m[i + 4] * p[1] + m[i + 8] * p[2] + m[i + 12];
    [at(0), at(1), at(2)]
}

/// `worldPoseValues`' 2D answer: the channels that reproduce a world matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose2d {
    /// `position.x`, `position.y`.
    pub position: [f64; 2],
    /// `scale.x`, `scale.y` (percent).
    pub scale: [f64; 2],
    /// Degrees.
    pub rotation: f64,
    /// Degrees.
    pub skew: f64,
}

impl Pose2d {
    pub fn channels(&self) -> [(&'static str, f64); 6] {
        [
            ("position.x", self.position[0]),
            ("position.y", self.position[1]),
            ("scale.x", self.scale[0]),
            ("scale.y", self.scale[1]),
            ("rotation", self.rotation),
            ("skew", self.skew),
        ]
    }
}

impl<'d> Scope<'_, 'd> {
    /// `worldAim3D(PM, layer, time)`: a camera's or light's aim point in
    /// world space (the stored aim is in its parent's space), or `None` for
    /// other layers.
    pub fn world_aim3d(&mut self, layer: &'d Node, time: f64) -> JsResult<Option<[f64; 3]>> {
        let Some((role, props)) = aim_properties3d(layer)? else {
            return Ok(None);
        };
        let mut point = [0.0; 3];
        for (i, key) in AIM_KEYS.iter().enumerate() {
            let path = format!("{}.{key}", role.name());
            point[i] = self.channel_p(layer, props[i], time, &path)?.to_number();
        }
        let parent = self.parent3d(layer, time, None)?;
        Ok(Some(transform_aim(&parent, &point)))
    }

    /// `aimValues3D(PM, layer, world, time, parent)`: the stored aim that
    /// points at `world` from the parent space `parent` (default: the
    /// layer's current one). Throws for a zero-scale parent.
    pub fn aim_values3d(
        &mut self,
        layer: &'d Node,
        world: &[f64; 3],
        time: f64,
        parent: Option<&Mat4>,
    ) -> JsResult<[f64; 3]> {
        let parent = match parent {
            Some(p) => *p,
            None => self.parent3d(layer, time, None)?,
        };
        let inverse = inverse3d(&parent).ok_or_else(|| {
            JsThrow::error(
                "Increase the camera or light parent’s zero scale before changing its hierarchy",
            )
        })?;
        Ok(transform_aim(&inverse, world))
    }

    /// `worldPoseValues(PM, layer, world, time, p, allowZero)` for a 2D world
    /// matrix: the channels that place the layer at `world` inside the
    /// parent space `parent` (default `transformParentMatrix`). Throws the
    /// legacy messages for a zero-scale parent or layer; with `allow_zero`, a
    /// layer collapsed to a line or point keeps what can be recovered.
    pub fn world_pose_values(
        &mut self,
        layer: &'d Node,
        world: &Mat2d,
        time: f64,
        parent: Option<&Mat2d>,
        allow_zero: bool,
    ) -> JsResult<Pose2d> {
        let p = match parent {
            Some(p) => *p,
            None => self.transform_parent_matrix(layer, time, None)?,
        };
        let determinant = p[0] * p[3] - p[1] * p[2];
        if determinant.abs() < 1e-10 {
            return Err(JsThrow::error(
                "This parent has zero scale. Increase its scale before parenting.",
            ));
        }
        let inverse = [
            p[3] / determinant,
            -p[1] / determinant,
            -p[2] / determinant,
            p[0] / determinant,
            (p[2] * p[5] - p[3] * p[4]) / determinant,
            (p[1] * p[4] - p[0] * p[5]) / determinant,
        ];
        let [a, b, c, d, x, y] = mul(&inverse, world);
        let sx = js_hypot(&[a, b]);
        let det = a * d - b * c;
        let deg = |r: f64| r * 180.0 / core::f64::consts::PI;
        if sx < 1e-10 || det.abs() < 1e-10 {
            if !allow_zero {
                return Err(JsThrow::error(
                    "Increase the child’s zero scale before changing its parent.",
                ));
            }
            let column_y = js_hypot(&[c, d]);
            if sx >= 1e-10 && column_y >= 1e-10 {
                return Err(JsThrow::error(
                    "Increase the group’s zero scale before ungrouping",
                ));
            }
            let ax = self.num(layer, "anchor.x", time)?;
            let ay = self.num(layer, "anchor.y", time)?;
            let rotation = if sx >= 1e-10 {
                deg(math::atan2(b, a))
            } else if column_y >= 1e-10 {
                deg(math::atan2(-c, d))
            } else {
                self.num(layer, "rotation", time)?
            };
            return Ok(Pose2d {
                position: [x + a * ax + c * ay, y + b * ax + d * ay],
                scale: [sx * 100.0, column_y * 100.0],
                rotation,
                skew: 0.0,
            });
        }
        let sy = det / sx;
        let ax = self.num(layer, "anchor.x", time)?;
        let ay = self.num(layer, "anchor.y", time)?;
        Ok(Pose2d {
            position: [x + a * ax + c * ay, y + b * ax + d * ay],
            scale: [sx * 100.0, sy * 100.0],
            rotation: deg(math::atan2(b, a)),
            skew: deg(math::atan((a * c + b * d) / (sx * sy))),
        })
    }

    /// `is3DLayer ? world3D : worldMatrix` (`worldPose` in `layer-groups.ts`),
    /// as the 4×4 or the 2×3 matrix.
    pub fn world_pose(&mut self, layer: &'d Node, time: f64) -> JsResult<WorldPose> {
        if self.is_3d_layer(layer)? {
            Ok(WorldPose::Space(self.world3d(layer, time, false)?))
        } else {
            Ok(WorldPose::Flat(self.world_matrix(layer, time)?))
        }
    }
}

/// A layer's world pose: flat (2×3) or in the 2.5D space (4×4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldPose {
    Flat(Mat2d),
    Space(Mat4),
}
