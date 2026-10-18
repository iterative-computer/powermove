//! Opt-in cold-start timing. Disabled in normal playback; no callback work.
use std::{cell::Cell, sync::OnceLock, time::Instant};
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub bounds_us: u64,
    pub map_us: u64,
    pub read_us: u64,
    pub dsp_us: u64,
    pub dsp_init_us: u64,
    pub dsp_seek_us: u64,
    pub dsp_process_us: u64,
    pub hold_us: u64,
    /// Producer wall wait, overlaps the summed worker scopes above.
    pub prepare_wait_us: u64,
    pub first_render_us: u64,
}
thread_local! { static COUNTS: Cell<Profile> = Cell::new(Profile::default()); }
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PM_AUDIO_PROFILE").is_some())
}
pub(crate) fn reset() {
    COUNTS.set(Profile::default());
}
pub(crate) fn snapshot() -> Profile {
    COUNTS.get()
}
pub(crate) enum Kind {
    Bounds,
    Map,
    Read,
    Dsp,
    DspInit,
    DspSeek,
    DspProcess,
    Hold,
    PrepareWait,
    FirstRender,
}
pub(crate) struct Timer(Option<Instant>, Kind);
pub(crate) fn timer(kind: Kind) -> Timer {
    Timer(enabled().then(Instant::now), kind)
}
impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(start) = self.0 {
            let us = start.elapsed().as_nanos() as f64 / 1000.0;
            COUNTS.with(|c| {
                let mut p = c.get();
                let field = match self.1 {
                    Kind::Bounds => &mut p.bounds_us,
                    Kind::Map => &mut p.map_us,
                    Kind::Read => &mut p.read_us,
                    Kind::Dsp => &mut p.dsp_us,
                    Kind::DspInit => &mut p.dsp_init_us,
                    Kind::DspSeek => &mut p.dsp_seek_us,
                    Kind::DspProcess => &mut p.dsp_process_us,
                    Kind::Hold => &mut p.hold_us,
                    Kind::PrepareWait => &mut p.prepare_wait_us,
                    Kind::FirstRender => &mut p.first_render_us,
                };
                *field += us.round() as u64;
                c.set(p);
            });
        }
    }
}

pub(crate) fn add(other: Profile) {
    COUNTS.with(|c| {
        let mut p = c.get();
        p.bounds_us += other.bounds_us;
        p.map_us += other.map_us;
        p.read_us += other.read_us;
        p.dsp_us += other.dsp_us;
        p.dsp_init_us += other.dsp_init_us;
        p.dsp_seek_us += other.dsp_seek_us;
        p.dsp_process_us += other.dsp_process_us;
        p.hold_us += other.hold_us;
        p.prepare_wait_us += other.prepare_wait_us;
        p.first_render_us += other.first_render_us;
        c.set(p);
    });
}
