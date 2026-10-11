//! Property tests for frame math, conversions, and timecode.

use pm_time::{
    Rate, Rational, SUPPORTED_RATES, TICKS_PER_SECOND, TimecodeFormat, seconds_from_ticks,
    ticks_from_seconds, ticks_from_timebase, timebase_from_ticks,
};
use proptest::prelude::*;

const FRAME_RANGE: i64 = 100_000_000; // 10^8

fn supported_rate() -> impl Strategy<Value = Rate> {
    (0..SUPPORTED_RATES.len()).prop_map(|i| SUPPORTED_RATES[i])
}

/// Arbitrary rationals up to one frame per tick, including rates that do not
/// divide the flick.
fn arbitrary_rate() -> impl Strategy<Value = Rate> {
    prop_oneof![
        (1i64..=1_000_000, 1i64..=1_000_000),
        (1i64..=TICKS_PER_SECOND, 1i64..=10),
        (1i64..=1_000, 1i64..=i32::MAX as i64),
    ]
    .prop_filter_map("rate", |(num, den)| Rate::new(num, den).ok())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 20_000, ..ProptestConfig::default() })]

    #[test]
    fn frame_of_inverts_frame_start_supported(rate in supported_rate(), n in -FRAME_RANGE..=FRAME_RANGE) {
        let start = rate.frame_start(n).unwrap();
        prop_assert_eq!(rate.frame_of(start), n);
        prop_assert_eq!(rate.frame_of(start - 1), n - 1);
        prop_assert_eq!(start, n * rate.exact_frame_ticks().unwrap());
    }

    #[test]
    fn frame_of_inverts_frame_start_arbitrary(rate in arbitrary_rate(), n in -FRAME_RANGE..=FRAME_RANGE) {
        if let Ok(start) = rate.frame_start(n) {
            prop_assert_eq!(rate.frame_of(start), n);
            prop_assert_eq!(rate.frame_of(start - 1), n - 1);
            let next = rate.frame_start(n + 1).unwrap();
            prop_assert!(next > start);
        }
    }

    #[test]
    fn frame_start_holds_far_from_zero(rate in arbitrary_rate(), n in any::<i64>()) {
        // Wherever the tick fits, the identity holds.
        if let Ok(start) = rate.frame_start(n) {
            prop_assert_eq!(rate.frame_of(start), n);
            if start > i64::MIN {
                prop_assert_eq!(rate.frame_of(start - 1), n - 1);
            }
        }
    }

    #[test]
    fn every_tick_belongs_to_the_frame_whose_range_contains_it(rate in arbitrary_rate(), t in -(1i64 << 60)..(1i64 << 60)) {
        let n = rate.frame_of(t);
        let start = rate.frame_start(n).unwrap();
        prop_assert!(start <= t);
        if let Ok(next) = rate.frame_start(n + 1) {
            prop_assert!(t < next);
        }
    }

    #[test]
    fn timebase_conversion_is_nearest(pts in -(1i64 << 40)..(1i64 << 40), den in 1i64..=10_000_000, num in 1i64..=1000) {
        let tb = Rational::new(num, den).unwrap();
        let exact_num = pts as i128 * tb.num() as i128 * TICKS_PER_SECOND as i128;
        let Ok(ticks) = ticks_from_timebase(pts, tb) else {
            // Only an out-of-range result may fail.
            prop_assert!((exact_num / tb.den() as i128).abs() >= i64::MAX as i128 - 1);
            return Ok(());
        };
        // |ticks − pts·num·TICKS/den| <= 1/2, exactly.
        let err2 = (ticks as i128 * tb.den() as i128 * 2 - exact_num * 2).abs();
        prop_assert!(err2 <= tb.den() as i128);
        // When the timebase divides the flick, the round trip is exact.
        if (TICKS_PER_SECOND as i128 * tb.num() as i128) % tb.den() as i128 == 0 {
            prop_assert_eq!(timebase_from_ticks(ticks, tb).unwrap(), pts);
        }
    }

    #[test]
    fn ticks_to_timebase_is_nearest(ticks in -(1i64 << 60)..(1i64 << 60), den in 1i64..=10_000_000, num in 1i64..=1000) {
        let tb = Rational::new(num, den).unwrap();
        let pts = timebase_from_ticks(ticks, tb).unwrap();
        // |pts − ticks·den/(num·TICKS)| <= 1/2
        let q = tb.num() as i128 * TICKS_PER_SECOND as i128;
        let err2 = (pts as i128 * q * 2 - ticks as i128 * tb.den() as i128 * 2).abs();
        prop_assert!(err2 <= q);
    }

    #[test]
    fn seconds_round_trip_where_floats_resolve_ticks(t in -(1i64 << 51)..(1i64 << 51)) {
        prop_assert_eq!(ticks_from_seconds(seconds_from_ticks(t)).unwrap(), t);
    }

    #[test]
    fn seconds_to_ticks_is_nearest(s in prop_oneof![-1.0e9f64..1.0e9, -1.0f64..1.0]) {
        prop_assume!(s.abs() >= 1e-6 || s == 0.0);
        let t = ticks_from_seconds(s).unwrap();
        // Exact check: s = m·2^e; t is nearest iff |t·2^-e − m·TICKS| <= 2^-e / 2,
        // and on a tie t is even.
        let bits = s.to_bits();
        let exp = ((bits >> 52) & 0x7ff) as i32;
        let mut m = ((bits & 0x000f_ffff_ffff_ffff) | (1 << 52)) as i128;
        if bits >> 63 == 1 { m = -m; }
        let e = exp - 1075;
        prop_assume!(s != 0.0 && e < 0 && -e <= 64);
        let shift = (-e) as u32;
        let diff = ((t as i128) << shift) - m * TICKS_PER_SECOND as i128;
        let half = 1i128 << (shift - 1);
        prop_assert!(diff.abs() < half || (diff.abs() == half && t % 2 == 0), "{} -> {}", s, t);
    }

    #[test]
    fn timecode_round_trips(frame in any::<i64>(), which in 0usize..6) {
        let format = [
            TimecodeFormat::for_rate(Rate::FPS_29_97),
            TimecodeFormat::for_rate(Rate::FPS_59_94),
            TimecodeFormat::non_drop(Rate::FPS_29_97),
            TimecodeFormat::for_rate(Rate::FPS_23_976),
            TimecodeFormat::for_rate(Rate::FPS_119_88),
            TimecodeFormat::for_rate(Rate::FPS_25),
        ][which];
        let text = format.format(frame);
        prop_assert_eq!(format.parse(&text).unwrap(), frame.rem_euclid(format.frames_per_day()));
    }

    #[test]
    fn dropped_labels_never_parse(h in 0u32..24, m in 0u32..60, ff in 0u32..4, sixty in any::<bool>()) {
        let format = if sixty { TimecodeFormat::for_rate(Rate::FPS_59_94) } else { TimecodeFormat::for_rate(Rate::FPS_29_97) };
        let drop = format.dropped_per_minute();
        let text = format!("{h:02}:{m:02}:00;{ff:02}");
        let parsed = format.parse(&text);
        if m % 10 != 0 && ff < drop {
            prop_assert!(parsed.is_err(), "{} parsed", text);
        } else {
            let frame = parsed.unwrap();
            prop_assert_eq!(format.format(frame), text);
        }
    }
}
