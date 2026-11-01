//! Powermove P-A11Y: pure reading-order planner (also runnable with rustc --test).
#[derive(Clone)]
pub(crate) enum Entry {
    Gpui(String),
    Native,
    RemainingGpui,
}
// Plan the merge separately from Objective-C messaging. Explicit GPUI entries
// are reserved before RemainingGpui is expanded; duplicates are emitted once.
pub(crate) fn merge_order(ids: &[Option<String>], order: &[Entry]) -> Vec<(bool, usize)> {
    let mut used = vec![false; ids.len()];
    let mut result = Vec::new();
    for (index, entry) in order.iter().enumerate() {
        match entry {
            Entry::Gpui(id) => {
                if let Some(i) = ids
                    .iter()
                    .enumerate()
                    .position(|(i, v)| !used[i] && v.as_ref() == Some(id))
                {
                    used[i] = true;
                    result.push((false, i));
                }
            }
            Entry::Native => result.push((true, index)),
            Entry::RemainingGpui => {
                for (i, id) in ids.iter().enumerate() {
                    if !used[i]
                        && !order
                            .iter()
                            .any(|v| matches!(v, Entry::Gpui(named) if Some(named) == id.as_ref()))
                    {
                        used[i] = true;
                        result.push((false, i));
                    }
                }
            }
        }
    }
    // A stale/missing anchor must never make real GPUI nodes disappear.
    for (i, used) in used.iter().enumerate() {
        if !used {
            result.push((false, i));
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_slots_interleave_and_gpui_tail_is_preserved() {
        let ids = vec![Some("navigation".into()), Some("timeline".into())];
        let order = vec![
            Entry::Gpui("navigation".into()),
            Entry::Native,
            Entry::Native,
            Entry::RemainingGpui,
        ];
        assert_eq!(
            merge_order(&ids, &order),
            vec![(false, 0), (true, 1), (true, 2), (false, 1)]
        );
    }
    #[test]
    fn merge_reserves_later_anchors_and_keeps_unnamed_nodes() {
        let ids = vec![Some("end".into()), None, Some("start".into())];
        let order = vec![
            Entry::Gpui("start".into()),
            Entry::RemainingGpui,
            Entry::Gpui("end".into()),
            Entry::Gpui("start".into()),
        ];
        assert_eq!(
            merge_order(&ids, &order),
            vec![(false, 2), (false, 1), (false, 0)]
        );
        assert_eq!(
            merge_order(&ids, &[Entry::Gpui("missing".into())]),
            vec![(false, 0), (false, 1), (false, 2)]
        );
    }
}
