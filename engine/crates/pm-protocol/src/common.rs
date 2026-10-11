//! Wire primitives shared by every contract: safe integers, ticks, rationals,
//! identifiers, geometry, and content hashes.
//!
//! Every integer that crosses the protocol is within JavaScript's safe integer
//! range (|v| ≤ 2^53 − 1), so the TypeScript side reads plain `number`s and no
//! decoder needs BigInt handling. The 64-bit newtypes here enforce that range
//! when they are constructed, serialized, and deserialized.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// Largest integer a JavaScript `number` represents exactly (2^53 − 1).
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Ticks per second: the flick, 1/705,600,000 s (section 5).
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// A 64-bit value outside the JavaScript safe integer range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{value} is outside the protocol's safe integer range (|v| <= 2^53 - 1)")]
pub struct UnsafeInteger {
    pub value: i128,
}

macro_rules! safe_uint {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(TS), ts(type = "number"))]
        #[serde(try_from = "u64", into = "u64")]
        pub struct $name(u64);

        impl $name {
            pub const ZERO: Self = Self(0);
            pub const MAX: Self = Self(MAX_SAFE_INTEGER as u64);

            /// Constructs the value, rejecting anything above 2^53 − 1.
            pub const fn new(value: u64) -> Result<Self, UnsafeInteger> {
                if value > MAX_SAFE_INTEGER as u64 {
                    Err(UnsafeInteger { value: value as i128 })
                } else {
                    Ok(Self(value))
                }
            }

            pub const fn get(self) -> u64 {
                self.0
            }

            /// The next value, or `None` at the top of the safe range.
            pub const fn next(self) -> Option<Self> {
                if self.0 >= MAX_SAFE_INTEGER as u64 { None } else { Some(Self(self.0 + 1)) }
            }
        }

        impl TryFrom<u64> for $name {
            type Error = UnsafeInteger;
            fn try_from(value: u64) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> u64 {
                value.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

safe_uint!(
    /// Correlates a request with its response or error. Each peer allocates
    /// ids for the requests it sends, so ids are unique per direction on one
    /// connection, never globally.
    RequestId
);
safe_uint!(
    /// Monotonic document revision. Every applied patch batch moves it forward;
    /// 0 is the empty document before the first load.
    Revision
);
safe_uint!(
    /// An engine session: one document mirror in one engine process.
    SessionId
);
safe_uint!(
    /// Per-producer sequence number (frames, clock samples, usage reports).
    Seq
);
safe_uint!(
    /// A byte count. Safe up to 8 PiB.
    ByteSize
);
safe_uint!(
    /// A count that can exceed 32 bits (audio samples of a long export).
    Count
);
safe_uint!(
    /// A monotonic host clock instant in microseconds (`mach_absolute_time`
    /// converted to µs on macOS, `QueryPerformanceCounter` on Windows,
    /// `CLOCK_MONOTONIC` on Linux). All processes on one machine share it.
    HostMicros
);

/// A time in ticks (1/705,600,000 s) inside the protocol's safe range,
/// about ±147 days. Document key times are not limited by this: they travel
/// inside patch bodies as the document's float seconds (contract 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(type = "number"))]
#[serde(try_from = "i64", into = "i64")]
pub struct Ticks(i64);

impl Ticks {
    pub const ZERO: Self = Self(0);
    pub const MIN: Self = Self(-MAX_SAFE_INTEGER);
    pub const MAX: Self = Self(MAX_SAFE_INTEGER);

    pub const fn new(value: i64) -> Result<Self, UnsafeInteger> {
        if value < -MAX_SAFE_INTEGER || value > MAX_SAFE_INTEGER {
            Err(UnsafeInteger { value: value as i128 })
        } else {
            Ok(Self(value))
        }
    }

    /// Clamps an i128 intermediate into the safe range.
    pub fn saturating_from_i128(value: i128) -> Self {
        Self(value.clamp(-(MAX_SAFE_INTEGER as i128), MAX_SAFE_INTEGER as i128) as i64)
    }

    pub const fn get(self) -> i64 {
        self.0
    }

    pub const fn from_whole_seconds(seconds: i32) -> Self {
        Self(seconds as i64 * TICKS_PER_SECOND)
    }
}

impl TryFrom<i64> for Ticks {
    type Error = UnsafeInteger;
    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Ticks> for i64 {
    fn from(value: Ticks) -> i64 {
        value.0
    }
}

/// A half-open tick interval `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TickRange {
    pub start: Ticks,
    pub end: Ticks,
}

impl TickRange {
    pub fn is_valid(&self) -> bool {
        self.start < self.end
    }
}

/// An exact frame rate, `num / den` frames per second. Both parts positive.
/// The wire carries the exact rational; pm-time owns frame math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    pub fn is_valid(&self) -> bool {
        self.num > 0 && self.den > 0
    }

    /// True when one frame is a whole number of ticks. New rates must divide
    /// the flick (section 5, rule 1); unusual rates already in projects keep
    /// working through pm-time's ceil and Euclidean math.
    pub fn divides_flick(&self) -> bool {
        self.is_valid() && (TICKS_PER_SECOND as u128 * self.den as u128).is_multiple_of(self.num as u128)
    }
}

/// A signed playback rate, `num / den` times real time (J/K/L shuttle, reverse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaybackRate {
    pub num: i32,
    pub den: u32,
}

impl PlaybackRate {
    pub const STOPPED: Self = Self { num: 0, den: 1 };
    pub const NORMAL: Self = Self { num: 1, den: 1 };

    pub fn is_valid(&self) -> bool {
        self.den > 0
    }
}

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(TS))]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Non-empty and at most [`MAX_ID_BYTES`] UTF-8 bytes.
            pub fn is_valid(&self) -> bool {
                !self.0.is_empty() && self.0.len() <= MAX_ID_BYTES
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

/// Longest identifier the protocol accepts, in UTF-8 bytes. The document's
/// own limits are tighter (asset ids 160, registration ids 128).
pub const MAX_ID_BYTES: usize = 256;

string_id!(
    /// A document: the open project as one mirror stream.
    DocumentId
);
string_id!(
    /// A composition id from the document.
    CompositionId
);
string_id!(
    /// A layer id from the document.
    LayerId
);
string_id!(
    /// An asset id from the document.
    AssetId
);
string_id!(
    /// A registered effect, transition, or extension-layer definition id.
    DefinitionId
);
string_id!(
    /// An effect instance id on a layer.
    EffectInstanceId
);
string_id!(
    /// A render, export, generation, or Blender job.
    JobId
);
string_id!(
    /// An agent run (harness run id). Cancelling the run cancels its jobs.
    AgentRunId
);
string_id!(
    /// A project in the profile store.
    ProjectId
);
string_id!(
    /// An extension id from its manifest.
    ExtensionId
);

/// A size in whole pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn contains(&self, rect: &PixelRect) -> bool {
        rect.x as u64 + rect.width as u64 <= self.width as u64
            && rect.y as u64 + rect.height as u64 <= self.height as u64
    }
}

/// A rectangle in whole pixels, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A rectangle in composition or view units (floating point).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A 2D affine or projective transform, column-major 3×3.
pub type Mat3 = [f64; 9];
/// A 3D transform, column-major 4×4.
pub type Mat4 = [f64; 16];

pub const MAT3_IDENTITY: Mat3 = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
pub const MAT4_IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// A 32-byte content hash (BLAKE3 for media and surfaces, section 13). The
/// protocol carries the digest; hashing is the producer's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(type = "Uint8Array"))]
#[serde(transparent)]
pub struct ContentHash(#[serde(with = "serde_bytes")] pub [u8; 32]);

/// Bytes carried inline. Bulk data never travels inline: frames and surfaces
/// use shared surfaces, large payloads use cache files behind capability
/// handles. [`crate::codec::MAX_MESSAGE_BYTES`] bounds every message.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(type = "Uint8Array"))]
#[serde(transparent)]
pub struct Bytes(#[serde(with = "serde_bytes")] pub Vec<u8>);

/// An RGBA color with straight alpha, components in the working encoding
/// (E-encoded in Standard color), not clamped.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_integers_reject_values_above_2_pow_53() {
        assert!(Revision::new(MAX_SAFE_INTEGER as u64).is_ok());
        assert!(Revision::new(MAX_SAFE_INTEGER as u64 + 1).is_err());
        assert_eq!(Revision::MAX.next(), None);
        assert_eq!(Revision::ZERO.next(), Some(Revision::new(1).unwrap()));
        assert!(Ticks::new(-MAX_SAFE_INTEGER).is_ok());
        assert!(Ticks::new(-MAX_SAFE_INTEGER - 1).is_err());
        assert!(Ticks::new(i64::MAX).is_err());
        assert_eq!(Ticks::saturating_from_i128(i128::MAX), Ticks::MAX);
        assert_eq!(Ticks::saturating_from_i128(i128::MIN), Ticks::MIN);
    }

    #[test]
    fn safe_integers_are_checked_on_decode() {
        let bytes = rmp_serde::to_vec(&(MAX_SAFE_INTEGER as u64 + 1)).unwrap();
        assert!(rmp_serde::from_slice::<Revision>(&bytes).is_err());
        let bytes = rmp_serde::to_vec(&i64::MIN).unwrap();
        assert!(rmp_serde::from_slice::<Ticks>(&bytes).is_err());
        let bytes = rmp_serde::to_vec(&-5_i64).unwrap();
        assert_eq!(rmp_serde::from_slice::<Ticks>(&bytes).unwrap().get(), -5);
    }

    #[test]
    fn every_supported_rate_divides_the_flick() {
        // Section 5: the supported frame rates all divide 705,600,000.
        let rates = [
            (24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1), (48000, 1001), (48, 1),
            (50, 1), (60000, 1001), (60, 1), (90, 1), (100, 1), (120000, 1001), (120, 1),
            (240000, 1001), (240, 1),
        ];
        for (num, den) in rates {
            assert!(FrameRate::new(num, den).divides_flick(), "{num}/{den}");
        }
        // Sample rates 8 kHz to 192 kHz as "frame rates" of one sample.
        for sr in [8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000] {
            assert!(FrameRate::new(sr, 1).divides_flick(), "{sr}");
        }
        assert!(FrameRate::new(7, 1).divides_flick());
        assert!(!FrameRate::new(11, 1).divides_flick());
        assert!(!FrameRate::new(13, 1).divides_flick());
        assert!(!FrameRate::new(1_000_003, 1).divides_flick());
        assert!(!FrameRate::new(0, 1).divides_flick());
    }

    #[test]
    fn ticks_per_second_factorization() {
        // 705,600,000 = 2^9 · 3^2 · 5^5 · 7^2
        assert_eq!(TICKS_PER_SECOND, 512 * 9 * 3125 * 49);
    }
}
