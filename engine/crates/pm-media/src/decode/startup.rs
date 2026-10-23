//! Opt-in, current-thread stage timings for media startup diagnostics.
//! The renderer's export decode runs on its calling thread. Other decode
//! workers are intentionally outside this current-thread measurement.

use serde::Serialize;
use std::cell::Cell;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupStages {
    pub probe_us: u64,
    pub probe_input_us: u64,
    pub index_us: u64,
    pub fingerprint_us: u64,
    /// Kept in reports to show the eliminated redundant demuxer open.
    pub extradata_input_us: u64,
    pub session_input_us: u64,
    pub decoder_create_us: u64,
    /// Building the VideoToolbox format description and output policy.
    pub vt_config_us: u64,
    /// Creating the hardware/software VideoToolbox decompression session.
    pub vt_session_us: u64,
    pub packet_reader_us: u64,
    pub session_build_us: u64,
    pub frame_decode_us: u64,
    pub source_opens: u32,
    pub session_creates: u32,
    pub frame_decodes: u32,
}

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    Probe,
    ProbeInput,
    Index,
    Fingerprint,
    SessionInput,
    DecoderCreate,
    VtConfig,
    VtSession,
    PacketReader,
    SessionBuild,
    FrameDecode,
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static STAGES: Cell<StartupStages> = const { Cell::new(StartupStages {
        probe_us: 0, probe_input_us: 0, index_us: 0, fingerprint_us: 0,
        extradata_input_us: 0, session_input_us: 0, decoder_create_us: 0,
        vt_config_us: 0, vt_session_us: 0,
        packet_reader_us: 0, session_build_us: 0, frame_decode_us: 0,
        source_opens: 0, session_creates: 0, frame_decodes: 0,
    }) };
}

/// Begin a fresh diagnostic span on this thread. No work is timed until called.
pub fn begin() {
    STAGES.set(StartupStages::default());
    ENABLED.set(true);
}

/// Finish the diagnostic span and disable instrumentation on this thread.
pub fn finish() -> StartupStages {
    ENABLED.set(false);
    STAGES.replace(StartupStages::default())
}

pub(crate) fn enabled() -> bool {
    ENABLED.get()
}

pub(crate) fn timed<T>(stage: Stage, f: impl FnOnce() -> T) -> T {
    if !ENABLED.get() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    record_elapsed(stage, start.elapsed());
    result
}

/// Record a stage performed on a scoped helper thread in the caller's span.
/// Its duration may overlap other stages; it is not a critical-path sum.
pub(crate) fn record_elapsed(stage: Stage, elapsed: Duration) {
    if !ENABLED.get() {
        return;
    }
    let elapsed = elapsed.as_micros().min(u64::MAX as u128) as u64;
    STAGES.with(|cell| {
        let mut value = cell.get();
        let field = match stage {
            Stage::Probe => &mut value.probe_us,
            Stage::ProbeInput => &mut value.probe_input_us,
            Stage::Index => &mut value.index_us,
            Stage::Fingerprint => &mut value.fingerprint_us,
            Stage::SessionInput => &mut value.session_input_us,
            Stage::DecoderCreate => &mut value.decoder_create_us,
            Stage::VtConfig => &mut value.vt_config_us,
            Stage::VtSession => &mut value.vt_session_us,
            Stage::PacketReader => &mut value.packet_reader_us,
            Stage::SessionBuild => &mut value.session_build_us,
            Stage::FrameDecode => &mut value.frame_decode_us,
        };
        *field = field.saturating_add(elapsed);
        cell.set(value);
    });
}

pub(crate) fn source_opened() {
    if ENABLED.get() {
        STAGES.with(|cell| {
            let mut value = cell.get();
            value.source_opens = value.source_opens.saturating_add(1);
            cell.set(value);
        });
    }
}

pub(crate) fn session_created() {
    if ENABLED.get() {
        STAGES.with(|cell| {
            let mut value = cell.get();
            value.session_creates = value.session_creates.saturating_add(1);
            cell.set(value);
        });
    }
}

pub(crate) fn frame_decoded() {
    if ENABLED.get() {
        STAGES.with(|cell| {
            let mut value = cell.get();
            value.frame_decodes = value.frame_decodes.saturating_add(1);
            cell.set(value);
        });
    }
}
