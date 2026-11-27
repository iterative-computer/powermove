//! Depth sets and composition ordering of 3D layers (plan section 8, "3D
//! layers"; `compositionDepthIds` and `compositionOrderedLayers` in
//! `core/scene3d/service.ts`), over one compositing pass in stack order
//! (index 0 is the top layer).
//!
//! **Depth sets.** A 3D object layer's set walks outward from the layer
//! through its neighbors in the ordered pass. Inactive layers of any type,
//! lights, and cameras do not end it; an active layer without a 3D role (a
//! 2D layer or a styled group, a compositing boundary) does. Models join
//! when their world opacity is above 0.001 and, when any layer of the
//! composition is soloed, when they or a group ancestor present in the pass
//! are soloed.
//!
//! **Ordering.** Within each run of consecutive 3D layers (objects,
//! lights, cameras), layers sort back to front by the camera-space depth
//! of their object origin (stable; lights, cameras, and objects missing
//! from the scene sort at depth 0). Runs are never reordered across 2D
//! layers.

use std::cmp::Ordering;
use std::ops::Range;

/// A layer's 3D role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Object,
    Light,
    Camera,
}

/// One entry of a compositing pass, as the depth-set walk sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassEntry {
    /// `PM.active(layer, time)`.
    pub active: bool,
    pub role: Option<Role>,
    /// The layer is soloed, or a group ancestor present in the same pass is.
    pub soloed: bool,
    /// `PM.worldOpacity(layer, time)` (objects only).
    pub opacity: f64,
}

/// `compositionDepthIds(layer)`: the pass indices in the depth set of the
/// object layer at `index`, ascending. `any_solo` is whether any layer of
/// the composition is soloed.
pub fn depth_set(pass: &[PassEntry], index: usize, any_solo: bool) -> Vec<usize> {
    let mut ids = Vec::new();
    let mut visit = |i: usize| -> bool {
        let l = pass[i];
        if !l.active {
            return true;
        }
        let Some(role) = l.role else {
            return false;
        };
        if role == Role::Object && (!any_solo || l.soloed) && l.opacity > 0.001 {
            ids.push(i);
        }
        true
    };
    let mut i = index as isize;
    while i >= 0 && visit(i as usize) {
        i -= 1;
    }
    let mut i = index + 1;
    while i < pass.len() && visit(i) {
        i += 1;
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The bottom member of a depth set in the ordered pass (the layer that
/// draws the scene background): the last pass entry in the set.
pub fn bottom(set: &[usize]) -> Option<usize> {
    set.iter().copied().max()
}

/// Consecutive runs of the pass: `(range, is_3d)`.
pub fn runs(is_3d: &[bool]) -> Vec<(Range<usize>, bool)> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < is_3d.len() {
        let kind = is_3d[start];
        let mut end = start + 1;
        while end < is_3d.len() && is_3d[end] == kind {
            end += 1;
        }
        out.push((start..end, kind));
        start = end;
    }
    out
}

/// `block.sort((a, b) => depth(b) − depth(a))`: a stable permutation of a
/// 3D run, nearest (largest camera-space z) first.
pub fn sort_run(depths: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..depths.len()).collect();
    order.sort_by(|&a, &b| {
        let d = depths[b] - depths[a];
        if d < 0.0 {
            Ordering::Less
        } else if d > 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    });
    order
}

/// The camera-space z of a world point: `point.applyMatrix4(view).z`
/// (`view` is `camera.matrixWorldInverse`, column-major).
pub fn view_z(view: &[f64; 16], p: [f64; 3]) -> f64 {
    let e = view;
    let w = 1.0 / (e[3] * p[0] + e[7] * p[1] + e[11] * p[2] + e[15]);
    (e[2] * p[0] + e[6] * p[1] + e[10] * p[2] + e[14]) * w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(active: bool, opacity: f64) -> PassEntry {
        PassEntry {
            active,
            role: Some(Role::Object),
            soloed: false,
            opacity,
        }
    }

    const TWO_D: PassEntry = PassEntry {
        active: true,
        role: None,
        soloed: false,
        opacity: 1.0,
    };

    #[test]
    fn barriers_split_sets_and_inactive_layers_do_not() {
        let light = PassEntry {
            role: Some(Role::Light),
            ..object(true, 1.0)
        };
        let inactive_2d = PassEntry {
            active: false,
            ..TWO_D
        };
        let pass = [
            object(true, 1.0),
            light,
            inactive_2d,
            object(true, 1.0),
            TWO_D,
            object(true, 1.0),
            object(true, 0.0005),
        ];
        assert_eq!(depth_set(&pass, 0, false), vec![0, 3]);
        assert_eq!(depth_set(&pass, 3, false), vec![0, 3]);
        assert_eq!(depth_set(&pass, 5, false), vec![5]);
        assert_eq!(bottom(&depth_set(&pass, 0, false)), Some(3));
    }

    #[test]
    fn solo_excludes_models_that_are_not_soloed() {
        let mut soloed = object(true, 1.0);
        soloed.soloed = true;
        let pass = [soloed, object(true, 1.0)];
        assert_eq!(depth_set(&pass, 0, true), vec![0]);
        assert_eq!(depth_set(&pass, 1, true), vec![0]);
    }

    #[test]
    fn runs_sort_nearest_first_and_stably() {
        assert_eq!(
            runs(&[true, true, false, true]),
            vec![(0..2, true), (2..3, false), (3..4, true)]
        );
        assert_eq!(sort_run(&[-5.0, -1.0, 0.0, -1.0]), vec![2, 1, 3, 0]);
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -6.0, 1.0,
        ];
        assert_eq!(view_z(&identity, [0.0, 0.0, 1.0]), -5.0);
    }
}
