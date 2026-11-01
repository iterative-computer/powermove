//! Track projection, v1.2.0 track-layout.ts:1–209. Lanes are semantic ordinals.
//! Overlap packing uses a range index instead of the tag's quadratic scan (N2).
use crate::{RevealState, TimelineIndex};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Bound::{Excluded, Unbounded};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    Video,
    Audio,
    Captions,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub area: Area,
    pub lane: usize,
    pub from: f64,
    pub duration: f64,
    pub depth: usize,
    pub parent: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub id: String,
    pub area: Area,
    pub lane_start: usize,
    pub lane_end: usize,
}
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub video: usize,
    pub audio: usize,
    pub captions: usize,
    pub items: Vec<Item>,
    pub bands: Vec<Band>,
    by_id: HashMap<String, usize>,
}
impl Layout {
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.by_id.get(id).map(|i| &self.items[*i])
    }
}
#[derive(Clone, Debug, Default)]
struct Unit {
    from: f64,
    end: f64,
    height: usize,
    base: usize,
    inner: usize,
    children: Vec<usize>,
}
pub fn area(kind: &str) -> Area {
    match kind {
        "audio" => Area::Audio,
        "captions" => Area::Captions,
        _ => Area::Video,
    }
}
pub fn layout(index: &TimelineIndex, reveal: &RevealState) -> Layout {
    let children = index.children();
    let roots = index.roots(&children);
    let mut units = vec![Unit::default(); index.layers.len()];
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    let mut todo = roots.clone();
    while let Some(i) = todo.pop() {
        if visited.insert(i) {
            order.push(i);
            if let Some(kids) = children.get(&Some(index.layers[i].id.clone())) {
                todo.extend(kids);
            }
        }
    }
    let mut built = HashSet::new();
    for i in order.into_iter().rev() {
        let l = &index.layers[i];
        if l.kind != "group" {
            units[i] = Unit {
                from: l.from,
                end: l.from + l.duration.max(0.),
                height: 1,
                ..Default::default()
            };
        } else {
            let kids: Vec<_> = children
                .get(&Some(l.id.clone()))
                .into_iter()
                .flatten()
                .copied()
                .filter(|k| built.contains(k))
                .collect();
            let from = kids
                .iter()
                .map(|k| units[*k].from)
                .reduce(f64::min)
                .unwrap_or(0.);
            let end = kids
                .iter()
                .map(|k| units[*k].end)
                .reduce(f64::max)
                .unwrap_or(index.duration);
            let open = !reveal
                .layers
                .get(&l.id)
                .map(|d| d.group_collapsed)
                .unwrap_or(l.group_collapsed);
            let inner = if open {
                pack(&kids, &mut units, index)
            } else {
                0
            };
            units[i] = Unit {
                from,
                end,
                height: inner.saturating_add(1),
                inner,
                children: if open { kids } else { Vec::new() },
                base: 0,
            };
        }
        built.insert(i);
    }
    let mut out = Layout::default();
    for current in [Area::Video, Area::Audio, Area::Captions] {
        let roots: Vec<_> = roots
            .iter()
            .copied()
            .filter(|i| area(&index.layers[*i].kind) == current)
            .collect();
        let height = if current == Area::Captions {
            for (rank, i) in roots.iter().enumerate() {
                units[*i].base = roots.len() - 1 - rank;
            }
            roots.len()
        } else {
            pack(&roots, &mut units, index)
        };
        match current {
            Area::Video => out.video = height,
            Area::Audio => out.audio = height,
            Area::Captions => out.captions = height,
        }
        let mut todo: Vec<_> = roots
            .into_iter()
            .rev()
            .map(|i| (i, 0usize, 0usize, None))
            .collect();
        let mut seen = HashSet::new();
        while let Some((i, offset, depth, parent)) = todo.pop() {
            if !seen.insert(i) {
                continue;
            }
            let u = &units[i];
            let l = &index.layers[i];
            let base = offset.saturating_add(u.base);
            out.by_id.insert(l.id.clone(), out.items.len());
            out.items.push(Item {
                id: l.id.clone(),
                area: current,
                lane: base.saturating_add(u.height).saturating_sub(1),
                from: u.from,
                duration: u.end - u.from,
                depth,
                parent,
            });
            if u.inner > 0 {
                out.bands.push(Band {
                    id: l.id.clone(),
                    area: current,
                    lane_start: base,
                    lane_end: base.saturating_add(u.inner),
                });
            }
            todo.extend(
                u.children
                    .iter()
                    .rev()
                    .map(|k| (*k, base, depth + 1, Some(l.id.clone()))),
            );
        }
    }
    out
}
fn pack(ids: &[usize], units: &mut [Unit], index: &TimelineIndex) -> usize {
    let mut starts: Vec<_> = ids.iter().map(|i| Total(units[*i].from)).collect();
    starts.sort();
    starts.dedup();
    let size = starts.len().max(1).next_power_of_two();
    let mut tree: Vec<BTreeMap<Total, usize>> = vec![BTreeMap::new(); 2 * size];
    let mut height = 0;
    for i in ids.iter().rev() {
        let u = &units[*i];
        let mut end = starts.partition_point(|v| v.0 < u.end - 1e-5) + size;
        let mut first = size;
        let mut base = index.layers[*i].preferred_track;
        while first < end {
            if first % 2 == 1 {
                if let Some((_, h)) = tree[first]
                    .range((Excluded(Total(u.from + 1e-5)), Unbounded))
                    .next()
                {
                    base = base.max(*h);
                }
                first += 1;
            }
            if end % 2 == 1 {
                end -= 1;
                if let Some((_, h)) = tree[end]
                    .range((Excluded(Total(u.from + 1e-5)), Unbounded))
                    .next()
                {
                    base = base.max(*h);
                }
            }
            first /= 2;
            end /= 2;
        }
        let top = base.saturating_add(u.height);
        let stop = Total(u.end);
        let mut at = starts.binary_search(&Total(u.from)).unwrap() + size;
        while at > 0 {
            let map = &mut tree[at];
            if map.range(stop..).next().is_none_or(|(_, h)| *h < top) {
                while let Some((&key, &h)) = map.range(..=stop).next_back() {
                    if h > top {
                        break;
                    }
                    map.remove(&key);
                }
                map.insert(stop, top);
            }
            at /= 2;
        }
        units[*i].base = base;
        height = height.max(top);
    }
    height
}
#[derive(Clone, Copy, Debug)]
struct Total(f64);
impl PartialEq for Total {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0).is_eq()
    }
}
impl Eq for Total {}
impl PartialOrd for Total {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Total {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LaneDelta {
    pub video: isize,
    pub audio: isize,
    pub captions: isize,
}
impl LaneDelta {
    fn get(self, area: Area) -> isize {
        match area {
            Area::Video => self.video,
            Area::Audio => self.audio,
            Area::Captions => self.captions,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct MovedLayer {
    pub id: String,
    pub from: f64,
    pub track: usize,
}
/// Pure track drag preview, track-layout.ts:230–304. The kernel owner must add
/// a validated atomic track/stack command before this can be committed.
pub fn plan_move(
    index: &TimelineIndex,
    reveal: &RevealState,
    ids: &[String],
    delta: f64,
    lanes: LaneDelta,
) -> Result<Vec<MovedLayer>, crate::ModelError> {
    crate::interaction::finite(delta)?;
    let selected: HashSet<_> = ids.iter().cloned().collect();
    let mut roots = HashSet::new();
    for id in ids {
        let l = index
            .layer(id)
            .ok_or_else(|| crate::ModelError::UnknownId(id.clone()))?;
        let mut parent = l.group.as_ref();
        let mut seen = HashSet::new();
        let mut implied = false;
        while let Some(id) = parent {
            if !seen.insert(id) {
                break;
            }
            if selected.contains(id) {
                implied = true;
                break;
            }
            parent = index.layer(id).and_then(|l| l.group.as_ref());
        }
        if !implied {
            roots.insert(id.clone());
        }
    }
    let shifted: HashSet<_> = index
        .expanded_ids(&roots.iter().cloned().collect::<Vec<_>>())
        .into_iter()
        .collect();
    for id in &shifted {
        if index.layer(id).unwrap().effectively_locked {
            return Err(crate::ModelError::Locked(id.clone()));
        }
    }
    let current = layout(index, reveal);
    let bands: HashMap<_, _> = current.bands.iter().map(|b| (b.id.as_str(), b)).collect();
    let children = index.children();
    let mut out = Vec::new();
    let mut todo = vec![(None, None)];
    let mut seen = HashSet::new();
    while let Some((parent, root_area)) = todo.pop() {
        let siblings = children.get(&parent).cloned().unwrap_or_default();
        let mut desired = HashMap::new();
        for i in &siblings {
            let l = &index.layers[*i];
            let inner = bands
                .get(l.id.as_str())
                .map_or(0, |b| b.lane_end - b.lane_start);
            let relative = current
                .item(&l.id)
                .and_then(|item| {
                    let lane = item.lane.saturating_sub(inner);
                    match parent.as_deref() {
                        None => Some(lane),
                        Some(p) => bands.get(p).map(|b| lane.saturating_sub(b.lane_start)),
                    }
                })
                .unwrap_or(l.preferred_track);
            let area = root_area.unwrap_or_else(|| area(&l.kind));
            let limit = parent.as_deref().and_then(|p| bands.get(p)).map_or_else(
                || match area {
                    Area::Video => current.video,
                    Area::Audio => current.audio,
                    Area::Captions => current.captions.saturating_sub(1),
                },
                |b| b.lane_end - b.lane_start,
            );
            let lane = if roots.contains(&l.id) {
                relative.saturating_add_signed(lanes.get(area)).min(limit)
            } else {
                relative
            };
            desired.insert(*i, lane);
        }
        let mut ordered = siblings.clone();
        for current_area in [Area::Video, Area::Audio, Area::Captions] {
            let slots: Vec<_> = siblings
                .iter()
                .enumerate()
                .filter(|(_, i)| area(&index.layers[**i].kind) == current_area)
                .map(|(position, i)| (position, *i))
                .collect();
            let mut sorted = slots.clone();
            sorted.sort_by(|(a, ia), (b, ib)| {
                desired[ib]
                    .cmp(&desired[ia])
                    .then_with(|| {
                        roots
                            .contains(&index.layers[*ib].id)
                            .cmp(&roots.contains(&index.layers[*ia].id))
                    })
                    .then(a.cmp(b))
            });
            for ((position, _), (_, i)) in slots.into_iter().zip(sorted) {
                ordered[position] = i;
            }
        }
        // A flat task stack emits parents immediately before their members.
        let mut emit = Vec::new();
        for i in ordered {
            if !seen.insert(i) {
                continue;
            }
            let l = &index.layers[i];
            let area = root_area.unwrap_or_else(|| area(&l.kind));
            emit.push((
                MovedLayer {
                    id: l.id.clone(),
                    from: l.from + if shifted.contains(&l.id) { delta } else { 0. },
                    track: desired[&i],
                },
                if l.kind == "group" {
                    Some((Some(l.id.clone()), Some(area)))
                } else {
                    None
                },
            ));
        }
        // Queue a separate traversal below; use output insertion frames to retain
        // source order without recursion for deeply nested groups.
        for (item, child) in emit {
            out.push(item);
            if let Some(child) = child {
                todo.push(child);
            }
        }
    }
    // Reorder into hierarchy order; sibling lane sorting is already represented
    // by the order in each parent's output block.
    let by_id: HashMap<_, _> = out
        .iter()
        .enumerate()
        .map(|(i, l)| (l.id.clone(), i))
        .collect();
    let mut ordered = Vec::new();
    let mut tasks: Vec<_> = out
        .iter()
        .filter(|l| {
            index
                .layer(&l.id)
                .is_some_and(|l| l.group.as_ref().is_none_or(|p| index.layer(p).is_none()))
        })
        .map(|l| l.id.clone())
        .rev()
        .collect();
    let mut visited = HashSet::new();
    let mut new_children: HashMap<String, Vec<String>> = HashMap::new();
    for l in &out {
        if let Some(p) = index.layer(&l.id).and_then(|l| l.group.as_ref()) {
            new_children
                .entry(p.clone())
                .or_default()
                .push(l.id.clone());
        }
    }
    while let Some(id) = tasks.pop() {
        if visited.insert(id.clone()) {
            ordered.push(out[by_id[&id]].clone());
            if let Some(kids) = new_children.get(&id) {
                tasks.extend(kids.iter().rev().cloned());
            }
        }
    }
    for l in &index.layers {
        if !visited.contains(&l.id) {
            ordered.push(MovedLayer {
                id: l.id.clone(),
                from: l.from + if shifted.contains(&l.id) { delta } else { 0. },
                track: l.preferred_track,
            });
        }
    }
    Ok(ordered)
}
