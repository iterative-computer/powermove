//! Adapter between the production remote-DOM protocol and the AppKit host.
//! One dedicated page per realm document; contributions mount inside that DOM.
use crate::{DomPanelHost, Error, PanelId, Result};
use pm_remote_dom::{
    Limits, Router, TrySink,
    protocol::{Batch, PageMessage, Session},
};
use serde_json::Value;

pub const REMOTE_DOM_SHELL: &str = include_str!("remote-dom.js");
const ENVELOPE_BYTES: usize =
    "{\"kind\":\"batch\",\"panel\":18446744073709551615,\"batch\":}".len();
pub const MAX_REMOTE_DOM_BATCH_BYTES: usize = crate::MAX_MESSAGE_BYTES - ENVELOPE_BYTES;

/// Created only after worker-side protocol admission. Keep a failed presentation
/// at the front of the UI queue and retry; admission is not a paint ack.
pub struct AdmittedDomBatch {
    generation: u64,
    panel: PanelId,
    payload: Value,
}
impl AdmittedDomBatch {
    pub fn present(&self, host: &mut impl DomPanelHost) -> Result<()> {
        if self.generation != host.generation() {
            return Err(Error::StaleGeneration);
        }
        host.apply(self.generation, self.panel, self.payload.clone())
    }
}

/// Own on the routing/V8 worker. AppKit receives only AdmittedDomBatch values;
/// it never validates an extension's tree or waits for its logic.
pub struct RemoteDomPanel {
    panel: PanelId,
    router: Router,
}
impl RemoteDomPanel {
    pub fn new(panel: PanelId, session: Session) -> Result<Self> {
        if panel == 0 || panel > pm_remote_dom::protocol::MAX_ID {
            return Err(Error::InvalidRequest);
        }
        Ok(Self {
            panel,
            router: Router::new(
                session,
                Limits {
                    bytes: MAX_REMOTE_DOM_BATCH_BYTES,
                    ..Limits::default()
                },
            )
            .map_err(|_| Error::InvalidRequest)?,
        })
    }
    /// Queue failure never advances the router. Retry the identical batch.
    pub fn enqueue(
        &mut self,
        value: Value,
        page: &mut impl TrySink<AdmittedDomBatch>,
    ) -> Result<()> {
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::InvalidRequest)?;
        let batch = self
            .router
            .decode_batch(&bytes)
            .map_err(|_| Error::InvalidRequest)?;
        struct Sink<'a, H>(&'a mut H, PanelId);
        impl<H: TrySink<AdmittedDomBatch>> TrySink<Batch> for Sink<'_, H> {
            fn try_send(&mut self, batch: Batch) -> std::result::Result<(), pm_remote_dom::Error> {
                self.0.try_send(AdmittedDomBatch {
                    generation: u64::from(batch.session.generation),
                    panel: self.1,
                    payload: serde_json::to_value(batch)
                        .map_err(|_| pm_remote_dom::Error::Invalid("batch"))?,
                })
            }
        }
        self.router
            .route_batch(batch, &mut Sink(page, self.panel))
            .map_err(|e| match e {
                pm_remote_dom::Error::Backpressure => Error::Backpressure,
                pm_remote_dom::Error::StaleSession => Error::StaleGeneration,
                _ => Error::InvalidRequest,
            })
    }
    /// Route actual WebKit feedback to a bounded V8-owner queue, never enter JS
    /// from AppKit. `applied` advances only after the page's acknowledgement.
    pub fn receive(&mut self, value: Value, realm: &mut impl TrySink<PageMessage>) -> Result<()> {
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::InvalidRequest)?;
        let message = self
            .router
            .decode_page(&bytes)
            .map_err(|_| Error::InvalidRequest)?;
        self.router.route_page(message, realm).map_err(|e| match e {
            pm_remote_dom::Error::Backpressure => Error::Backpressure,
            pm_remote_dom::Error::StaleSession => Error::StaleGeneration,
            _ => Error::InvalidRequest,
        })
    }
    pub fn inspection(&self) -> pm_remote_dom::Inspection {
        self.router.inspect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[derive(Default)]
    struct Page {
        full: bool,
        admitted: Vec<AdmittedDomBatch>,
    }
    impl TrySink<AdmittedDomBatch> for Page {
        fn try_send(
            &mut self,
            batch: AdmittedDomBatch,
        ) -> std::result::Result<(), pm_remote_dom::Error> {
            if self.full {
                return Err(pm_remote_dom::Error::Backpressure);
            }
            self.admitted.push(batch);
            Ok(())
        }
    }
    #[test]
    fn webkit_queue_admission_is_not_a_paint_ack_and_backpressure_is_retryable() {
        let session = Session {
            panel: "synthetic".into(),
            generation: 1,
        };
        let mut adapter = RemoteDomPanel::new(1, session.clone()).unwrap();
        let mut page = Page {
            full: true,
            ..Default::default()
        };
        let batch = json!({"version":1,"session":session,"seq":1,"caused_by":null,"ops":[]});
        assert_eq!(
            adapter.enqueue(batch.clone(), &mut page),
            Err(Error::Backpressure)
        );
        assert_eq!(adapter.inspection().sent, 0);
        page.full = false;
        adapter.enqueue(batch, &mut page).unwrap();
        assert_eq!(adapter.inspection().sent, 1);
        assert_eq!(adapter.inspection().applied, 0);
        let (mut realm, received) = pm_remote_dom::bounded_channel(1).unwrap();
        realm
            .try_send(PageMessage::Applied {
                session: session.clone(),
                seq: 0,
            })
            .unwrap();
        let acknowledgement = json!({"kind":"applied","session":session,"seq":1});
        assert_eq!(
            adapter.receive(acknowledgement.clone(), &mut realm),
            Err(Error::Backpressure)
        );
        assert_eq!(adapter.inspection().applied, 0);
        received.try_recv().unwrap();
        adapter.receive(acknowledgement, &mut realm).unwrap();
        assert_eq!(adapter.inspection().applied, 1);
        assert!(matches!(
            received.try_recv().unwrap(),
            PageMessage::Applied { seq: 1, .. }
        ));
        let stale =
            json!({"kind":"applied","session":{"panel":"synthetic","generation":2},"seq":1});
        assert_eq!(
            adapter.receive(stale, &mut realm),
            Err(Error::StaleGeneration)
        );
    }

    #[test]
    fn native_panel_ids_must_survive_javascript_transport_exactly() {
        let session = Session {
            panel: "synthetic".into(),
            generation: 1,
        };
        for panel in [0, pm_remote_dom::protocol::MAX_ID + 1, u64::MAX] {
            assert!(matches!(
                RemoteDomPanel::new(panel, session.clone()),
                Err(Error::InvalidRequest)
            ));
        }
        assert!(RemoteDomPanel::new(pm_remote_dom::protocol::MAX_ID, session).is_ok());
    }
}
