//! Crossing latch ported from v1.2.0 directional-snap.ts:36–133.
//! Tolerances are in time units supplied by the view; no visual constants here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    In,
    Out,
}
#[derive(Clone, Copy, Debug)]
pub struct Anchor {
    pub time: f64,
    pub edge: Option<Edge>,
}
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub time: f64,
    pub edge: Option<Edge>,
    pub weight: f64,
}
impl Target {
    pub fn at(time: f64) -> Self {
        Self {
            time,
            edge: None,
            weight: 1.,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lock {
    pub anchor: usize,
    pub target: f64,
    pub direction: i8,
}
#[derive(Clone, Debug, Default)]
pub struct DirectionalSnap {
    previous: f64,
    pub lock: Option<Lock>,
}
impl DirectionalSnap {
    pub fn new(initial: f64) -> Self {
        Self {
            previous: if initial.is_finite() { initial } else { 0. },
            lock: None,
        }
    }
    pub fn reset(&mut self, delta: f64) {
        if delta.is_finite() {
            self.previous = delta;
        }
        self.lock = None;
    }
    pub fn resolve(
        &mut self,
        anchors: &[Anchor],
        raw: f64,
        targets: &[Target],
        tolerance: f64,
        enabled: bool,
    ) -> f64 {
        let raw = if raw.is_finite() { raw } else { self.previous };
        let before = self.previous;
        self.previous = raw;
        let travel = raw - before;
        let direction = if travel.abs() <= 1e-9 {
            0
        } else if travel > 0. {
            1
        } else {
            -1
        };
        let radius = if tolerance.is_finite() {
            tolerance.max(0.)
        } else {
            0.
        };
        if !enabled {
            self.lock = None;
            return raw;
        }
        let released = self.lock;
        if let Some(lock) = self.lock {
            if let Some(anchor) = anchors.get(lock.anchor) {
                let exists = targets
                    .iter()
                    .any(|t| t.time == lock.target && (t.edge.is_none() || t.edge == anchor.edge));
                if exists
                    && (direction == 0 || direction == lock.direction)
                    && (anchor.time + raw - lock.target).abs() <= radius + 1e-9
                {
                    return lock.target - anchor.time;
                }
            }
            self.lock = None;
        }
        if direction == 0 {
            return raw;
        }
        // Sorted spatial index preserves source order for ties. Limits comparisons
        // to the release neighbourhood for 60k-edge projects.
        let mut indexed: Vec<_> = targets
            .iter()
            .enumerate()
            .filter(|(_, t)| t.time.is_finite())
            .collect();
        indexed.sort_by(|a, b| a.1.time.total_cmp(&b.1.time));
        let mut best_distance = f64::INFINITY;
        let mut best_weight = f64::NEG_INFINITY;
        for (index, anchor) in anchors.iter().enumerate() {
            if !anchor.time.is_finite() {
                continue;
            }
            let margin = 8. * f64::EPSILON * (anchor.time.abs() + raw.abs() + radius) + 1e-9;
            let first =
                indexed.partition_point(|(_, t)| t.time < anchor.time + raw - radius - margin);
            let end =
                indexed.partition_point(|(_, t)| t.time <= anchor.time + raw + radius + margin);
            let mut nearby = indexed[first..end].to_vec();
            nearby.sort_by_key(|(i, _)| *i);
            for (_, target) in nearby {
                if target.edge.is_some() && target.edge != anchor.edge {
                    continue;
                }
                if released.is_some_and(|l| l.anchor == index && l.target == target.time) {
                    continue;
                }
                let at = target.time - anchor.time;
                let crossed = if direction > 0 {
                    at > before + 1e-9 && at <= raw + 1e-9
                } else {
                    at < before - 1e-9 && at >= raw - 1e-9
                };
                let distance = (raw - at).abs();
                if !crossed || distance > radius + 1e-9 {
                    continue;
                }
                let weight = if target.weight.is_finite() {
                    target.weight
                } else {
                    1.
                };
                if distance < best_distance - 1e-9
                    || ((distance - best_distance).abs() <= 1e-9 && weight > best_weight)
                {
                    best_distance = distance;
                    best_weight = weight;
                    self.lock = Some(Lock {
                        anchor: index,
                        target: target.time,
                        direction,
                    });
                }
            }
        }
        self.lock.map_or(raw, |l| l.target - anchors[l.anchor].time)
    }
}

/// Keyframes use a proximity latch rather than strip crossing semantics.
/// timeline.ts:298–324. Acquire/release distances are supplied by the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupLock {
    pub anchor: f64,
    pub target: f64,
}
pub fn keyframe_group(
    anchors: &[f64],
    raw: f64,
    targets: &[f64],
    acquire: f64,
    locked: Option<GroupLock>,
    release: f64,
) -> (f64, Option<GroupLock>) {
    let raw = if raw.is_finite() { raw } else { 0. };
    let acquire = if acquire.is_finite() {
        acquire.max(0.)
    } else {
        0.
    };
    let release = if release.is_finite() {
        release.max(acquire)
    } else {
        acquire
    };
    if let Some(lock) = locked
        && anchors.contains(&lock.anchor)
        && (lock.anchor + raw - lock.target).abs() <= release
    {
        return (lock.target - lock.anchor, Some(lock));
    }
    let mut lock = None;
    let mut distance = acquire + f64::EPSILON;
    for anchor in anchors.iter().filter(|t| t.is_finite()) {
        for target in targets.iter().filter(|t| t.is_finite()) {
            let next = (anchor + raw - target).abs();
            if next < distance {
                distance = next;
                lock = Some(GroupLock {
                    anchor: *anchor,
                    target: *target,
                });
            }
        }
    }
    (lock.map_or(raw, |l| l.target - l.anchor), lock)
}
