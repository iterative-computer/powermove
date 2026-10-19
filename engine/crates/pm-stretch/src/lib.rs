//! Pitch-preserving time stretch (plan section 16): our own binding to
//! Signalsmith Stretch (MIT, vendored in `vendor/`, see its README).
//!
//! [`Stretcher`] is a phase-vocoder time stretch over planar `f32`
//! channels. Each [`Stretcher::process`] call takes some input and returns
//! some output; the ratio of the two is the playback rate of that call
//! (2 input samples per output sample play twice as fast at the same
//! pitch). The library's latency is reported in two halves
//! ([`Stretcher::input_latency`], [`Stretcher::output_latency`]): input is
//! supplied `input_latency` samples ahead of the processing time and
//! output arrives `output_latency` samples behind it.
//!
//! Determinism: the library's portable FFT is compiled without fast math,
//! and its random engine (used only below 0.5× speed, to decorrelate
//! phases) is seeded at construction and reseeded by [`Stretcher::reset`],
//! so the same calls on the same input give the same bytes on every
//! machine.

use std::ffi::{c_int, c_long, c_void};
use std::ptr::NonNull;

unsafe extern "C" {
    fn pm_stretch_new(channels: c_int, block: c_int, interval: c_int, seed: c_long) -> *mut c_void;
    fn pm_stretch_free(handle: *mut c_void);
    fn pm_stretch_input_latency(handle: *const c_void) -> c_int;
    fn pm_stretch_output_latency(handle: *const c_void) -> c_int;
    fn pm_stretch_block_samples(handle: *const c_void) -> c_int;
    fn pm_stretch_interval_samples(handle: *const c_void) -> c_int;
    fn pm_stretch_seek_length(handle: *const c_void) -> c_int;
    fn pm_stretch_output_seek_length(handle: *const c_void, rate: f32) -> c_int;
    fn pm_stretch_reset(handle: *mut c_void) -> c_int;
    fn pm_stretch_set_transpose(handle: *mut c_void, factor: f32) -> c_int;
    fn pm_stretch_seek(
        handle: *mut c_void,
        inputs: *const *const f32,
        samples: c_int,
        rate: f64,
    ) -> c_int;
    fn pm_stretch_output_seek(
        handle: *mut c_void,
        inputs: *const *const f32,
        samples: c_int,
        checkpoint: Option<extern "C" fn()>,
    ) -> c_int;
    fn pm_stretch_process(
        handle: *mut c_void,
        inputs: *const *const f32,
        input_samples: c_int,
        outputs: *const *mut f32,
        output_samples: c_int,
        checkpoint: Option<extern "C" fn()>,
    ) -> c_int;
    fn pm_stretch_exact(
        handle: *mut c_void,
        inputs: *const *const f32,
        input_samples: c_int,
        outputs: *const *mut f32,
        output_samples: c_int,
    ) -> c_int;
    fn pm_stretch_flush(
        handle: *mut c_void,
        outputs: *const *mut f32,
        output_samples: c_int,
        rate: f32,
        checkpoint: Option<extern "C" fn()>,
    ) -> c_int;
}

/// The seed every stretcher starts from (any fixed value keeps runs
/// reproducible).
pub const SEED: i64 = 0x504d_5354;

/// The library's default preset: 120 ms analysis blocks every 30 ms.
pub const DEFAULT_BLOCK_SECONDS: f64 = 0.12;
pub const DEFAULT_INTERVAL_SECONDS: f64 = 0.03;

/// The most channels one stretcher processes (the widest layout pm-media
/// reports is 24 channels, 22.2).
pub const MAX_CHANNELS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StretchError {
    /// The library could not allocate.
    Alloc,
    /// Arguments of the wrong shape (channel count, lengths).
    Shape(String),
}

impl std::fmt::Display for StretchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StretchError::Alloc => write!(f, "the time stretch could not allocate"),
            StretchError::Shape(m) => write!(f, "time stretch: {m}"),
        }
    }
}

impl std::error::Error for StretchError {}

pub type Result<T> = std::result::Result<T, StretchError>;

/// One phase-vocoder time stretch over `channels` planar channels.
pub struct Stretcher {
    handle: NonNull<c_void>,
    channels: usize,
    inputs: Vec<*const f32>,
    outputs: Vec<*mut f32>,
}

// SAFETY: the handle owns its C++ object exclusively; nothing in it is tied
// to the creating thread. `&mut self` serializes every call.
unsafe impl Send for Stretcher {}

fn status(code: c_int) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(StretchError::Alloc)
    }
}

impl Stretcher {
    /// The library's default preset for `sample_rate`.
    pub fn new(channels: usize, sample_rate: u32) -> Result<Stretcher> {
        let block = (sample_rate as f64 * DEFAULT_BLOCK_SECONDS) as usize;
        let interval = (sample_rate as f64 * DEFAULT_INTERVAL_SECONDS) as usize;
        Stretcher::with_config(channels, block, interval, SEED)
    }

    /// Analysis blocks of `block` samples every `interval` samples.
    pub fn with_config(
        channels: usize,
        block: usize,
        interval: usize,
        seed: i64,
    ) -> Result<Stretcher> {
        if !(1..=MAX_CHANNELS).contains(&channels) {
            return Err(StretchError::Shape(format!(
                "{channels} channels (1 to {MAX_CHANNELS})"
            )));
        }
        if interval == 0 || block < 2 * interval || block > 1 << 20 {
            return Err(StretchError::Shape(format!(
                "block {block} and interval {interval}"
            )));
        }
        // SAFETY: plain values; a null return is an allocation failure.
        let raw = unsafe {
            pm_stretch_new(
                channels as c_int,
                block as c_int,
                interval as c_int,
                seed as c_long,
            )
        };
        let handle = NonNull::new(raw).ok_or(StretchError::Alloc)?;
        Ok(Stretcher {
            handle,
            channels,
            inputs: vec![std::ptr::null(); channels],
            outputs: vec![std::ptr::null_mut(); channels],
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Samples of input supplied ahead of the processing time.
    pub fn input_latency(&self) -> usize {
        // SAFETY: a live handle.
        unsafe { pm_stretch_input_latency(self.handle.as_ptr()) as usize }
    }

    /// Samples of output behind the processing time.
    pub fn output_latency(&self) -> usize {
        // SAFETY: a live handle.
        unsafe { pm_stretch_output_latency(self.handle.as_ptr()) as usize }
    }

    pub fn block_samples(&self) -> usize {
        // SAFETY: a live handle.
        unsafe { pm_stretch_block_samples(self.handle.as_ptr()) as usize }
    }

    /// Output samples between two analyses.
    pub fn interval_samples(&self) -> usize {
        // SAFETY: a live handle.
        unsafe { pm_stretch_interval_samples(self.handle.as_ptr()) as usize }
    }

    /// The input [`Self::seek`] ideally receives.
    pub fn seek_length(&self) -> usize {
        // SAFETY: a live handle.
        unsafe { pm_stretch_seek_length(self.handle.as_ptr()) as usize }
    }

    /// The input [`Self::output_seek`] needs at `rate`.
    pub fn output_seek_length(&self, rate: f64) -> usize {
        // SAFETY: a live handle.
        unsafe { pm_stretch_output_seek_length(self.handle.as_ptr(), rate as f32).max(0) as usize }
    }

    /// Back to a new instance's state (the random engine reseeded).
    pub fn reset(&mut self) -> Result<()> {
        // SAFETY: a live handle.
        status(unsafe { pm_stretch_reset(self.handle.as_ptr()) })
    }

    /// Shifts every frequency by `factor` (1: none).
    pub fn set_transpose(&mut self, factor: f64) -> Result<()> {
        // SAFETY: a live handle.
        status(unsafe { pm_stretch_set_transpose(self.handle.as_ptr(), factor as f32) })
    }

    fn input_pointers(&mut self, input: &[&[f32]]) -> Result<usize> {
        if input.len() != self.channels {
            return Err(StretchError::Shape(format!(
                "{} input channels for {}",
                input.len(),
                self.channels
            )));
        }
        let n = input[0].len();
        if input.iter().any(|c| c.len() != n) || n > c_int::MAX as usize {
            return Err(StretchError::Shape(
                "input channels differ in length".into(),
            ));
        }
        for (p, c) in self.inputs.iter_mut().zip(input) {
            *p = c.as_ptr();
        }
        Ok(n)
    }

    fn output_pointers(&mut self, output: &mut [&mut [f32]]) -> Result<usize> {
        if output.len() != self.channels {
            return Err(StretchError::Shape(format!(
                "{} output channels for {}",
                output.len(),
                self.channels
            )));
        }
        let n = output[0].len();
        if output.iter().any(|c| c.len() != n) || n > c_int::MAX as usize {
            return Err(StretchError::Shape(
                "output channels differ in length".into(),
            ));
        }
        for (p, c) in self.outputs.iter_mut().zip(output.iter_mut()) {
            *p = c.as_mut_ptr();
        }
        Ok(n)
    }

    /// Supplies input history before the processing time without output
    /// (a smooth change of input position); ideally [`Self::seek_length`]
    /// samples, ending `input_latency` samples past the new processing
    /// time.
    pub fn seek(&mut self, input: &[&[f32]], rate: f64) -> Result<()> {
        let n = self.input_pointers(input)?;
        // SAFETY: `inputs` holds `channels` pointers to `n` readable floats.
        status(unsafe {
            pm_stretch_seek(self.handle.as_ptr(), self.inputs.as_ptr(), n as c_int, rate)
        })
    }

    /// Resets and starts at the first sample of `input` (which holds
    /// [`Self::output_seek_length`] samples for the rate to come): the next
    /// output of [`Self::process`] is aligned with that sample.
    pub fn output_seek(&mut self, input: &[&[f32]]) -> Result<()> {
        self.output_seek_inner(input, None)
    }

    /// The same output seek, with checkpoints inside its pre-roll processing.
    /// The callback must not panic or unwind over C++.
    pub fn output_seek_with_checkpoint(
        &mut self,
        input: &[&[f32]],
        checkpoint: extern "C" fn(),
    ) -> Result<()> {
        self.output_seek_inner(input, Some(checkpoint))
    }

    fn output_seek_inner(
        &mut self,
        input: &[&[f32]],
        checkpoint: Option<extern "C" fn()>,
    ) -> Result<()> {
        let n = self.input_pointers(input)?;
        // SAFETY: as in `seek`.
        status(unsafe {
            pm_stretch_output_seek(
                self.handle.as_ptr(),
                self.inputs.as_ptr(),
                n as c_int,
                checkpoint,
            )
        })
    }

    /// Consumes every sample of `input` and fills every sample of `output`.
    /// `input` may be empty (a frozen spectrum, or silence).
    pub fn process(&mut self, input: &[&[f32]], output: &mut [&mut [f32]]) -> Result<()> {
        self.process_inner(input, output, None)
    }

    /// The same processing call, with a cooperative callback before each
    /// indivisible DSP step. The callback must not panic or unwind over C++.
    pub fn process_with_checkpoint(
        &mut self,
        input: &[&[f32]],
        output: &mut [&mut [f32]],
        checkpoint: extern "C" fn(),
    ) -> Result<()> {
        self.process_inner(input, output, Some(checkpoint))
    }

    fn process_inner(
        &mut self,
        input: &[&[f32]],
        output: &mut [&mut [f32]],
        checkpoint: Option<extern "C" fn()>,
    ) -> Result<()> {
        let n_in = if input.is_empty() {
            for p in &mut self.inputs {
                *p = std::ptr::null();
            }
            0
        } else {
            self.input_pointers(input)?
        };
        let n_out = self.output_pointers(output)?;
        // SAFETY: `inputs` points at `n_in` readable floats per channel (or
        // is unused when `n_in` is 0), `outputs` at `n_out` writable floats
        // per channel, for `channels` channels.
        status(unsafe {
            pm_stretch_process(
                self.handle.as_ptr(),
                self.inputs.as_ptr(),
                n_in as c_int,
                self.outputs.as_ptr(),
                n_out as c_int,
                checkpoint,
            )
        })
    }

    /// Reads the remaining output with no further input.
    pub fn flush(&mut self, output: &mut [&mut [f32]], rate: f64) -> Result<()> {
        self.flush_inner(output, rate, None)
    }

    /// The same flush, with checkpoints inside any tail processing.
    /// The callback must not panic or unwind over C++.
    pub fn flush_with_checkpoint(
        &mut self,
        output: &mut [&mut [f32]],
        rate: f64,
        checkpoint: extern "C" fn(),
    ) -> Result<()> {
        self.flush_inner(output, rate, Some(checkpoint))
    }

    fn flush_inner(
        &mut self,
        output: &mut [&mut [f32]],
        rate: f64,
        checkpoint: Option<extern "C" fn()>,
    ) -> Result<()> {
        let n = self.output_pointers(output)?;
        // SAFETY: as in `process`.
        status(unsafe {
            pm_stretch_flush(
                self.handle.as_ptr(),
                self.outputs.as_ptr(),
                n as c_int,
                rate as f32,
                checkpoint,
            )
        })
    }
}

impl Stretcher {
    /// The library's one-shot stretch of a whole buffer into `output`
    /// (input and output lengths set the rate): the reference a streaming
    /// use is measured against. Returns false when the input is too short.
    pub fn exact(&mut self, input: &[&[f32]], output: &mut [&mut [f32]]) -> Result<bool> {
        let n_in = self.input_pointers(input)?;
        let n_out = self.output_pointers(output)?;
        // SAFETY: as in `process`.
        let r = unsafe {
            pm_stretch_exact(
                self.handle.as_ptr(),
                self.inputs.as_ptr(),
                n_in as c_int,
                self.outputs.as_ptr(),
                n_out as c_int,
            )
        };
        match r {
            1 => Ok(true),
            0 => Ok(false),
            _ => Err(StretchError::Alloc),
        }
    }
}

impl Drop for Stretcher {
    fn drop(&mut self) {
        // SAFETY: the handle came from `pm_stretch_new` and is freed once.
        unsafe { pm_stretch_free(self.handle.as_ptr()) }
    }
}

impl std::fmt::Debug for Stretcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stretcher")
            .field("channels", &self.channels)
            .field("block", &self.block_samples())
            .field("interval", &self.interval_samples())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_shapes() {
        assert!(Stretcher::new(0, 48_000).is_err());
        assert!(Stretcher::new(MAX_CHANNELS + 1, 48_000).is_err());
        let mut s = Stretcher::new(2, 48_000).unwrap();
        let a = [0f32; 16];
        let mut out = [0f32; 16];
        assert!(s.process(&[&a], &mut [&mut out]).is_err());
    }

    #[test]
    fn reports_the_default_preset() {
        let s = Stretcher::new(2, 48_000).unwrap();
        assert_eq!(s.block_samples(), 5760);
        assert_eq!(s.interval_samples(), 1440);
        assert!(s.input_latency() > 0 && s.output_latency() > 0);
    }
}
