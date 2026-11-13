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
//!
//! The run is the composition's, not the export's: it is followed on the
//! export's frame grid to both sides of the exported range, as far as the
//! composition goes, so a work area that starts after a fade or ends before
//! a title delivers the frames the whole export delivers, and ranges that
//! share a grid join without a seam. A run longer than [`MAX_RUN`] frames,
//! or one whose frames cannot all be planned, is filtered.
//!
//! The inverse returns a source's own chroma only while the delivery leaves
//! its colour as it is ([`Delivery`]): an HDR output, a tone-mapped one, or
//! a frame something drew on after rendering is filtered like any other.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use pm_render::{FrameFootage, Renderer};

use crate::convert::Chroma;
use crate::settings::{ColorTags, InputFormat};

/// What the delivery does to a frame after it is rendered.
#[derive(Clone, Copy, Debug)]
pub struct Delivery {
    pub format: InputFormat,
    pub tags: ColorTags,
    /// The frame is tone-mapped for the output (`RenderRequest::output_tone`).
    pub tone_mapped: bool,
    /// Something draws on the frame after it is rendered
    /// (`ExportPlan::frame_post`).
    pub postprocessed: bool,
}

impl Delivery {
    /// Whether untouched Rec.709 footage reaches the encoder input with its
    /// own colour, so the inverse of its upsampling returns its own chroma:
    /// a subsampled Rec.709 output (primaries, matrix and display encoding
    /// as the source's; the transfer tag is a label), no tone transform, no
    /// postprocessor. `Some(rows)` then says whether rows are subsampled too.
    fn inverse(self) -> Option<bool> {
        use InputFormat as F;
        let rows = match self.format {
            F::V216 => false,
            F::V420 | F::X420 | F::V0a8 | F::I420 | F::I420a => true,
            F::Y416 | F::Rgba8 | F::Rgba16 => return None,
        };
        let same_colour = matches!(
            self.tags,
            ColorTags::Rec709Broadcast | ColorTags::Rec709QuickTime
        );
        (same_colour && !self.tone_mapped && !self.postprocessed).then_some(rows)
    }
}

/// The longest run of untouched footage that is looked along: the plans of
/// its frames are read once, at the first of its frames an export delivers
/// (about 15 µs each for a single clip; `tests/chroma_delivery.rs` measures
/// it), so this bounds the wait before that frame. A longer run is filtered,
/// whatever part of it is exported.
pub const MAX_RUN: u64 = 1 << 18;

/// What a frame's plan shows of one source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    /// The source alone, untouched.
    Untouched,
    /// The source with something done to it, or with other picture.
    Touched,
    /// Nothing of it.
    Absent,
}

/// The run of frames around frame `n` (which shows a source untouched) that
/// show the source, within `grid`, when every one of them shows it
/// untouched and there are at most `limit`. `shown` is `None` for a frame
/// whose plan is not known.
fn clean_run(
    n: i64,
    grid: &Range<i64>,
    limit: u64,
    shown: &mut dyn FnMut(i64) -> Option<Shown>,
) -> Option<Range<i64>> {
    let (mut first, mut end) = (n, n + 1);
    let fits = |first: i64, end: i64| (end - first) as u64 <= limit;
    while first > grid.start {
        match shown(first - 1)? {
            Shown::Absent => break,
            Shown::Touched => return None,
            Shown::Untouched => first -= 1,
        }
        if !fits(first, end) {
            return None;
        }
    }
    while end < grid.end {
        match shown(end)? {
            Shown::Absent => break,
            Shown::Touched => return None,
            Shown::Untouched => end += 1,
        }
        if !fits(first, end) {
            return None;
        }
    }
    fits(first, end).then_some(first..end)
}

/// The chroma decimation of an export's frames, asked in frame order.
/// Frames are numbered from the export's first; the composition's frames
/// before it have negative numbers.
pub struct ChromaPolicy {
    /// `Delivery::inverse`; `None`: every frame is filtered.
    inverse: Option<bool>,
    /// The composition's frames on the export's grid.
    grid: Range<i64>,
    /// A source and the run of frames it is untouched in.
    clean: Option<(PathBuf, Range<i64>)>,
    /// A source whose run through the frame before is filtered.
    filtered: Option<PathBuf>,
    /// Whether a source's chroma is what the inverse returns for this output.
    sources: HashMap<PathBuf, bool>,
}

impl ChromaPolicy {
    /// `grid`: the composition's frames on the export's frame grid (it
    /// holds the export's own `0..frames`); `None` when they are not known,
    /// and every frame is filtered.
    pub fn new(delivery: Delivery, grid: Option<Range<i64>>) -> Self {
        Self {
            inverse: delivery.inverse().filter(|_| grid.is_some()),
            grid: grid.unwrap_or(0..0),
            clean: None,
            filtered: None,
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

    /// The decimation of frame `n`, whose plan reported `footage`. `ahead`
    /// gives the footage of another frame of the grid without rendering it
    /// (`None`: unknown); it is asked once for each frame of a run of
    /// untouched footage, at the first of its frames delivered.
    pub fn frame(
        &mut self,
        n: u64,
        footage: FrameFootage,
        renderer: &mut Renderer,
        ahead: &mut dyn FnMut(&mut Renderer, i64) -> Option<FrameFootage>,
    ) -> Chroma {
        let Some(rows) = self.inverse else {
            return Chroma::Filtered;
        };
        // A run known to be filtered ends where its source leaves.
        if self
            .filtered
            .as_deref()
            .is_some_and(|path| !footage.may_show(path))
        {
            self.filtered = None;
        }
        let Some(path) = self.untouched(renderer, &footage, rows) else {
            return Chroma::Filtered;
        };
        let n = n as i64;
        if matches!(&self.clean, Some((p, run)) if p == path && run.contains(&n)) {
            return Chroma::Footage;
        }
        if self.filtered.as_deref() == Some(path) {
            return Chroma::Filtered;
        }
        // The first delivered frame of a run of this source: the run goes
        // both ways, past the exported range, to where a frame no longer
        // shows the source or the composition ends.
        let grid = self.grid.clone();
        let run = clean_run(n, &grid, MAX_RUN, &mut |m| {
            let other = ahead(renderer, m)?;
            Some(if !other.may_show(path) {
                Shown::Absent
            } else if self.untouched(renderer, &other, rows) == Some(path) {
                Shown::Untouched
            } else {
                Shown::Touched
            })
        });
        match run {
            Some(run) => {
                self.clean = Some((path.to_path_buf(), run));
                Chroma::Footage
            }
            None => {
                self.filtered = Some(path.to_path_buf());
                Chroma::Filtered
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clean run around `n` of frames shown as `frames` says (index 0
    /// is grid frame `first`), and the frames asked about.
    fn run(
        frames: &[Option<Shown>],
        first: i64,
        n: i64,
        limit: u64,
    ) -> (Option<Range<i64>>, usize) {
        let grid = first..first + frames.len() as i64;
        let mut asked = 0;
        let found = clean_run(n, &grid, limit, &mut |m| {
            asked += 1;
            frames[(m - first) as usize]
        });
        (found, asked)
    }

    const U: Option<Shown> = Some(Shown::Untouched);
    const T: Option<Shown> = Some(Shown::Touched);
    const A: Option<Shown> = Some(Shown::Absent);

    #[test]
    fn a_run_is_the_same_from_any_of_its_frames() {
        // Frames -3..5: absent, five untouched, absent, touched.
        let frames = [A, U, U, U, U, U, A, T];
        for n in -2..3 {
            assert_eq!(run(&frames, -3, n, 100).0, Some(-2..3), "from frame {n}");
        }
    }

    #[test]
    fn a_touched_frame_on_either_side_filters_the_whole_run() {
        // A fade before the exported frames, and a title after them.
        for frames in [[T, U, U, U, U, A], [A, U, U, U, U, T], [A, U, T, U, U, A]] {
            for n in 0..frames.len() as i64 {
                if frames[n as usize] == U {
                    assert_eq!(run(&frames, 0, n, 100).0, None, "{frames:?} from {n}");
                }
            }
        }
    }

    #[test]
    fn a_run_reaches_the_ends_of_the_composition() {
        assert_eq!(run(&[U, U, U, U], -2, 0, 100).0, Some(-2..2));
    }

    #[test]
    fn a_frame_that_cannot_be_planned_filters_the_run() {
        assert_eq!(run(&[A, U, U, None, U], 0, 1, 100).0, None);
        assert_eq!(run(&[None, U, U, A], 0, 2, 100).0, None);
    }

    #[test]
    fn a_run_longer_than_the_limit_is_filtered_from_any_frame_at_bounded_cost() {
        let mut frames = vec![U; 40];
        frames[0] = A;
        for n in 1..40 {
            let (found, asked) = run(&frames, 0, n, 8);
            assert_eq!(found, None, "from frame {n}");
            assert!(asked <= 9, "{asked} frames planned from frame {n}");
        }
        // A run of exactly the limit is kept.
        let mut frames = vec![U; 10];
        (frames[0], frames[9]) = (A, A);
        assert_eq!(run(&frames, 0, 4, 8).0, Some(1..9));
    }
}
