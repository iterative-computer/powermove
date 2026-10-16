//! The scratch document a kernel edits, and the operations it returns.

use std::sync::Arc;

use pm_eval::{Comp, Evaluator, ExpressionEngine, JsResult, JsThrow, Scope};
use pm_model::{Document, JsString, Node, Op, Patch, PathPart, Value};

/// `PM.uid(prefix)` for ids a kernel creates (keys, layers, styles,
/// animators): `prefix + base + n`, with `n` counting up from `start`.
///
/// The renderer passes a fresh random `base` for every kernel call, so ids
/// are unique across calls as `PM.uid`'s are. The parity vectors pass an
/// empty base and the generator's own counter, which reproduces the
/// generator's `PM.uid` (`prefix + ++n`) wherever the call order agrees;
/// comparisons rename created ids by first appearance in any case.
#[derive(Clone, Debug)]
pub struct IdSource {
    base: String,
    next: u64,
}

impl IdSource {
    pub fn new(base: impl Into<String>, start: u64) -> IdSource {
        IdSource {
            base: base.into(),
            next: start,
        }
    }

    /// A fresh id with the legacy prefix (`k` keys, `L` layers, `ts` text
    /// styles, `ta` text animators).
    pub fn next(&mut self, prefix: &str) -> JsString {
        let id = format!("{prefix}{}{}", self.base, self.next);
        self.next += 1;
        JsString::from(id.as_str())
    }

    /// The counter's next value (for callers that continue it).
    pub fn peek(&self) -> u64 {
        self.next
    }
}

impl Default for IdSource {
    fn default() -> Self {
        IdSource::new("", 1)
    }
}

/// One kernel run: a scratch copy of the canonical document, an evaluator
/// over it, and the compositions written so far.
pub struct Edit<'o> {
    original: &'o Document,
    doc: Document,
    ev: Evaluator,
    comp: JsString,
    open: JsString,
    touched: Vec<JsString>,
    pub ids: IdSource,
}

fn internal(e: impl std::fmt::Display) -> JsThrow {
    JsThrow::type_error(format!("pm-edit: {e}"))
}

impl<'o> Edit<'o> {
    /// A run over `original` (canonical form: every composition in `comps`)
    /// that edits composition `comp` (`PM.curComp()`) with `open` as the
    /// open composition (`PM.proj`).
    pub fn new(
        original: &'o Document,
        comp: &str,
        open: &str,
        expressions: Option<Arc<dyn ExpressionEngine>>,
        ids: IdSource,
    ) -> JsResult<Edit<'o>> {
        for id in [comp, open] {
            let found = original
                .lookup(&[PathPart::from("comps"), PathPart::from(id)])
                .is_some_and(|n| matches!(n.kind(), pm_model::NodeKind::Object(_)));
            if !found {
                return Err(JsThrow::type_error(format!("No composition {id}")));
            }
        }
        let mut ev = Evaluator::new();
        if let Some(engine) = expressions {
            ev = ev.with_expressions(engine);
        }
        Ok(Edit {
            original,
            doc: original.clone(),
            ev,
            comp: JsString::from(comp),
            open: JsString::from(open),
            touched: Vec::new(),
            ids,
        })
    }

    /// The composition being edited.
    pub fn comp_id(&self) -> &JsString {
        &self.comp
    }

    /// The document as the kernel left it so far.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Runs `f` with a [`Scope`] over the scratch document as it is now.
    pub fn read<R>(&mut self, f: impl FnOnce(&mut Scope<'_, '_>) -> JsResult<R>) -> JsResult<R> {
        let root = self
            .doc
            .root()
            .ok_or_else(|| JsThrow::type_error("The mirror holds no document"))?;
        let revision = self.doc.revision();
        let comp_id = self.comp.to_string_lossy();
        let open_id = self.open.to_string_lossy();
        let comp = Comp::in_root(root, &comp_id, revision).map_err(internal)?;
        let open = Comp::in_root(root, &open_id, revision).map_err(internal)?;
        let mut scope = Scope::new(&mut self.ev, &comp, &open);
        f(&mut scope)
    }

    /// [`Edit::read`] with a node outside the document (a detached copy of a
    /// layer, as the legacy code measures `{ ...layer, d: { ... } }`) that
    /// lives as long as the scope's nodes.
    pub fn read_with<'e, R>(
        &'e mut self,
        extra: &'e Node,
        f: impl FnOnce(&mut Scope<'_, 'e>, &'e Node) -> JsResult<R>,
    ) -> JsResult<R> {
        let root = self
            .doc
            .root()
            .ok_or_else(|| JsThrow::type_error("The mirror holds no document"))?;
        let revision = self.doc.revision();
        let comp_id = self.comp.to_string_lossy();
        let open_id = self.open.to_string_lossy();
        let comp = Comp::in_root(root, &comp_id, revision).map_err(internal)?;
        let open = Comp::in_root(root, &open_id, revision).map_err(internal)?;
        let mut scope = Scope::new(&mut self.ev, &comp, &open);
        f(&mut scope, extra)
    }

    /// The node at an absolute (canonical) path.
    pub fn node(&self, path: &[PathPart]) -> Option<&Node> {
        self.doc.lookup(path)
    }

    /// `['comps', comp]`.
    pub fn comp_path(&self) -> Vec<PathPart> {
        vec![PathPart::from("comps"), PathPart::Key(self.comp.clone())]
    }

    /// `['comps', comp, 'layers', index]`.
    pub fn layer_path(&self, index: usize) -> Vec<PathPart> {
        let mut path = self.comp_path();
        path.push(PathPart::from("layers"));
        path.push(PathPart::from(index));
        path
    }

    /// The edited composition's layers as they are now.
    pub fn layers(&self) -> &[Node] {
        let mut path = self.comp_path();
        path.push(PathPart::from("layers"));
        match self.doc.lookup(&path).map(Node::kind) {
            Some(pm_model::NodeKind::Array(items)) => items,
            _ => &[],
        }
    }

    /// The layer at `index` now.
    pub fn layer(&self, index: usize) -> Option<&Node> {
        self.layers().get(index)
    }

    /// Applies leaf patches (absolute paths) to the scratch document as one
    /// revision.
    pub fn write(&mut self, patches: Vec<Patch>) -> JsResult<()> {
        if patches.is_empty() {
            return Ok(());
        }
        for patch in &patches {
            self.note(&patch.path);
        }
        let revision = self.doc.revision() + 1;
        self.doc.apply_at(&patches, revision).map_err(internal)?;
        Ok(())
    }

    /// Applies operations (leaf patches or id-keyed operations).
    pub fn write_ops(&mut self, ops: Vec<Op>) -> JsResult<()> {
        if ops.is_empty() {
            return Ok(());
        }
        for op in &ops {
            self.note(op.path());
        }
        let revision = self.doc.revision() + 1;
        self.doc.apply_ops_at(&ops, revision).map_err(internal)?;
        Ok(())
    }

    /// Sets one member (`None` is `undefined`).
    pub fn set(&mut self, path: Vec<PathPart>, value: Option<Value>) -> JsResult<()> {
        self.write(vec![Patch {
            path,
            exists: true,
            value,
        }])
    }

    /// Deletes one member.
    pub fn delete(&mut self, path: Vec<PathPart>) -> JsResult<()> {
        self.write(vec![Patch::delete(path)])
    }

    fn note(&mut self, path: &[PathPart]) {
        if let [PathPart::Key(comps), PathPart::Key(id), ..] = path
            && comps.as_bytes() == b"comps"
            && !self.touched.contains(id)
        {
            self.touched.push(id.clone());
        }
    }

    /// The operations that turn the original document into the edited one:
    /// `diff_ops` of every written composition, with canonical paths.
    pub fn finish(self) -> Vec<Op> {
        let mut out = Vec::new();
        for id in &self.touched {
            let path = [PathPart::from("comps"), PathPart::Key(id.clone())];
            let before = self.original.lookup(&path).map(Node::to_value);
            let after = self.doc.lookup(&path).map(Node::to_value);
            for op in pm_model::ops::diff_ops(before.as_ref(), after.as_ref()) {
                out.push(prefixed(op, &path));
            }
        }
        out
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
