//! The kernel's fonts: the same registry rendering asks, measured on the
//! kernel's own thread, so a command never waits on the main thread's turn.
//! Viewer tools ask their text questions through [`query`], which keeps one
//! mirror per project and feeds its engine the fonts a question lacks.
use pm_kernel::text_host::{SharedTextHost, TextHost};
use pm_kernel::{Published, QueryContext, QueryError, QueryMirror};
use pm_render::text::host::RenderFonts;
use pm_text::host::BrokerResolver;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

struct RegistryText(RenderFonts);
impl TextHost for RegistryText {
    fn measure(&self) -> Box<dyn pm_eval::text_geom::TextMeasure> {
        Box::new(pm_eval::text_measure::CompatMeasure::new(
            BrokerResolver::new(self.0.broker.clone(), self.0.health.clone()),
        ))
    }
    fn take_failure(&self) -> Option<String> {
        self.0.health.take_failure()
    }
}
/// The registry viewer queries ask; set once with the kernel's host.
static FONTS: OnceLock<RenderFonts> = OnceLock::new();
/// One query mirror per project: it follows publications incrementally and
/// keeps the fonts it was given.
static MIRRORS: Mutex<BTreeMap<String, QueryMirror>> = Mutex::new(BTreeMap::new());
#[cfg(debug_assertions)]
pub fn mirror_count() -> Option<usize> {
    MIRRORS.try_lock().ok().map(|mirrors| mirrors.len())
}
/// How long one ask may wait on the registry (it answers from main).
const ASK: Duration = Duration::from_secs(2);

pub fn text_host(fonts: &RenderFonts) -> SharedTextHost {
    let _ = FONTS.set(fonts.sibling());
    SharedTextHost(Arc::new(RegistryText(fonts.sibling())))
}
/// Runs `f` against the project's mirror at `published`; a question that
/// needs fonts gets them from the registry and runs again. Off main only:
/// an ask waits for the registry.
pub fn query<T>(
    published: &Published,
    mut f: impl FnMut(&mut QueryContext) -> Result<T, QueryError>,
) -> Result<T, QueryError> {
    let mut mirrors = MIRRORS.lock().unwrap_or_else(|e| e.into_inner());
    let mirror = mirrors
        .entry(published.project.clone())
        .or_insert_with(|| QueryMirror::new(published));
    mirror.sync(published);
    let q = mirror.queries();
    for _ in 0..6 {
        match f(q) {
            Err(QueryError::NeedsFonts) if fill(q) => {}
            // Ink bounds (picking text, tight boxes) read glyph outlines.
            Err(QueryError::NeedsInk) if q.enable_font_outlines() => {}
            other => return other,
        }
    }
    f(q)
}
/// Answers what the last text question lacked; `false` when nothing can.
fn fill(q: &mut QueryContext) -> bool {
    let Some(fonts) = FONTS.get() else {
        return false;
    };
    let Ok(report) = serde_json::from_str::<serde_json::Value>(&q.font_report()) else {
        return false;
    };
    let misses: Vec<_> = serde_json::from_value(report["misses"].clone()).unwrap_or_default();
    let pending: Vec<(String, u32)> =
        serde_json::from_value(report["pending"].clone()).unwrap_or_default();
    if misses.is_empty() && pending.is_empty() {
        return false;
    }
    let Ok(reply) = fonts.broker.ask(misses, pending, ASK) else {
        return false;
    };
    let Ok(answers) = serde_json::to_vec(&reply.answers) else {
        return false;
    };
    q.add_fonts(&answers, reply.fonts).is_ok()
}

/// The faces the engine has for families the compat page's elements ask
/// for: `(family, weight, italic)` served under the family's own name, so a
/// hidden element an extension measures uses the face the engine draws that
/// family with. A family this Mac lacks is not served: the page falls back
/// to its standard font, as today's DOM did (Chromium's, Times), and an
/// extension measuring in it measures what it always measured (a
/// typewriter caret placed from such a measurement lands where it always
/// landed). Off main only: it waits on the registry.
pub fn page_faces(wanted: &[(String, u16, bool)]) -> Vec<pm_compat_host::PageFace> {
    use pm_text::font::{FontDesc, FontResolver};
    let Some(fonts) = FONTS.get() else {
        return vec![];
    };
    let mut resolver = BrokerResolver::new(fonts.broker.clone(), fonts.health.clone());
    wanted
        .iter()
        .filter_map(|(family, weight, italic)| {
            let desc = FontDesc {
                weight: f32::from(*weight),
                italic: *italic,
                size: 16.,
            };
            let id = resolver.family(family, &desc)?;
            let face = resolver.face(id);

            let bytes = pm_text::subset::whole_face(face.data.as_slice(), face.index).ok()?;
            Some(pm_compat_host::PageFace {
                family: family.clone(),
                weight: *weight,
                italic: *italic,
                bytes: bytes.into(),
            })
        })
        .collect()
}
