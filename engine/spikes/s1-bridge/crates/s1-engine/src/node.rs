//! napi bindings (feature `node`): the addon surface used by the utility process.
use crate::engine;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use s1_mach::wire::PixelFormat;

#[napi(object)]
pub struct EngineConfig {
    pub width: u32,
    pub height: u32,
    /// "bgra" or "rgbaf16".
    pub format: String,
    pub pool_size: u32,
    pub lead_frames: u32,
    pub extra_passes: u32,
}

/// The engine as seen from the utility process's JavaScript.
#[napi(js_name = "Engine")]
pub struct JsEngine {
    inner: engine::Engine,
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::from_reason(e.to_string())
}

#[napi]
impl JsEngine {
    /// Creates the GPU device and pool, checks in with main through the per-launch
    /// bootstrap name in `PM_S1_BRIDGE_NAME`, sends every surface once, and starts the
    /// render thread (idle).
    #[napi(constructor)]
    pub fn new(config: EngineConfig) -> Result<Self> {
        let format = match config.format.as_str() {
            "bgra" => PixelFormat::Bgra8,
            "rgbaf16" => PixelFormat::Rgba16F,
            other => return Err(err(format!("unknown format {other}"))),
        };
        let bridge_name = std::env::var("PM_S1_BRIDGE_NAME").map_err(|_| err("PM_S1_BRIDGE_NAME is not set"))?;
        if config.pool_size == 0 || config.pool_size > s1_mach::wire::MAX_POOL {
            return Err(err("pool_size out of range"));
        }
        let inner = engine::Engine::start(engine::Config {
            width: config.width,
            height: config.height,
            format,
            pool_size: config.pool_size,
            lead_frames: config.lead_frames,
            extra_passes: config.extra_passes,
            bridge_name,
        })
        .map_err(err)?;
        Ok(Self { inner })
    }

    #[napi]
    pub fn info(&self) -> Result<String> {
        serde_json::to_string(&self.inner.info).map_err(err)
    }

    #[napi]
    pub fn set_mode(&self, mode: String) -> Result<()> {
        let m = engine::Mode::parse(&mode).ok_or_else(|| err(format!("unknown mode {mode}")))?;
        self.inner.set_mode(m);
        Ok(())
    }

    #[napi]
    pub fn set_lead_frames(&self, lead: u32) {
        self.inner.set_lead_frames(lead);
    }

    #[napi]
    pub fn set_display_timing(&self, anchor_us: f64, period_us: f64) -> Result<()> {
        if !(anchor_us.is_finite() && period_us.is_finite() && period_us > 2000.0 && period_us < 50_000.0) {
            return Err(err("invalid display timing"));
        }
        self.inner.set_display_timing(anchor_us, period_us);
        Ok(())
    }

    /// kind 0: pointer at (x, y) frame pixels; kind 1: walk key.
    #[napi]
    pub fn input(&self, revision: f64, x: f64, y: f64, ts_us: f64, kind: u32) {
        self.inner.input(revision as u64, x as f32, y as f32, ts_us as i64, kind);
    }

    #[napi]
    pub fn release(&self, pool_index: u32, seq: f64) {
        self.inner.release(pool_index, seq as u64);
    }

    #[napi]
    pub fn stats(&self) -> Result<String> {
        let s = self.inner.stats();
        let busy = self.inner.busy_slots();
        serde_json::to_string(&serde_json::json!({ "stats": s, "busy": busy })).map_err(err)
    }

    /// Drains the per-frame log as a JSON array.
    #[napi]
    pub fn take_log(&self) -> Result<String> {
        serde_json::to_string(&self.inner.take_log()).map_err(err)
    }

    #[napi]
    pub fn stop(&mut self) {
        self.inner.stop();
    }
}

/// Microseconds on CLOCK_UPTIME_RAW (Chromium's TimeTicks clock on macOS).
#[napi]
pub fn now_us() -> f64 {
    s1_mach::now_us() as f64
}
