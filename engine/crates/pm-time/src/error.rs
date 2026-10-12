use core::fmt;

/// Errors from time arithmetic and parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// A rational with a zero denominator.
    ZeroDenominator,
    /// A rate that is not positive or exceeds one frame per tick.
    InvalidRate,
    /// A float rate that is not finite, not positive, or cannot be represented.
    InvalidFloatRate,
    /// A float time that is NaN or infinite.
    NonFiniteSeconds,
    /// The result does not fit in the target integer type.
    Overflow,
    /// A timebase that is not positive.
    InvalidTimebase,
    /// A timecode format that cannot exist (for example drop-frame at 25 fps).
    InvalidTimecodeFormat,
    /// A timecode string that does not parse, with the reason.
    InvalidTimecode(&'static str),
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeError::ZeroDenominator => f.write_str("rational with a zero denominator"),
            TimeError::InvalidRate => {
                f.write_str("rate must be positive and at most 705,600,000 per second")
            }
            TimeError::InvalidFloatRate => {
                f.write_str("frame rate must be a finite positive number")
            }
            TimeError::NonFiniteSeconds => f.write_str("time must be a finite number of seconds"),
            TimeError::Overflow => f.write_str("time value out of range"),
            TimeError::InvalidTimebase => f.write_str("timebase must be positive"),
            TimeError::InvalidTimecodeFormat => f.write_str("invalid timecode format"),
            TimeError::InvalidTimecode(reason) => write!(f, "invalid timecode: {reason}"),
        }
    }
}

impl std::error::Error for TimeError {}
