//! Which chroma decimation each frame of an export takes (section 9,
//! "Chroma"; section 2 invariant (1)).
//!
//! Rendered frames are decimated band-limited ([`Chroma::Filtered`]).
//! Untouched footage is the exception: it takes the exact inverse of the
//! renderer's upsampling ([`Chroma::Footage`]) and so keeps its own chroma.
//! The two respond very differently to fine coloured detail, so the choice
//! never changes inside a continuous piece of picture: it is made once for
//! the whole run of frames a source stays in the picture, and is the inverse
//! only when every frame of that run is the source alone, untouched (no
//! opacity or transform, no effect or transition, nothing over it). A clip
//! that fades, or that a title crosses, is filtered from its first frame to
//! its last.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pm_render::{FrameFootage, Renderer};

use crate::convert::Chroma;
use crate::settings::InputFormat;

/// What an export delivers.
#[derive(Clone, Copy, Debug)]
pub struct Delivery {
    pub format: InputFormat,
}

impl Delivery {
    /// Whether the output is subsampled, so the inverse of a source's
    /// upsampling returns its own chroma. `Some(rows)` then says whether
    /// rows are subsampled too.
    fn inverse(self) -> Option<bool> {
        use InputFormat as F;
        match self.format {
            F::V216 => Some(false),
            F::V420 | F::X420 | F::V0a8 | F::I420 | F::I420a => Some(true),
            F::Y416 | F::Rgba8 | F::Rgba16 => None,
        }
    }
}

/// The chroma decimation of an export's frames, asked in frame order.
pub struct ChromaPolicy {
    /// `Delivery::inverse`; `None`: every frame is filtered.
    inverse: Option<bool>,
    /// What the frame before showed.
    before: FrameFootage,
    /// A source proven untouched in every frame up to (not including) this
    /// frame index.
    clean: Option<(PathBuf, u64)>,
    /// Whether a source's chroma is what the inverse returns for this output.
    sources: HashMap<PathBuf, bool>,
}

impl ChromaPolicy {
    pub fn new(delivery: Delivery) -> Self {
        Self {
            inverse: delivery.inverse(),
            before: FrameFootage::default(),
            clean: None,
            sources: HashMap::new(),
        }
    }

    /// The source `footage` is, when the frame is that source alone and
    /// untouched, and its chroma was upsampled at the output's factors and
    /// siting from the output's primaries, matrix and encoding.
    fn untouched<'a>(
        &mut self,
        renderer: &mut Renderer,
        footage: &'a FrameFootage,
        rows: bool,
    ) -> Option<&'a Path> {
        use pm_protocol::color::{
            AlphaMode, ChromaSiting, MatrixCoefficients, Primaries, Subsampling, Transfer,
        };
        let candidate = footage.candidate.as_ref()?;
        let facts = candidate.facts;
        if !(facts.single_source && facts.untransformed && facts.effect_free) {
            return None;
        }
        let path = candidate.path.as_path();
        let invertible = *self.sources.entry(path.to_path_buf()).or_insert_with(|| {
            let Ok(source) = renderer.video_color(path) else {
                return false;
            };
            // Rows of a 4:2:0 output are the inverse of the renderer's
            // linear upsampling only for a 4:2:0 source centred between
            // rows (type 0); a 4:2:2 output keeps the rendered rows and
            // needs only the columns.
            let sited = match source.subsampling {
                Subsampling::S420 => source.siting == ChromaSiting::Left,
                Subsampling::S422 => !rows && source.siting.horizontally_cosited(),
                Subsampling::S444 => false,
            };
            sited
                && matches!(source.alpha, AlphaMode::Opaque)
                && source.primaries == Primaries::Bt709
                && source.matrix == MatrixCoefficients::Bt709
                && source.transfer == Transfer::DisplayE
        });
        invertible.then_some(path)
    }

    /// The decimation of frame `n` of `frames`, whose plan reported
    /// `footage`. `ahead` gives the footage of a later frame without
    /// rendering it (`None`: unknown), asked only at the first frame of a
    /// piece of untouched footage, for the frames that piece runs through.
    pub fn frame(
        &mut self,
        n: u64,
        frames: u64,
        footage: FrameFootage,
        renderer: &mut Renderer,
        ahead: &mut dyn FnMut(&mut Renderer, u64) -> Option<FrameFootage>,
    ) -> Chroma {
        let Some(rows) = self.inverse else {
            return Chroma::Filtered;
        };
        let chroma = match self.untouched(renderer, &footage, rows) {
            None => Chroma::Filtered,
            Some(path) if matches!(&self.clean, Some((p, end)) if p == path && n < *end) => {
                Chroma::Footage
            }
            // The piece began in a frame that was filtered.
            Some(path) if self.before.may_show(path) => Chroma::Filtered,
            Some(path) => {
                // A piece of picture begins here: it runs until a frame no
                // longer shows the source, and is clean when every frame of
                // it is the source alone, untouched.
                let mut end = n + 1;
                let clean = loop {
                    if end == frames {
                        break true;
                    }
                    let Some(next) = ahead(renderer, end) else {
                        break false;
                    };
                    if !next.may_show(path) {
                        break true;
                    }
                    if self.untouched(renderer, &next, rows) != Some(path) {
                        break false;
                    }
                    end += 1;
                };
                if clean {
                    self.clean = Some((path.to_path_buf(), end));
                    Chroma::Footage
                } else {
                    Chroma::Filtered
                }
            }
        };
        self.before = footage;
        chroma
    }
}
