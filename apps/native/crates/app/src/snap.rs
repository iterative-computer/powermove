//! Move snapping (viewer.ts `snapBox`): the moving selection's box corners
//! and centre align with the composition's and other layers' within 8
//! display points per axis; ties go to the shorter guide.
use super::{GizmoSurface, Pose, transform};
use gpui::*;
use pm_kernel::QueryMirror;

pub const DISTANCE: f64 = 8.;
/// A guide from where a moved point lands to what it met.
pub type Guide = ([f64; 2], [f64; 2]);
/// Layers read for candidates; beyond this a huge composition snaps to its
/// frame and the first layers only.
const MAX_LAYERS: usize = 400;

fn box_of(points: impl Iterator<Item = [f64; 2]>) -> Option<[f64; 4]> {
    points.fold(None, |b, p| {
        Some(match b {
            None => [p[0], p[1], p[0], p[1]],
            Some([x0, y0, x1, y1]) => [x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1])],
        })
    })
}
fn corners_and_centre(b: [f64; 4]) -> [[f64; 2]; 5] {
    let [x0, y0, x1, y1] = b;
    [
        [x0, y0],
        [x1, y0],
        [x1, y1],
        [x0, y1],
        [(x0 + x1) / 2., (y0 + y1) / 2.],
    ]
}
/// What a move can snap to: the composition box and the other active layers.
pub fn points(p: &pm_kernel::Published, moving: &[String], time: f64) -> Option<Vec<[f64; 2]>> {
    let root = p.snapshot.root()?;
    // The composition the viewer renders (a root-level project is its own).
    let comp = pm_kernel::model::comp_node(root, Some(&p.snapshot.render_comp())).unwrap_or(root);
    let size = |k: &str| comp.get(k).and_then(crate::node_number).unwrap_or(0.);
    let mut out = corners_and_centre([0., 0., size("w"), size("h")]).to_vec();
    let mut mirror = QueryMirror::new(p);
    for layer in pm_kernel::model::list_layers(root, None)
        .into_iter()
        .filter(|l| {
            !moving.contains(&l.id) && !matches!(l.kind.as_str(), "audio" | "camera" | "group")
        })
        .take(MAX_LAYERS)
    {
        let q = mirror.queries();
        if !q.active(None, &layer.id, time).unwrap_or(false) {
            continue;
        }
        let (Ok(Some(b)), Ok(world)) = (
            q.bounds(None, &layer.id, time),
            mirror.queries().world_matrix(None, &layer.id, time),
        ) else {
            continue;
        };
        let corners = [[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]];
        if let Some(b) = box_of(corners.iter().map(|&c| transform(&world, c))) {
            out.extend(corners_and_centre(b));
        }
    }
    Some(out)
}
/// The selection's box after `delta`, as snap points.
pub fn box_points(poses: &[Pose], delta: [f64; 2]) -> Vec<[f64; 2]> {
    box_of(poses.iter().flat_map(|p| p.corners))
        .map(|b| {
            corners_and_centre([
                b[0] + delta[0],
                b[1] + delta[1],
                b[2] + delta[0],
                b[3] + delta[1],
            ])
            .to_vec()
        })
        .unwrap_or_default()
}
/// The correction on each unlocked axis and the guides it draws, each from
/// where the moved point lands to what it met.
pub fn snap(
    moving: &[[f64; 2]],
    targets: &[[f64; 2]],
    threshold: f64,
    axes: [bool; 2],
) -> ([f64; 2], Vec<Guide>) {
    let best = |axis: usize| {
        let mut best: Option<(f64, f64, [f64; 2], [f64; 2])> = None;
        for &m in moving {
            for &t in targets {
                let offset = t[axis] - m[axis];
                let distance = offset.abs();
                let cross = (t[1 - axis] - m[1 - axis]).abs();
                if distance <= threshold
                    && best.is_none_or(|(d, c, _, _)| distance < d || (distance == d && cross < c))
                {
                    best = Some((distance, cross, m, t));
                }
            }
        }
        best.map(|(_, _, m, t)| (t[axis] - m[axis], m, t))
    };
    let x = best(0).filter(|_| axes[0]);
    let y = best(1).filter(|_| axes[1]);
    let offset = [x.map_or(0., |s| s.0), y.map_or(0., |s| s.0)];
    let mut guides = vec![];
    for found in [x, y].into_iter().flatten() {
        let (_, m, t) = found;
        guides.push(([m[0] + offset[0], m[1] + offset[1]], t));
    }
    (offset, guides)
}
/// The system's alignment feedback when a move catches a guide.
pub fn haptic() {
    use objc2_app_kit::{
        NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime,
        NSHapticFeedbackPerformer,
    };
    let performer = NSHapticFeedbackManager::defaultPerformer();
    performer.performFeedbackPattern_performanceTime(
        NSHapticFeedbackPattern::Alignment,
        NSHapticFeedbackPerformanceTime::Now,
    );
}
impl GizmoSurface {
    /// viewer-chrome.md: 1 pt accent guides.
    pub(super) fn guides_paint(&self, frame: super::ViewFrame, window: &mut Window) {
        let ink = rgb(self.tokens.colors.accent);
        let at = |p: [f64; 2]| {
            frame.origin
                + point(
                    px((p[0] * frame.scale) as f32),
                    px((p[1] * frame.scale) as f32),
                )
        };
        for &(a, b) in &self.guides {
            let mut line = PathBuilder::stroke(px(1.));
            line.move_to(at(a));
            line.line_to(at(b));
            if let Ok(line) = line.build() {
                window.paint_path(line, ink);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn the_nearest_candidate_wins_on_each_unlocked_axis() {
        let moving = [[10., 10.], [20., 20.]];
        let targets = [[13., 50.], [0., 23.], [100., 100.]];
        let (offset, guides) = super::snap(&moving, &targets, 4., [true, true]);
        assert_eq!(offset, [3., 3.]);
        assert_eq!(guides.len(), 2);
        let (offset, _) = super::snap(&moving, &targets, 4., [true, false]);
        assert_eq!(offset, [3., 0.]);
        let (offset, _) = super::snap(&moving, &targets, 2., [true, true]);
        assert_eq!(offset, [0., 0.]);
    }
}
