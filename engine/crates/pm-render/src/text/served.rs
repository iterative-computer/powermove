//! [`TextFonts`] over a font registry served from another thread
//! (`pm_text::served`): what the engines' render threads and `pm-cli` give
//! their renderers.
//!
//! Layout is level 1 (`pm_text::compat`) over a [`ServedResolver`]: each
//! registry query is answered by the registry itself before the layout
//! continues, with the full font bytes (outlines, color glyphs, variable
//! axes, hidden system fonts, project fonts). The renderer's font cache seeds
//! (amendment 19, the editor renderer's state; `docs/engine/text.md`) come
//! from the server, per window: each layout measures with the seeds of the
//! renderer that shows it ([`SeedScope`]).
//!
//! When the server's generation moves (project fonts or seeds changed), the
//! next text layer starts from a fresh resolver and measurer, and the
//! generation enters every raster's content key, so no cached layout,
//! glyph box, or raster of the old fonts is reused.

use std::sync::Arc;

use pm_eval::text_geom::TextMeasure;
use pm_eval::text_measure::CompatMeasure;
use pm_text::compat::{FontCacheSeed, GlyphRun, LineRuns, TextContent};
use pm_text::font::{FaceId, FaceInfo, FontResolver};
use pm_text::served::{FontServer, ServedResolver};

use super::TextFonts;

/// What a layout is for, so it measures with the font cache seeds of the
/// renderer that shows it: each editor window is its own renderer with its
/// own font cache (`docs/engine/text.md`, "Seeds per window").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SeedScope {
    /// The engine port the layout's document came through (a window's
    /// port names that window).
    pub port: Option<u32>,
    /// The engine session (sessions main opened for a window's export).
    pub session: Option<u64>,
}

impl SeedScope {
    /// A document a client port opened.
    pub fn port(port: u32) -> Self {
        SeedScope {
            port: Some(port),
            session: None,
        }
    }
}

/// Font cache seeds as a server knows them (the editor renderers' state).
pub trait SeedSource: Send + Sync {
    /// The seeds of the renderer a layout in `scope` is for.
    fn seeds(&self, scope: SeedScope) -> Vec<FontCacheSeed>;
}

/// No seeds: a fresh renderer.
pub struct NoSeeds;

impl SeedSource for NoSeeds {
    fn seeds(&self, _: SeedScope) -> Vec<FontCacheSeed> {
        Vec::new()
    }
}

/// Fixed seeds for every scope (`pm-cli`, tests).
pub struct FixedSeeds(pub Vec<FontCacheSeed>);

impl SeedSource for FixedSeeds {
    fn seeds(&self, _: SeedScope) -> Vec<FontCacheSeed> {
        self.0.clone()
    }
}

/// Measurers kept for other seed sets (windows on other kinds of display)
/// besides the current one.
const PARKED_MEASURERS: usize = 3;

/// Level 1 text over a served registry.
pub struct ServedFonts {
    server: Arc<dyn FontServer>,
    seeds: Arc<dyn SeedSource>,
    scope: SeedScope,
    measure: CompatMeasure<ServedResolver>,
    /// Measurers of other seed sets, most recent first, all of the current
    /// server generation (a layout cache belongs to one font cache).
    parked: Vec<CompatMeasure<ServedResolver>>,
    /// The server generation `measure` was built for.
    generation: u64,
    rebuilds: u64,
}

impl ServedFonts {
    pub fn new(server: Arc<dyn FontServer>, seeds: Arc<dyn SeedSource>) -> Self {
        let generation = server.generation();
        let scope = SeedScope::default();
        let measure = Self::measure_for(&server, seeds.seeds(scope));
        ServedFonts {
            server,
            seeds,
            scope,
            measure,
            parked: Vec::new(),
            generation,
            rebuilds: 0,
        }
    }

    fn measure_for(
        server: &Arc<dyn FontServer>,
        seeds: Vec<FontCacheSeed>,
    ) -> CompatMeasure<ServedResolver> {
        let mut measure = CompatMeasure::new(ServedResolver::new(server.clone()));
        measure.compat().set_font_cache_seeds(seeds);
        measure
    }

    /// The seeds the current measurer uses.
    pub fn seeds(&mut self) -> Vec<FontCacheSeed> {
        self.measure.compat().font_cache_seeds().to_vec()
    }

    /// The scope layouts are measured for now.
    pub fn scope(&self) -> SeedScope {
        self.scope
    }

    /// Times the measurer was replaced after a generation change.
    pub fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    /// Round trips to the registry made by the current measurer.
    pub fn round_trips(&mut self) -> u64 {
        self.measure.compat().resolver().round_trips()
    }

    /// Asks the registry for these queries in one round trip (the families
    /// a document names, before its first frame).
    pub fn prefetch(&mut self, queries: &[pm_text::answers::Query]) {
        self.sync();
        self.measure.compat().resolver_mut().prefetch(queries);
    }

    /// Makes the measurer the one of `seeds`: the current one when its seeds
    /// are these, else a parked one, else a new one (the current one is
    /// parked).
    fn select(&mut self, seeds: Vec<FontCacheSeed>) {
        if self.measure.compat().font_cache_seeds() == seeds.as_slice() {
            return;
        }
        let parked = self
            .parked
            .iter_mut()
            .position(|m| m.compat().font_cache_seeds() == seeds.as_slice());
        let next = match parked {
            Some(i) => self.parked.remove(i),
            None => Self::measure_for(&self.server, seeds),
        };
        let previous = std::mem::replace(&mut self.measure, next);
        self.parked.insert(0, previous);
        self.parked.truncate(PARKED_MEASURERS);
    }
}

impl TextFonts for ServedFonts {
    fn measure(&mut self) -> &mut dyn TextMeasure {
        &mut self.measure
    }

    fn line_runs(&mut self, d: &TextContent) -> Vec<LineRuns> {
        self.measure.compat().line_runs(d)
    }

    fn text_runs(&mut self, d: &TextContent, text: &str, tracking: f64) -> Vec<GlyphRun> {
        self.measure.compat().text_runs(d, text, tracking).0
    }

    fn face(&mut self, id: FaceId) -> &FaceInfo {
        self.measure.compat().resolver().face(id)
    }

    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        self.measure.compat().resolver_mut().em_advance(face, size)
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn sync(&mut self) {
        let now = self.server.generation();
        if now != self.generation || !self.measure.compat().resolver().is_current() {
            // New fonts or seeds: every measurer starts over.
            self.parked.clear();
            self.measure = Self::measure_for(&self.server, self.seeds.seeds(self.scope));
            self.generation = self.server.generation();
            self.rebuilds += 1;
        } else {
            self.select(self.seeds.seeds(self.scope));
        }
    }

    fn set_seed_scope(&mut self, scope: SeedScope) {
        self.scope = scope;
    }

    fn take_failures(&mut self) -> Vec<String> {
        self.measure.compat().resolver_mut().take_failures()
    }
}
