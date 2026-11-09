//! The renderer's view of the font host (plan section 12; Phase 3c "full
//! render fonts on the render and job threads"; RFC 16).
//!
//! A rendering thread lays text out with [`EngineFonts`] over a
//! [`BrokerResolver`]: the registry's answers replayed over the full font
//! files, asking the main thread's [`pm_text::host::FontService`] through the
//! broker for what it lacks. [`RenderFonts`] keeps a renderer's fonts in step
//! with the registry's generation and carries the font-health boundary.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pm_text::font::{FaceId, FaceInfo};
use pm_text::host::{BrokerResolver, FontBroker, FontHealth};

/// The fonts a renderer lays text out with, over a broker (the interactive
/// engine's render thread and the job engine's lanes).
#[cfg(target_os = "macos")]
pub struct EngineFonts {
    measure: pm_eval::text_measure::CompatMeasure<BrokerResolver>,
    generation: u64,
}

#[cfg(target_os = "macos")]
impl EngineFonts {
    /// Fonts at the broker's current generation.
    pub fn new(broker: &FontBroker, health: &FontHealth) -> Self {
        let generation = broker.generation();
        EngineFonts {
            measure: pm_eval::text_measure::CompatMeasure::new(BrokerResolver::new(
                broker.clone(),
                health.clone(),
            )),
            generation,
        }
    }
}

#[cfg(target_os = "macos")]
impl crate::text::TextFonts for EngineFonts {
    fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
        &mut self.measure
    }
    fn line_runs(&mut self, d: &pm_text::compat::TextContent) -> Vec<pm_text::compat::LineRuns> {
        self.measure.compat().line_runs(d)
    }
    fn text_runs(
        &mut self,
        d: &pm_text::compat::TextContent,
        text: &str,
        tracking: f64,
    ) -> Vec<pm_text::compat::GlyphRun> {
        self.measure.compat().text_runs(d, text, tracking).0
    }
    fn face(&mut self, id: FaceId) -> &FaceInfo {
        use pm_text::font::FontResolver;
        self.measure.compat().resolver().face(id)
    }
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        use pm_text::font::FontResolver;
        self.measure.compat().resolver_mut().em_advance(face, size)
    }
    fn generation(&self) -> u64 {
        self.generation
    }
}

/// A renderer's fonts and their state, as a rendering thread keeps them.
#[cfg(target_os = "macos")]
#[derive(Clone)]
pub struct RenderFonts {
    pub broker: FontBroker,
    pub health: FontHealth,
    /// Set after a render met a failure: the next use installs fonts afresh.
    reinstall: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for RenderFonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderFonts")
            .field("broker", &self.broker)
            .field("health", &self.health)
            .finish()
    }
}

#[cfg(target_os = "macos")]
impl RenderFonts {
    pub fn new(broker: FontBroker) -> Self {
        RenderFonts {
            broker,
            health: FontHealth::default(),
            reinstall: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The same fonts for a renderer on another thread: the same broker,
    /// with its own health, so a failure one thread met is reported by that
    /// thread's render only.
    pub fn sibling(&self) -> Self {
        Self {
            broker: self.broker.clone(),
            health: FontHealth::default(),
            reinstall: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The registry generation (includes project font changes).
    pub fn generation(&self) -> u64 {
        self.broker.generation()
    }

    /// Installs fonts at the broker's generation into `renderer` when it has
    /// none or older ones; true when it installed.
    pub fn sync(&self, renderer: &mut crate::Renderer, installed: &mut Option<u64>) -> bool {
        let generation = self.generation();
        let reinstall = self.reinstall.swap(false, Ordering::AcqRel);
        if *installed == Some(generation) && !reinstall {
            return false;
        }
        let fonts: Box<dyn crate::text::TextFonts> =
            Box::new(EngineFonts::new(&self.broker, &self.health));
        renderer.set_text_fonts(Some(fonts));
        *installed = Some(generation);
        true
    }

    /// After a render: the font failure it met, if any. The renderer's fonts
    /// are installed afresh before the next render (their caches may hold
    /// answers the failure left empty).
    pub fn after_render(&self) -> Option<String> {
        let failure = self.health.take_failure();
        if failure.is_some() {
            self.reinstall.store(true, Ordering::Release);
        }
        failure
    }
}
