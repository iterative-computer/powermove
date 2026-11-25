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
    /// Lanes top to bottom, as v1 and Premiere stack them: captions, then
    /// video from the highest track down to V1, then audio from A1 down.
    pub fn display_order(&self) -> Vec<(Area, usize)> {
        let captions = (0..self.captions).rev().map(|l| (Area::Captions, l));
        let video = (0..self.video).rev().map(|l| (Area::Video, l));
        let audio = (0..self.audio).map(|l| (Area::Audio, l));
        captions.chain(video).chain(audio).collect()
    }
}
#[derive(Clone, Debug, Default)]
struct Unit {
    from: f64,
    end: f64,
    /// Seconds at its start and end that another clip on its track may
    /// overlap: its In and Out transitions (an audio clip's fades), where
    /// the clip before or after it runs on into it across a cut.
    lead: f64,
    tail: f64,
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
    layout_focused(index, reveal, None)
}
pub fn layout_focused(
    index: &TimelineIndex,
    reveal: &RevealState,
    focused_group: Option<&str>,
) -> Layout {
    let children = index.children();
    let roots = if let Some(gid) = focused_group {
        children.get(&Some(gid.to_owned())).cloned().unwrap_or_default()
    } else {
        index.roots(&children)
    };
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
            let [lead, tail] = l
                .transitions
                .each_ref()
                .map(|t| t.as_ref().map_or(0., |t| t.duration.max(0.)));
            units[i] = Unit {
                from: l.from,
                end: l.from + l.duration.max(0.),
                lead,
                tail,
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
            // Reserve the group strip and the members' lanes across both spans.
            let (from, end) = if l.duration > 0. {
                if kids.is_empty() {
                    (l.from, l.from + l.duration)
                } else {
                    (from.min(l.from), end.max(l.from + l.duration))
                }
            } else {
                (from, end)
            };
            let open = !reveal
                .layers
                .get(&l.id)
                .map(|d| d.group_collapsed)
                .unwrap_or(l.group_collapsed);
            let inner = if open {
                pack(&kids, &mut units, index, &reveal.held)
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
                lead: 0.,
                tail: 0.,
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
            pack(&roots, &mut units, index, &reveal.held)
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
impl Unit {
    /// The time it holds its lane alone: less what its transitions let a
    /// neighbour on its track overlap.
    fn held(&self) -> (f64, f64) {
        let (from, end) = (self.from + self.lead, self.end - self.tail);
        if from <= end {
            (from, end)
        } else {
            let mid = (self.from + self.end) / 2.;
            (mid, mid)
        }
    }
}
fn pack(
    ids: &[usize],
    units: &mut [Unit],
    index: &TimelineIndex,
    held: &HashMap<String, crate::reveal::Held>,
) -> usize {
    let holds: HashMap<usize, usize> = ids
        .iter()
        .filter_map(|&i| {
            let u = &units[i];
            held.get(&index.layers[i].id)
                .filter(|h| (h.from - u.from).abs() < 1e-6 && (h.end - u.end).abs() < 1e-6)
                .map(|h| (i, h.lane))
        })
        .collect();
    if holds.is_empty() {
        return pack_fresh(ids, units, index);
    }
    // Held clips first, each on its lane when free there (an edited clip
    // keeps its lane only if its time is unchanged); then every other one
    // above whatever it overlaps (a new clip, or one an edit moved).
    let overlap = |a: &Unit, b: (f64, f64)| a.held().0 < b.1 - 1e-5 && b.0 < a.held().1 - 1e-5;
    // Each placed unit: its held time, its lanes, and its place in the stack.
    let mut placed: Vec<(f64, f64, usize, usize, usize)> = Vec::new();
    let mut rest = Vec::new();
    for &i in ids.iter().rev() {
        let u = &units[i];
        match holds.get(&i).copied() {
            Some(lane)
                if !placed.iter().any(|&(f, e, lo, hi, _)| {
                    overlap(u, (f, e)) && lane < hi && lo < lane + u.height
                }) =>
            {
                placed.push((u.held().0, u.held().1, lane, lane + u.height, i));
                units[i].base = lane;
            }
            _ => rest.push(i),
        }
    }
    for i in rest {
        let u = &units[i];
        // A clip getting its first lane (a new layer) goes above every clip
        // it overlaps that is under it in the stack: the higher track is drawn
        // on top, so the timeline never contradicts the picture. One an edit
        // moved keeps its track where it can (nothing else re-packs).
        let fresh = !held.contains_key(&index.layers[i].id);
        let floor = placed
            .iter()
            .filter(|&&(f, e, _, _, at)| fresh && at > i && overlap(u, (f, e)))
            .map(|&(_, _, _, hi, _)| hi)
            .max()
            .unwrap_or(0);
        let mut base = floor.max(index.layers[i].preferred_track);
        // Then the lowest lane from there free over its time.
        loop {
            let top = base + u.height;
            match placed
                .iter()
                .filter(|&&(f, e, lo, hi, _)| overlap(u, (f, e)) && base < hi && lo < top)
                .map(|&(_, _, _, hi, _)| hi)
                .max()
            {
                Some(hi) => base = hi,
                None => break,
            }
        }
        placed.push((u.held().0, u.held().1, base, base + u.height, i));
        units[i].base = base;
    }
    placed.iter().map(|&(_, _, _, hi, _)| hi).max().unwrap_or(0)
}
fn pack_fresh(ids: &[usize], units: &mut [Unit], index: &TimelineIndex) -> usize {
    let mut starts: Vec<_> = ids.iter().map(|i| Total(units[*i].held().0)).collect();
    starts.sort();
    starts.dedup();
    let size = starts.len().max(1).next_power_of_two();
    let mut tree: Vec<BTreeMap<Total, usize>> = vec![BTreeMap::new(); 2 * size];
    let mut height = 0;
    for i in ids.iter().rev() {
        let (from, end) = units[*i].held();
        let u = &units[*i];
        let mut end_at = starts.partition_point(|v| v.0 < end - 1e-5) + size;
        let mut first = size;
        let mut base = index.layers[*i].preferred_track;
        while first < end_at {
            if first % 2 == 1 {
                if let Some((_, h)) = tree[first]
                    .range((Excluded(Total(from + 1e-5)), Unbounded))
                    .next()
                {
                    base = base.max(*h);
                }
                first += 1;
            }
            if end_at % 2 == 1 {
                end_at -= 1;
                if let Some((_, h)) = tree[end_at]
                    .range((Excluded(Total(from + 1e-5)), Unbounded))
                    .next()
                {
                    base = base.max(*h);
                }
            }
            first /= 2;
            end_at /= 2;
        }
        let top = base.saturating_add(u.height);
        let stop = Total(end);
        let mut at = starts.binary_search(&Total(from)).unwrap() + size;
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
/// The stack after a Tracks move: the moved layers (with their groups'
/// members) taken out as one block and put back where `planned` (the lanes'
/// order) puts them, every other layer keeping its order. `None` keeps the
/// stack as it is: when the block would end where it was, or would cross a
/// locked layer, a layer of another area (a caption composites in the same
/// stack), a layer of another group, a 3D layer or any layer when the block is
/// 3D (a 3D run's separators stay), or a track matte pair. Then only the
/// lane changes.
pub fn move_order(
    index: &TimelineIndex,
    ids: &[String],
    planned: &[String],
) -> Option<Vec<String>> {
    let moved: HashSet<String> = index.expanded_ids(ids).into_iter().collect();
    let original: Vec<&str> = index.layers.iter().map(|l| l.id.as_str()).collect();
    let block: Vec<&str> = original
        .iter()
        .copied()
        .filter(|id| moved.contains(*id))
        .collect();
    let first = *block.first()?;
    let rest: Vec<&str> = original
        .iter()
        .copied()
        .filter(|id| !moved.contains(*id))
        .collect();
    // Where the lanes put the block: before the first untouched layer that
    // follows it in `planned`, else at the end.
    let at = planned.iter().position(|id| id == first)?;
    let anchor = planned[at..].iter().find(|id| !moved.contains(*id));
    let insert = anchor
        .and_then(|a| rest.iter().position(|id| id == a))
        .unwrap_or(rest.len());
    let mut candidate = rest.clone();
    for (i, id) in block.iter().enumerate() {
        candidate.insert(insert + i, id);
    }
    if candidate == original {
        return None;
    }
    // The untouched layers the block passes.
    let before = |order: &[&str], id: &str| {
        order.iter().position(|x| *x == id) < order.iter().position(|x| *x == first)
    };
    let root = index.layer(first)?;
    let matte = |id: &str| {
        index
            .source
            .as_ref()
            .and_then(|s| {
                let root = s.root()?;
                let loc = pm_kernel::model::find_layer(root, id)?;
                let node = pm_kernel::model::layer_at(root, &loc)?;
                Some(
                    node.get("matteMode")
                        .is_some_and(|m| !matches!(m.kind(), pm_model::NodeKind::Null)),
                )
            })
            .unwrap_or(false)
    };
    let three_d = block
        .iter()
        .any(|id| index.layer(id).is_some_and(|l| l.three_d));
    let block_matte = block.iter().any(|id| matte(id));
    for id in &rest {
        if before(&original, id) == before(&candidate, id) {
            continue;
        }
        let Some(x) = index.layer(id) else {
            return None;
        };
        if x.effectively_locked
            || area(&x.kind) != area(&root.kind)
            || x.group != root.group
            || x.three_d
            || three_d
            || block_matte
            || matte(id)
        {
            return None;
        }
    }
    // A track matte reads the layer directly above it: the layer that now
    // sits below the block, or below where it was, must not use one.
    let last = *block.last()?;
    for order in [&original, &candidate] {
        let next = order
            .iter()
            .position(|x| *x == last)
            .and_then(|i| order.get(i + 1))
            .copied();
        if let Some(next) = next
            && !moved.contains(next)
            && matte(next)
        {
            return None;
        }
    }
    Some(candidate.into_iter().map(str::to_owned).collect())
}

/// The lanes the Tracks timeline shows now, with each clip's time, to hold
/// through the next change (`RevealState::held`).
pub fn shown(index: &TimelineIndex, reveal: &RevealState) -> HashMap<String, crate::reveal::Held> {
    let current = layout(index, reveal);
    relative_lanes(index, reveal)
        .into_iter()
        .filter_map(|(id, lane)| {
            let item = current.item(&id)?;
            Some((
                id,
                crate::reveal::Held {
                    lane: lane as usize,
                    from: item.from,
                    end: item.from + item.duration,
                },
            ))
        })
        .collect()
}

/// Every layer's lane as the Tracks timeline shows it, relative to its group
/// (a layer's stored `track`), for an edit that must keep clips on their
/// tracks (`TimingCommand::InTracks`).
pub fn relative_lanes(index: &TimelineIndex, reveal: &RevealState) -> Vec<(String, u32)> {
    plan_move(index, reveal, &[], 0., LaneDelta::default())
        .unwrap_or_default()
        .into_iter()
        .map(|l| (l.id, u32::try_from(l.track).unwrap_or(u32::MAX)))
        .collect()
}
/// Where a new layer made over `from..end` goes in Tracks (PM-512): the
/// lowest track above every clip of its area it overlaps (a new track when
/// that is past the last), and the stack place directly above the topmost of
/// those clips (with its members, for a group), so the higher track is
/// drawn on top. With nothing to overlap: V1 (or A1) at the top of the stack.
/// Returns (track, stack index).
pub fn top_place(
    index: &TimelineIndex,
    reveal: &RevealState,
    kind: &str,
    from: f64,
    end: f64,
) -> (usize, usize) {
    let area = area(kind);
    let layout = layout(index, reveal);
    let under: Vec<&Item> = layout
        .items
        .iter()
        .filter(|i| {
            i.area == area
                && i.parent.is_none()
                && i.from < end - 1e-5
                && from < i.from + i.duration - 1e-5
        })
        .collect();
    let track = under.iter().map(|i| i.lane + 1).max().unwrap_or(0);
    let ids: Vec<String> = under.iter().map(|i| i.id.clone()).collect();
    let place = index
        .expanded_ids(&ids)
        .iter()
        .filter_map(|id| index.layers.iter().position(|l| &l.id == id))
        .min()
        .unwrap_or(0);
    (track, place)
}

/// `command` as the Tracks timeline makes it: wrapped with the lanes it
/// shows, so the kernel keeps every clip on its track.
pub fn in_tracks(
    index: &TimelineIndex,
    reveal: &RevealState,
    command: pm_kernel::commands::timing::TimingCommand,
) -> pm_kernel::commands::timing::TimingCommand {
    pm_kernel::commands::timing::TimingCommand::InTracks {
        lanes: relative_lanes(index, reveal),
        command: Box::new(command),
    }
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
