//! Transcendental functions. Every target uses the `libm` crate so native and
//! wasm answers are bit-identical. The `system-math` feature exists only as a
//! negative control: it switches to the platform's math library (Apple libm
//! on macOS) to show what the bit-identity check catches without `libm`.

#[cfg(not(feature = "system-math"))]
mod imp {
    #[inline]
    pub fn sin(x: f64) -> f64 {
        libm::sin(x)
    }
    #[inline]
    pub fn cos(x: f64) -> f64 {
        libm::cos(x)
    }
    #[inline]
    pub fn tan(x: f64) -> f64 {
        libm::tan(x)
    }
}

#[cfg(feature = "system-math")]
mod imp {
    #[inline]
    pub fn sin(x: f64) -> f64 {
        x.sin()
    }
    #[inline]
    pub fn cos(x: f64) -> f64 {
        x.cos()
    }
    #[inline]
    pub fn tan(x: f64) -> f64 {
        x.tan()
    }
}

pub use imp::{cos, sin, tan};
