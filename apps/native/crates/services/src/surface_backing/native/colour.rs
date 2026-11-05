//! N004. Actual pm-promedia CPU scopes/LUT workers; no visual scale or tone map.
use super::super::{Session, Ticket, invalid, stale, ticket};
use crate::{Result, ServiceError, ServiceErrorCode};
use pm_promedia::{
    lut::CubeLut,
    scopes::{self, ScopeConfig, Scopes},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisStage {
    SourceCodeValues,
    WorkingLinear,
    DisplayEncoded,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameIdentity {
    pub session: Session,
    pub revision: u64,
    pub frame: i64,
    pub colour_pipeline: String,
}
#[derive(Clone, Debug)]
pub struct ScopeRequest {
    pub ticket: Ticket,
    pub identity: FrameIdentity,
    pub stage: AnalysisStage,
    pub config: ScopeConfig,
}
#[derive(Clone, Debug, PartialEq)]
pub enum AnalysisState {
    Empty,
    Pending,
    Ready(Arc<Scopes>),
    Failed(ServiceErrorCode),
}
pub struct ColourAnalysis {
    pub state: AnalysisState,
    /// Smoothing/animation isn't performed here. Waiting retains the last real
    /// analysis; a read error clears it. The UI shows no spinner before 500 ms.
    pub previous: Option<Arc<Scopes>>,
    pub pending_since_ms: Option<u64>,
    last_sample_ms: Option<u64>,
    active: Option<ScopeRequest>,
}
impl Default for ColourAnalysis {
    fn default() -> Self {
        Self {
            state: AnalysisState::Empty,
            previous: None,
            pending_since_ms: None,
            last_sample_ms: None,
            active: None,
        }
    }
}
impl ColourAnalysis {
    pub fn try_begin(
        &mut self,
        identity: FrameIdentity,
        stage: AnalysisStage,
        config: ScopeConfig,
        now_ms: u64,
        playing: bool,
        visible: bool,
    ) -> Option<ScopeRequest> {
        if !visible
            || (playing
                && self
                    .last_sample_ms
                    .is_some_and(|at| now_ms.saturating_sub(at) < 100))
        {
            return None;
        }
        self.last_sample_ms = Some(now_ms);
        self.pending_since_ms = Some(now_ms);
        Some(self.begin(identity, stage, config))
    }
    pub fn begin(
        &mut self,
        identity: FrameIdentity,
        stage: AnalysisStage,
        config: ScopeConfig,
    ) -> ScopeRequest {
        let request = ScopeRequest {
            ticket: ticket(),
            identity,
            stage,
            config,
        };
        self.active = Some(request.clone());
        self.state = AnalysisState::Pending;
        request
    }
    pub fn invalidate(&mut self) {
        self.active = None;
        self.state = AnalysisState::Empty;
        self.previous = None;
        self.pending_since_ms = None;
    }
    pub fn settle(
        &mut self,
        request: &ScopeRequest,
        current: &FrameIdentity,
        result: Result<Scopes>,
    ) -> Result<()> {
        if self.active.as_ref().map(|r| r.ticket) != Some(request.ticket) {
            return Err(stale());
        }
        self.active = None;
        self.pending_since_ms = None;
        if &request.identity != current {
            self.state = AnalysisState::Empty;
            self.previous = None;
            return Err(stale());
        }
        match result {
            Ok(scopes) => {
                let scopes = Arc::new(scopes);
                self.previous = Some(scopes.clone());
                self.state = AnalysisState::Ready(scopes);
            }
            Err(e) => {
                self.previous = None;
                self.state = AnalysisState::Failed(e.code);
                return Err(e);
            }
        }
        Ok(())
    }
}
/// Frame provider must confirm the exact analysis stage; no implicit transfer
/// function/LUT/tone mapping and no clamping HDR code values to SDR.
pub fn compute(
    request: &ScopeRequest,
    supplied_stage: AnalysisStage,
    rgb: &[[f64; 3]],
    width: usize,
    height: usize,
) -> Result<Scopes> {
    if supplied_stage != request.stage {
        return Err(invalid("analysis stage mismatch"));
    }
    scopes::compute(rgb, width, height, request.config).map_err(|e| {
        ServiceError::message(
            ServiceErrorCode::InvalidData,
            format!("scope analysis: {e:?}"),
        )
    })
}
pub fn parse_lut(source: &str) -> Result<CubeLut> {
    if source.len() > 128 * 1024 * 1024 {
        return Err(ServiceError::message(
            ServiceErrorCode::TooLarge,
            "LUT source exceeds limit",
        ));
    }
    CubeLut::parse(source).map_err(|e| ServiceError::new(ServiceErrorCode::InvalidData, e))
}
/// Inspect transform only. Export/preview adoption must be an explicit kernel
/// colour-pipeline edit owned by the engine service, never a panel-local effect.
pub fn inspect_lut(lut: &CubeLut, rgb: [f64; 3]) -> Result<[f64; 3]> {
    lut.apply(rgb)
        .map_err(|e| ServiceError::new(ServiceErrorCode::InvalidData, e))
}
