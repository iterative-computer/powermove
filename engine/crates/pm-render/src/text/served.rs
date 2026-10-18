//! [`TextFonts`] over a font registry served from another thread
//! (`pm_text::served`): what the engines' render threads and `pm-cli` give
//! their renderers.
//!
//! Layout is level 1 (`pm_text::compat`) over a [`ServedResolver`]: each
//! registry query is answered by the registry itself before the layout
//! continues, with the full font bytes (outlines, color glyphs, variable
//! axes, hidden system fonts, project fonts). The renderer's font cache seeds
//! (amendment 19, the editor renderer's state; `docs/engine/text.md`) come
//! from the server.
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

/// Font cache seeds as a server knows them (the editor renderer's state).
pub trait SeedSource: Send + Sync {
    fn seeds(&self) -> Vec<FontCacheSeed>;
}

/// No seeds: a fresh renderer.
pub struct NoSeeds;

impl SeedSource for NoSeeds {
    fn seeds(&self) -> Vec<FontCacheSeed> {
        Vec::new()
    }
}

/// Fixed seeds (`pm-cli`, tests).
pub struct FixedSeeds(pub Vec<FontCacheSeed>);

impl SeedSource for FixedSeeds {
    fn seeds(&self) -> Vec<FontCacheSeed> {
        self.0.clone()
    }
}

/// Level 1 text over a served registry.
pub struct ServedFonts {
    server: Arc<dyn FontServer>,
    seeds: Arc<dyn SeedSource>,
    measure: CompatMeasure<ServedResolver>,
    /// The server generation `measure` was built for.
    generation: u64,
    rebuilds: u64,
}

impl ServedFonts {
    pub fn new(server: Arc<dyn FontServer>, seeds: Arc<dyn SeedSource>) -> Self {
        let generation = server.generation();
        let measure = Self::measure_for(&server, &*seeds);
        ServedFonts {
            server,
            seeds,
            measure,
            generation,
            rebuilds: 0,
        }
    }

    fn measure_for(
        server: &Arc<dyn FontServer>,
        seeds: &dyn SeedSource,
    ) -> CompatMeasure<ServedResolver> {
        let mut measure = CompatMeasure::new(ServedResolver::new(server.clone()));
        measure.compat().set_font_cache_seeds(seeds.seeds());
        measure
    }

    /// The seeds the current measurer uses.
    pub fn seeds(&mut self) -> Vec<FontCacheSeed> {
        self.measure.compat().font_cache_seeds().to_vec()
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
            self.measure = Self::measure_for(&self.server, &*self.seeds);
            self.generation = self.server.generation();
            self.rebuilds += 1;
        }
    }

    fn take_failures(&mut self) -> Vec<String> {
        self.measure.compat().resolver_mut().take_failures()
    }
}
