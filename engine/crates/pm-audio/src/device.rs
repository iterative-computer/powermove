//! The output device: `cpal` on CoreAudio, and the presentation latency the
//! master clock folds in (contract 6 invariant 3).
//!
//! CoreAudio's render callback carries the host time of the buffer's first
//! frame at the device's output (the HAL's output time stamp). The listener
//! hears it later by the device latency and the output stream latency
//! (`kAudioDevicePropertyLatency` + `kAudioStreamPropertyLatency`), the sum
//! Chromium's AUHAL output uses for its playout time. The safety offset is
//! the HAL's scheduling margin before the output time and is not added.
//! (cpal's own `playback` estimate adds the I/O buffer as well, which would
//! put the clock a buffer late, about 10 ms at 512 frames.)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::error::{AudioError, Result};

/// What the default output device reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub sample_rate: u32,
    pub channels: usize,
    /// Device latency plus output stream latency, in frames.
    pub latency_frames: u32,
    pub safety_offset_frames: u32,
    pub buffer_frames: u32,
}

impl DeviceInfo {
    pub fn latency_us(&self) -> u32 {
        (self.latency_frames as u64 * 1_000_000 / self.sample_rate.max(1) as u64) as u32
    }
}

#[cfg(target_os = "macos")]
pub mod coreaudio {
    //! The few CoreAudio HAL properties the clock needs.
    use std::ffi::c_void;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    const fn fourcc(s: &[u8; 4]) -> u32 {
        ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
    }

    const SYSTEM: u32 = 1;
    const GLOBAL: u32 = fourcc(b"glob");
    const OUTPUT: u32 = fourcc(b"outp");
    const MAIN: u32 = 0;
    const DEFAULT_OUTPUT: u32 = fourcc(b"dOut");
    const LATENCY: u32 = fourcc(b"ltnc");
    const SAFETY: u32 = fourcc(b"saft");
    const STREAMS: u32 = fourcc(b"stm#");
    const BUFFER: u32 = fourcc(b"fsiz");
    const NOMINAL_RATE: u32 = fourcc(b"nsrt");

    fn get<T: Copy + Default>(object: u32, selector: u32, scope: u32) -> Option<T> {
        let address = Address {
            selector,
            scope,
            element: MAIN,
        };
        let mut value = T::default();
        let mut size = std::mem::size_of::<T>() as u32;
        // SAFETY: `value` is a writable T of `size` bytes.
        let r = unsafe {
            AudioObjectGetPropertyData(
                object,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                &mut value as *mut T as *mut c_void,
            )
        };
        (r == 0 && size as usize == std::mem::size_of::<T>()).then_some(value)
    }

    /// The default output device id.
    pub fn default_output() -> Option<u32> {
        get::<u32>(SYSTEM, DEFAULT_OUTPUT, GLOBAL).filter(|&d| d != 0)
    }

    /// (device latency + first output stream latency, safety offset,
    /// buffer frames, nominal rate) of a device.
    pub fn latency(device: u32) -> Option<(u32, u32, u32, f64)> {
        let device_latency = get::<u32>(device, LATENCY, OUTPUT)?;
        let safety = get::<u32>(device, SAFETY, OUTPUT).unwrap_or(0);
        let buffer = get::<u32>(device, BUFFER, GLOBAL).unwrap_or(0);
        let rate = get::<f64>(device, NOMINAL_RATE, GLOBAL).unwrap_or(0.0);
        let address = Address {
            selector: STREAMS,
            scope: OUTPUT,
            element: MAIN,
        };
        let mut size = 0u32;
        // SAFETY: a size query.
        let r = unsafe {
            AudioObjectGetPropertyDataSize(device, &address, 0, std::ptr::null(), &mut size)
        };
        let mut stream_latency = 0;
        if r == 0 && size >= 4 {
            let mut ids = vec![0u32; size as usize / 4];
            // SAFETY: `ids` holds `size` bytes.
            let r = unsafe {
                AudioObjectGetPropertyData(
                    device,
                    &address,
                    0,
                    std::ptr::null(),
                    &mut size,
                    ids.as_mut_ptr() as *mut c_void,
                )
            };
            if r == 0
                && let Some(&stream) = ids.first()
            {
                stream_latency = get::<u32>(stream, LATENCY, GLOBAL).unwrap_or(0);
            }
        }
        Some((device_latency + stream_latency, safety, buffer, rate))
    }
}

/// The default output device's id and its presentation latency in µs
/// (device + stream latency), for following device changes. The latency is
/// counted in the device's own frames, so it converts at that device's
/// nominal rate; `rate` (the open stream's) stands in when the device does
/// not report one.
pub fn default_output_state(rate: u32) -> Option<(u32, u32)> {
    #[cfg(target_os = "macos")]
    {
        let id = coreaudio::default_output()?;
        let (latency, _, _, nominal) = coreaudio::latency(id)?;
        let rate = if nominal.is_finite() && nominal >= 1.0 {
            nominal
        } else {
            rate.max(1) as f64
        };
        Some((id, (latency as f64 * 1e6 / rate).round() as u32))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = rate;
        None
    }
}

/// The default output device's format and latency.
pub fn default_output_info() -> Result<DeviceInfo> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| AudioError::Device("no output device".into()))?;
    let config = device
        .default_output_config()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    let (latency, safety, buffer) = coreaudio::default_output()
        .and_then(coreaudio::latency)
        .map(|(l, s, b, _)| (l, s, b))
        .unwrap_or((0, 0, 0));
    #[cfg(not(target_os = "macos"))]
    let (latency, safety, buffer) = (0, 0, 0);
    Ok(DeviceInfo {
        name,
        sample_rate: config.sample_rate(),
        channels: config.channels() as usize,
        latency_frames: latency,
        safety_offset_frames: safety,
        buffer_frames: buffer,
    })
}

/// A running output stream. The callback receives the interleaved buffer,
/// its channel count, and the host time (µs) of its first frame at the
/// device output.
pub struct OutputStream {
    stream: cpal::Stream,
    pub info: DeviceInfo,
    failed: Arc<AtomicBool>,
    cpal_delay_us: Arc<AtomicU64>,
    callbacks: Arc<AtomicU64>,
}

impl OutputStream {
    pub fn open(
        mut callback: impl FnMut(&mut [f32], usize, u64) + Send + 'static,
    ) -> Result<OutputStream> {
        let info = default_output_info()?;
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| AudioError::Device("no output device".into()))?;
        let config = cpal::StreamConfig {
            channels: info.channels as u16,
            sample_rate: info.sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let channels = info.channels;
        let failed = Arc::new(AtomicBool::new(false));
        let flag = failed.clone();
        let cpal_delay_us = Arc::new(AtomicU64::new(0));
        let delay = cpal_delay_us.clone();
        let callbacks = Arc::new(AtomicU64::new(0));
        let count = callbacks.clone();
        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                move |data: &mut [f32], cb: &cpal::OutputCallbackInfo| {
                    let ts = cb.timestamp();
                    let host_us = (ts.callback.as_nanos() / 1_000) as u64;
                    delay.store(
                        (ts.playback
                            .as_nanos()
                            .saturating_sub(ts.callback.as_nanos())
                            / 1_000) as u64,
                        Ordering::Relaxed,
                    );
                    count.fetch_add(1, Ordering::Relaxed);
                    callback(data, channels, host_us);
                },
                move |_e| flag.store(true, Ordering::Relaxed),
                None,
            )
            .map_err(|e| AudioError::Device(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        Ok(OutputStream {
            stream,
            info,
            failed,
            cpal_delay_us,
            callbacks,
        })
    }

    /// cpal's own presentation estimate for the last buffer, minus the
    /// callback's host time (diagnostics: it adds the I/O buffer to the
    /// latency this crate uses).
    pub fn cpal_playback_delay_us(&self) -> u64 {
        self.cpal_delay_us.load(Ordering::Relaxed)
    }

    /// Callbacks so far.
    pub fn callbacks(&self) -> u64 {
        self.callbacks.load(Ordering::Relaxed)
    }

    /// The stream reported an error (device removed, ...).
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    pub fn pause(&self) -> Result<()> {
        self.stream
            .pause()
            .map_err(|e| AudioError::Device(e.to_string()))
    }
}
