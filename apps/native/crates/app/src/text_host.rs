//! The kernel's fonts: the same registry rendering asks, measured on the
//! kernel's own thread, so a command never waits on the main thread's turn.
use pm_kernel::text_host::{SharedTextHost, TextHost};
use pm_node::fonts::render::{BrokerResolver, RenderFonts};
use std::sync::Arc;

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
pub fn text_host(fonts: &RenderFonts) -> SharedTextHost {
    SharedTextHost(Arc::new(RegistryText(fonts.sibling())))
}
