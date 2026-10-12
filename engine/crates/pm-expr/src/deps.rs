//! The inputs an evaluation observed, for incremental invalidation.

use crate::string::JsStr;

/// Context inputs an evaluation read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ContextReads(u8);

impl ContextReads {
    /// `t` (layer-local time), also read by `wiggle`.
    pub const LOCAL_TIME: ContextReads = ContextReads(1);
    /// `time` or `T` (composition time).
    pub const TIME: ContextReads = ContextReads(1 << 1);
    pub const FPS: ContextReads = ContextReads(1 << 2);
    /// `frame` (native dialect).
    pub const FRAME: ContextReads = ContextReads(1 << 3);
    /// `value`, the keyframed value of the channel.
    pub const VALUE: ContextReads = ContextReads(1 << 4);
    /// `ch`, the channel key.
    pub const CHANNEL: ContextReads = ContextReads(1 << 5);
    /// `idx`, the layer's index in its composition.
    pub const INDEX: ContextReads = ContextReads(1 << 6);

    pub const fn empty() -> ContextReads {
        ContextReads(0)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: ContextReads) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: ContextReads) -> bool {
        self.0 & other.0 != 0
    }

    pub fn insert(&mut self, other: ContextReads) {
        self.0 |= other.0;
    }
}

impl std::ops::BitOr for ContextReads {
    type Output = ContextReads;
    fn bitor(self, rhs: ContextReads) -> ContextReads {
        ContextReads(self.0 | rhs.0)
    }
}

/// Where a field path starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Root {
    Layer,
    Comp,
    /// The keyframed value, when it is an object or array.
    Value,
}

/// One step of a field path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    /// An own property of an object (`p`, `"toString"` when a conversion
    /// probed for it, `"0"` for `obj[0]` on an object).
    Key(JsStr),
    /// An array element (present or not).
    Index(u64),
    /// An array's length.
    Length,
}

/// A field the evaluation read: the value at `root.path`.
///
/// A read of a primitive depends on its value; a read of an object or array
/// depends only on which object is there (its identity and kind), because
/// every read inside it is recorded as its own longer path. A field that did
/// not exist is recorded too, so creating it invalidates.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldRead {
    pub root: Root,
    pub path: Vec<Step>,
}

/// Everything one evaluation observed. If none of it changes, evaluating the
/// same expression again gives the same result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dependencies {
    pub context: ContextReads,
    /// Sorted and without duplicates.
    pub fields: Vec<FieldRead>,
}

impl Dependencies {
    /// Whether the result can change with time alone (`t`, `time`, `T`,
    /// `frame`, or `wiggle`). `fps` and the keyframed value are separate.
    pub fn depends_on_time(&self) -> bool {
        self.context.intersects(ContextReads::LOCAL_TIME | ContextReads::TIME | ContextReads::FRAME)
    }

    /// Whether a change at `root.path` (the node there was replaced or
    /// modified, so anything below it may differ) can change what this
    /// evaluation saw: true when a recorded field lies at or below that path,
    /// or, for the keyframed value, when the whole value changed and the
    /// expression read `value`. A change to an array's length is a change at
    /// the array's own path.
    pub fn affected_by(&self, root: Root, path: &[Step]) -> bool {
        if root == Root::Value && self.context.contains(ContextReads::VALUE) && path.is_empty() {
            return true;
        }
        self.fields.iter().any(|f| f.root == root && f.path.starts_with(path))
    }

    pub(crate) fn finish(&mut self) {
        self.fields.sort();
        self.fields.dedup();
    }
}
