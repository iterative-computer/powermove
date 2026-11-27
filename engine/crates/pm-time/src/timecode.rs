//! SMPTE timecode at the UI and IO edges.
//!
//! * Drop-frame (DF) exists only for 29.97 (`30000/1001`) and 59.94
//!   (`60000/1001`). It skips frame labels 0 and 1 (29.97) or 0 to 3 (59.94) at the
//!   start of every minute except minutes divisible by ten, and is written with a
//!   `;` before the frame field: `00:01:00;02`.
//! * Every other rate counts non-drop (NDF) with `:` separators, using the nominal
//!   base of `legacy/core/timecode.ts` (the rate rounded to the nearest integer,
//!   halves up, at least 1): 23.976 counts 24 labels per second, 119.88 counts
//!   120. The NTSC rates 29.97 and 59.94 can also count NDF when asked.
//! * Timecode wraps at 24 hours: 2,589,408 frames at 29.97 DF, `base × 86400`
//!   for NDF. Negative frames wrap from the end of the day.
//! * Parsing is strict: two digits for hours, minutes, and seconds, the format's
//!   frame-field width, the format's separator before the frame field (`;` for
//!   DF, also `.` and `,` as `parseDropFrameTimecode` accepts; `:` for NDF),
//!   fields in range, and no dropped labels (`00:01:00;00` is rejected at
//!   29.97 DF).

use core::fmt;

use crate::{Rate, TimeError};

/// How frames are labelled: the nominal base and whether labels drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimecodeFormat {
    base: u32,
    drop_frame: bool,
}

/// A timecode label within one day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timecode {
    pub hours: u32,
    pub minutes: u32,
    pub seconds: u32,
    pub frames: u32,
    pub drop_frame: bool,
    /// Frame-field width in digits (2 for bases up to 100, 3 above).
    pub frame_digits: u8,
}

impl TimecodeFormat {
    /// The default format for a rate: DF at 29.97 and 59.94, NDF otherwise.
    pub fn for_rate(rate: Rate) -> TimecodeFormat {
        let drop_frame = rate == Rate::FPS_29_97 || rate == Rate::FPS_59_94;
        TimecodeFormat {
            base: nominal_base(rate),
            drop_frame,
        }
    }

    /// Non-drop counting at the rate's nominal base.
    pub fn non_drop(rate: Rate) -> TimecodeFormat {
        TimecodeFormat {
            base: nominal_base(rate),
            drop_frame: false,
        }
    }

    /// Drop-frame counting; fails unless the rate is 29.97 or 59.94.
    pub fn drop_frame(rate: Rate) -> Result<TimecodeFormat, TimeError> {
        if rate == Rate::FPS_29_97 || rate == Rate::FPS_59_94 {
            Ok(TimecodeFormat {
                base: nominal_base(rate),
                drop_frame: true,
            })
        } else {
            Err(TimeError::InvalidTimecodeFormat)
        }
    }

    /// An explicit base; drop-frame requires base 30 or 60.
    pub fn new(base: u32, drop_frame: bool) -> Result<TimecodeFormat, TimeError> {
        if base == 0 || base > 1_000_000 || (drop_frame && base != 30 && base != 60) {
            return Err(TimeError::InvalidTimecodeFormat);
        }
        Ok(TimecodeFormat { base, drop_frame })
    }

    /// Labels per nominal second.
    pub const fn base(self) -> u32 {
        self.base
    }

    pub const fn is_drop_frame(self) -> bool {
        self.drop_frame
    }

    /// Labels dropped at the start of each non-tenth minute (0 for NDF).
    pub const fn dropped_per_minute(self) -> u32 {
        if self.drop_frame { self.base / 15 } else { 0 }
    }

    /// Frames in 24 hours.
    pub const fn frames_per_day(self) -> i64 {
        let nominal = self.base as i64 * 86_400;
        // 1,296 non-tenth minutes per day.
        nominal - self.dropped_per_minute() as i64 * 1_296
    }

    fn frame_digits(self) -> u8 {
        if self.base > 100 { 3 } else { 2 }
    }

    /// The label for frame `frame` (wrapping modulo 24 hours).
    pub fn timecode_of(self, frame: i64) -> Timecode {
        let mut f = frame.rem_euclid(self.frames_per_day());
        let base = self.base as i64;
        if self.drop_frame {
            let drop = self.dropped_per_minute() as i64;
            let per_minute = base * 60 - drop;
            let per_ten = base * 600 - drop * 9;
            let tens = f / per_ten;
            let rem = f % per_ten;
            f += drop * 9 * tens;
            if rem > drop {
                f += drop * ((rem - drop) / per_minute);
            }
        }
        let frames = (f % base) as u32;
        let total_seconds = f / base;
        Timecode {
            hours: (total_seconds / 3600) as u32,
            minutes: ((total_seconds / 60) % 60) as u32,
            seconds: (total_seconds % 60) as u32,
            frames,
            drop_frame: self.drop_frame,
            frame_digits: self.frame_digits(),
        }
    }

    /// The frame index (within the day) of a label; rejects out-of-range fields and
    /// dropped labels.
    pub fn frame_of(self, tc: &Timecode) -> Result<i64, TimeError> {
        if tc.drop_frame != self.drop_frame {
            return Err(TimeError::InvalidTimecode(
                "drop-frame mode does not match the format",
            ));
        }
        if tc.hours >= 24 {
            return Err(TimeError::InvalidTimecode("hours must be below 24"));
        }
        if tc.minutes >= 60 || tc.seconds >= 60 {
            return Err(TimeError::InvalidTimecode(
                "minutes and seconds must be below 60",
            ));
        }
        if tc.frames >= self.base {
            return Err(TimeError::InvalidTimecode(
                "frames must be below the timecode base",
            ));
        }
        let drop = self.dropped_per_minute();
        if drop > 0 && tc.seconds == 0 && !tc.minutes.is_multiple_of(10) && tc.frames < drop {
            return Err(TimeError::InvalidTimecode(
                "this frame label is dropped in drop-frame timecode",
            ));
        }
        let base = self.base as i64;
        let total_minutes = tc.hours as i64 * 60 + tc.minutes as i64;
        let nominal = ((total_minutes * 60) + tc.seconds as i64) * base + tc.frames as i64;
        Ok(nominal - drop as i64 * (total_minutes - total_minutes / 10))
    }

    /// Formats frame `frame` (wrapping modulo 24 hours).
    pub fn format(self, frame: i64) -> String {
        self.timecode_of(frame).to_string()
    }

    /// Parses a label in this format into a frame index within the day.
    pub fn parse(self, text: &str) -> Result<i64, TimeError> {
        let bytes = text.as_bytes();
        let digits = self.frame_digits() as usize;
        if bytes.len() != 9 + digits {
            return Err(TimeError::InvalidTimecode("expected HH:MM:SS:FF"));
        }
        let field = |range: core::ops::Range<usize>| -> Result<u32, TimeError> {
            let mut value = 0u32;
            for &b in &bytes[range] {
                if !b.is_ascii_digit() {
                    return Err(TimeError::InvalidTimecode("fields must be decimal digits"));
                }
                value = value * 10 + (b - b'0') as u32;
            }
            Ok(value)
        };
        if bytes[2] != b':' || bytes[5] != b':' {
            return Err(TimeError::InvalidTimecode(
                "hours, minutes, and seconds are separated by ':'",
            ));
        }
        let separator = bytes[8];
        match (self.drop_frame, separator) {
            (true, b';' | b'.' | b',') | (false, b':') => {}
            (true, b':') => {
                return Err(TimeError::InvalidTimecode(
                    "drop-frame timecode uses ';' before frames",
                ));
            }
            (false, b';') => {
                return Err(TimeError::InvalidTimecode(
                    "';' marks drop-frame timecode, which this rate does not use",
                ));
            }
            _ => {
                return Err(TimeError::InvalidTimecode(
                    "unexpected separator before frames",
                ));
            }
        }
        let tc = Timecode {
            hours: field(0..2)?,
            minutes: field(3..5)?,
            seconds: field(6..8)?,
            frames: field(9..9 + digits)?,
            drop_frame: self.drop_frame,
            frame_digits: digits as u8,
        };
        self.frame_of(&tc)
    }
}

impl fmt::Display for Timecode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let separator = if self.drop_frame { ';' } else { ':' };
        write!(
            f,
            "{:02}:{:02}:{:02}{}{:0width$}",
            self.hours,
            self.minutes,
            self.seconds,
            separator,
            self.frames,
            width = self.frame_digits as usize
        )
    }
}

/// The label base: the rate itself for integer rates, otherwise the nearest
/// integer with halves rounding up, at least 1. This is `timecodeBase` in
/// `legacy/core/timecode.ts` (`Math.max(1, Math.round(rate))`), so 23.976
/// counts 24 labels per second and 12.5 counts 13.
fn nominal_base(rate: Rate) -> u32 {
    let num = rate.num() as i128;
    let den = rate.den() as i128;
    // floor(num/den + 1/2), exact.
    let base = (2 * num + den).div_euclid(2 * den);
    base.clamp(1, u32::MAX as i128) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nominal_bases() {
        assert_eq!(TimecodeFormat::for_rate(Rate::FPS_23_976).base(), 24);
        assert_eq!(TimecodeFormat::for_rate(Rate::FPS_29_97).base(), 30);
        assert_eq!(TimecodeFormat::for_rate(Rate::FPS_119_88).base(), 120);
        assert_eq!(TimecodeFormat::for_rate(Rate::FPS_25).base(), 25);
        assert_eq!(
            TimecodeFormat::for_rate(Rate::new(25, 2).unwrap()).base(),
            13
        );
        // Math.round semantics: 7/3 fps counts 2 labels, 0.4 fps at least 1.
        assert_eq!(TimecodeFormat::for_rate(Rate::new(7, 3).unwrap()).base(), 2);
        assert_eq!(TimecodeFormat::for_rate(Rate::new(2, 5).unwrap()).base(), 1);
        assert_eq!(TimecodeFormat::for_rate(Rate::new(5, 2).unwrap()).base(), 3);
        assert!(TimecodeFormat::for_rate(Rate::FPS_29_97).is_drop_frame());
        assert!(TimecodeFormat::for_rate(Rate::FPS_59_94).is_drop_frame());
        assert!(!TimecodeFormat::for_rate(Rate::FPS_23_976).is_drop_frame());
        assert!(!TimecodeFormat::for_rate(Rate::FPS_119_88).is_drop_frame());
        assert!(TimecodeFormat::drop_frame(Rate::FPS_23_976).is_err());
        assert!(TimecodeFormat::new(25, true).is_err());
    }

    #[test]
    fn three_digit_frames_above_100() {
        let f = TimecodeFormat::for_rate(Rate::FPS_120);
        assert_eq!(f.format(119), "00:00:00:119");
        assert_eq!(f.parse("00:00:00:119").unwrap(), 119);
        assert_eq!(f.format(120), "00:00:01:000");
    }
}
