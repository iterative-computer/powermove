//! The planning context every command runs in.
//!
//! A [`Cx`] holds the transaction's scratch tree. Writes apply to it at
//! once, so a command that edits in several steps (insert a group, then move
//! layers into it) reads its own earlier writes, exactly as the old code
//! read what it wrote a line earlier. [`Cx::finish`] returns the operations
//! in order; the store applies them to its own scratch tree again and
//! captures their inverses.

use std::sync::atomic::{AtomicU64, Ordering};

use pm_model::ops::diff_node_ops;
use pm_model::patch::{Patch, PathPart};
use pm_model::{Document, JsString, Node, NodeKind, Op, Value};

use super::Origin;
use crate::error::CommandError;
use crate::model::{self, LayerLoc, str_of};

/// A fresh id with the old app's prefix and shape (`PM.uid`: the prefix and
/// seven base-36 characters): `L` layers, `k` keyframes, `K` masks, `C`
/// compositions, `marker` markers, `fx` effects.
pub fn new_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static SEED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let seed = *SEED.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        h.finish()
    });
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // splitmix64 of seed + n: distinct per call, unpredictable across runs.
    let mut z = seed.wrapping_add(n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = String::with_capacity(prefix.len() + 7);
    out.push_str(prefix);
    for _ in 0..7 {
        out.push(DIGITS[(z % 36) as usize] as char);
        z /= 36;
    }
    out
}

/// The scratch state of one command.
pub struct Cx<'a> {
    base: &'a Node,
    origin: Origin,
    doc: Document,
    ops: Vec<Op>,
    revision: u64,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(root: &'a Node, origin: Origin) -> Cx<'a> {
        Cx {
            base: root,
            origin,
            doc: Document::from_root(Some(root.clone()), 0),
            ops: Vec::new(),
            revision: 0,
        }
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// The tree the command started from.
    pub fn base(&self) -> &'a Node {
        self.base
    }

    /// The scratch tree as written so far.
    pub fn root(&self) -> &Node {
        self.doc.root().unwrap_or(self.base)
    }

    pub fn lookup(&self, path: &[PathPart]) -> Option<&Node> {
        self.root().lookup(path)
    }

    /// The value at `path` (cloned), `None` when absent.
    pub fn value_at(&self, path: &[PathPart]) -> Option<Value> {
        self.lookup(path).map(Node::to_value)
    }

    /// The operations written, in order.
    pub(crate) fn finish(self) -> Vec<Op> {
        self.ops
    }

    // ── writes ─────────────────────────────────────────────────────────────

    /// Applies one operation to the scratch tree and records it.
    pub fn write(&mut self, op: Op) -> Result<(), CommandError> {
        self.revision += 1;
        self.doc
            .apply_ops_at(std::slice::from_ref(&op), self.revision)
            .map_err(|e| CommandError::Malformed(e.to_string()))?;
        self.ops.push(op);
        Ok(())
    }

    /// Sets the member at `path` (written only when it differs).
    pub fn set(&mut self, path: Vec<PathPart>, value: Value) -> Result<(), CommandError> {
        if self
            .lookup(&path)
            .is_some_and(|n| super::values::same(n, &value))
        {
            return Ok(());
        }
        self.write(Op::Leaf(Patch::set(path, value)))
    }

    /// Removes the member at `path` (when present).
    pub fn remove(&mut self, path: Vec<PathPart>) -> Result<(), CommandError> {
        if self.lookup(&path).is_none() {
            return Ok(());
        }
        self.write(Op::Leaf(Patch::delete(path)))
    }

    /// Replaces the subtree at `path` with `after`, writing only the
    /// difference (leaf sets, and id-keyed inserts/deletes/orders inside
    /// arrays of records).
    pub fn replace(&mut self, path: Vec<PathPart>, after: Value) -> Result<(), CommandError> {
        let next = Node::from_value(after, self.revision + 1);
        let ops = diff_node_ops(self.lookup(&path), Some(&next));
        for op in ops {
            self.write(prefixed(op, &path))?;
        }
        Ok(())
    }

    // ── layers ─────────────────────────────────────────────────────────────

    /// Finds a layer (open composition first, then the others).
    pub fn find(&self, layer_id: &str) -> Result<LayerLoc, CommandError> {
        model::find_layer(self.root(), layer_id)
            .ok_or_else(|| CommandError::LayerNotFound(layer_id.to_owned()))
    }

    pub fn layer_node(&self, loc: &LayerLoc) -> Result<&Node, CommandError> {
        model::layer_at(self.root(), loc)
            .ok_or_else(|| CommandError::Malformed("a layer moved during the edit".into()))
    }

    /// The layer as a value, with its location.
    pub fn layer(&self, layer_id: &str) -> Result<(LayerLoc, Value), CommandError> {
        let loc = self.find(layer_id)?;
        let value = self.layer_node(&loc)?.to_value();
        Ok((loc, value))
    }

    /// [`Cx::layer`] refusing a locked layer (or one inside a locked group).
    pub fn layer_mut_checked(&self, layer_id: &str) -> Result<(LayerLoc, Value), CommandError> {
        let (loc, value) = self.layer(layer_id)?;
        self.check_lock(&loc, layer_id)?;
        Ok((loc, value))
    }

    /// The old lock rule (`lockedMessage`).
    pub fn check_lock(&self, loc: &LayerLoc, layer_id: &str) -> Result<(), CommandError> {
        let layer = self.layer_node(loc)?;
        super::spine::check_lock(self.root(), loc, layer, layer_id)
    }

    /// Every layer of every composition: the open one first.
    pub fn all_layers(&self) -> Vec<LayerLoc> {
        let root = self.root();
        let open = model::open_comp_id(root);
        let mut comps = vec![open.clone()];
        if open.is_some()
            && let Some(NodeKind::Object(map)) = root.get("comps").map(Node::kind)
        {
            for (k, _) in map.iter() {
                if let Some(k) = k.as_str()
                    && Some(k) != open.as_deref()
                {
                    comps.push(Some(k.to_owned()));
                }
            }
        }
        let mut out = Vec::new();
        for comp_id in comps {
            if let Some(comp) = model::comp_node(root, comp_id.as_deref()) {
                for index in 0..model::layers_of(comp).count() {
                    out.push(LayerLoc {
                        comp_id: comp_id.clone(),
                        index,
                    });
                }
            }
        }
        out
    }

    /// Runs an engine edit kernel (`pm_edit`) over the scratch tree for
    /// composition `comp` and writes the operations it returns. A kernel's
    /// legacy throw becomes [`CommandError::Invalid`] with its message.
    pub fn kernel<R>(
        &mut self,
        comp: Option<&str>,
        f: impl FnOnce(&mut pm_edit::Edit<'_>) -> pm_eval::JsResult<R>,
    ) -> Result<R, CommandError> {
        let comp_id = self
            .comp_id(comp)
            .ok_or_else(|| CommandError::Malformed("the project has no compositions".into()))?;
        let open = model::open_comp_id(self.root()).unwrap_or_else(|| comp_id.clone());
        let doc = Document::from_root(Some(self.root().clone()), self.revision);
        let ids = pm_edit::IdSource::new(new_id(""), 1);
        let engine: std::sync::Arc<dyn pm_eval::ExpressionEngine> =
            std::sync::Arc::new(pm_wasm::expr::PmExpressions);
        let (result, ops) = {
            let mut edit = pm_edit::Edit::new(&doc, &comp_id, &open, Some(engine), ids)
                .map_err(|e| CommandError::Invalid(e.message))?;
            let result = f(&mut edit).map_err(|e| CommandError::Invalid(e.message))?;
            (result, edit.finish())
        };
        for op in ops {
            self.write(op)?;
        }
        Ok(result)
    }

    /// Writes an edited copy of a layer back (only what changed).
    pub fn put_layer(&mut self, loc: &LayerLoc, layer: Value) -> Result<(), CommandError> {
        self.replace(loc.path(), layer)
    }

    // ── compositions ───────────────────────────────────────────────────────

    /// `None` resolves to the open composition.
    pub fn comp_id(&self, comp: Option<&str>) -> Option<String> {
        comp.map(str::to_owned)
            .or_else(|| model::open_comp_id(self.root()))
    }

    pub fn comp_node(&self, comp: Option<&str>) -> Result<&Node, CommandError> {
        let id = self.comp_id(comp);
        model::comp_node(self.root(), id.as_deref())
            .filter(|c| matches!(c.kind(), NodeKind::Object(_)))
            .ok_or_else(|| CommandError::CompNotFound(id.unwrap_or_default()))
    }

    /// `['comps', id]` (or `[]` for a project without compositions).
    pub fn comp_path(&self, comp: Option<&str>) -> Vec<PathPart> {
        model::comp_path(self.comp_id(comp).as_deref())
    }

    /// The composition's frame rate (`PM.proj.fps`, 30 when unusable).
    pub fn fps(&self, comp: Option<&str>) -> f64 {
        self.comp_node(comp)
            .ok()
            .and_then(|c| c.get("fps"))
            .and_then(model::number_of)
            .filter(|f| f.is_finite() && *f > 0.0)
            .unwrap_or(30.0)
    }

    /// The composition's duration (`PM.proj.dur`).
    pub fn duration(&self, comp: Option<&str>) -> f64 {
        self.comp_node(comp)
            .ok()
            .and_then(|c| c.get("dur"))
            .and_then(model::number_of)
            .unwrap_or(0.0)
    }

    /// The layer ids of a composition, in stack order.
    pub fn layer_ids(&self, comp: Option<&str>) -> Vec<String> {
        self.comp_node(comp)
            .map(|c| {
                model::layers_of(c)
                    .filter_map(|l| l.get("id").and_then(str_of).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The layers of a composition as values.
    pub fn layers(&self, comp: Option<&str>) -> Vec<Value> {
        self.comp_node(comp)
            .map(|c| model::layers_of(c).map(Node::to_value).collect())
            .unwrap_or_default()
    }

    /// `['comps', id, 'layers']`.
    pub fn layers_path(&self, comp: Option<&str>) -> Vec<PathPart> {
        let mut p = self.comp_path(comp);
        p.push("layers".into());
        p
    }

    /// Inserts a layer record at `index` of a composition.
    pub fn insert_layer(
        &mut self,
        comp: Option<&str>,
        index: usize,
        layer: Value,
    ) -> Result<(), CommandError> {
        let path = self.layers_path(comp);
        self.write(Op::Insert {
            path,
            index: index as u32,
            value: layer,
        })
    }

    /// Removes a layer record by id.
    pub fn delete_layer(&mut self, comp: Option<&str>, id: &str) -> Result<(), CommandError> {
        let path = self.layers_path(comp);
        self.write(Op::Delete {
            path,
            id: JsString::from(id),
        })
    }

    /// Reorders a composition's layers into `ids` (a permutation).
    pub fn order_layers(&mut self, comp: Option<&str>, ids: &[String]) -> Result<(), CommandError> {
        if self.layer_ids(comp) == ids {
            return Ok(());
        }
        let path = self.layers_path(comp);
        self.write(Op::Order {
            path,
            ids: ids.iter().map(|s| JsString::from(s.as_str())).collect(),
        })
    }
}

fn prefixed(op: Op, prefix: &[PathPart]) -> Op {
    let join = |path: Vec<PathPart>| {
        let mut full = prefix.to_vec();
        full.extend(path);
        full
    };
    match op {
        Op::Leaf(mut patch) => {
            patch.path = join(std::mem::take(&mut patch.path));
            Op::Leaf(patch)
        }
        Op::Order { path, ids } => Op::Order {
            path: join(path),
            ids,
        },
        Op::Insert { path, index, value } => Op::Insert {
            path: join(path),
            index,
            value,
        },
        Op::Delete { path, id } => Op::Delete {
            path: join(path),
            id,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_the_old_shape_and_do_not_repeat() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let id = new_id("k");
            assert_eq!(id.len(), 8);
            assert!(id.starts_with('k'));
            assert!(seen.insert(id));
        }
    }
}
