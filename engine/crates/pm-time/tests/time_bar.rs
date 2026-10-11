//! The section 2 "Time" bar, vector by vector.

use pm_time::{Rate, SUPPORTED_RATES, TimeError, TimecodeFormat};

fn df29() -> TimecodeFormat {
    TimecodeFormat::for_rate(Rate::FPS_29_97)
}

fn df59() -> TimecodeFormat {
    TimecodeFormat::for_rate(Rate::FPS_59_94)
}

#[test]
fn drop_frame_29_97_vectors() {
    let f = df29();
    assert!(f.is_drop_frame());
    assert_eq!(f.format(1799), "00:00:59;29");
    assert_eq!(f.format(1800), "00:01:00;02");
    assert_eq!(f.format(17982), "00:10:00;00");
    assert_eq!(f.frames_per_day(), 2_589_408);
    assert_eq!(f.format(2_589_408), "00:00:00;00");
    assert_eq!(f.format(2_589_407), "23:59:59;29");
    assert_eq!(f.format(-1), "23:59:59;29");
    for (frame, text) in [
        (1799, "00:00:59;29"),
        (1800, "00:01:00;02"),
        (17982, "00:10:00;00"),
    ] {
        assert_eq!(f.parse(text).unwrap(), frame);
    }
    assert_eq!(f.parse("23:59:59;29").unwrap(), 2_589_407);
}

#[test]
fn drop_frame_59_94_vectors() {
    let f = df59();
    assert!(f.is_drop_frame());
    assert_eq!(f.format(3600), "00:01:00;04");
    assert_eq!(f.format(35964), "00:10:00;00");
    assert_eq!(f.format(3599), "00:00:59;59");
    assert_eq!(f.parse("00:01:00;04").unwrap(), 3600);
    assert_eq!(f.parse("00:10:00;00").unwrap(), 35964);
    assert_eq!(f.frames_per_day(), 5_178_816);
}

#[test]
fn other_ntsc_rates_count_non_drop() {
    for (rate, base) in [
        (Rate::FPS_23_976, 24),
        (Rate::FPS_47_952, 48),
        (Rate::FPS_119_88, 120),
        (Rate::FPS_239_76, 240),
    ] {
        let f = TimecodeFormat::for_rate(rate);
        assert!(!f.is_drop_frame(), "{rate}");
        assert_eq!(f.base(), base);
        assert_eq!(f.frames_per_day(), base as i64 * 86_400);
        let minute = f.format(base as i64 * 60);
        assert!(minute.starts_with("00:01:00:0"), "{rate}: {minute}");
        assert!(TimecodeFormat::drop_frame(rate).is_err());
    }
    assert_eq!(
        TimecodeFormat::for_rate(Rate::FPS_23_976).format(1440),
        "00:01:00:00"
    );
    // 29.97 can also count non-drop.
    let ndf = TimecodeFormat::non_drop(Rate::FPS_29_97);
    assert_eq!(ndf.format(1800), "00:01:00:00");
    assert_eq!(ndf.format(17982), "00:09:59:12");
}

#[test]
fn invalid_drop_frame_strings_are_rejected() {
    let f = df29();
    for text in [
        "00:01:00;00",
        "00:01:00;01",
        "00:59:00;00",
        "23:59:00;01",
        "01:01:00;00",
    ] {
        assert!(
            matches!(f.parse(text), Err(TimeError::InvalidTimecode(_))),
            "{text}"
        );
    }
    for text in [
        "00:10:00;00",
        "00:10:00;01",
        "00:01:00;02",
        "00:01:01;00",
        "01:00:00;00",
    ] {
        assert!(f.parse(text).is_ok(), "{text}");
    }
    let f = df59();
    for text in [
        "00:01:00;00",
        "00:01:00;01",
        "00:01:00;02",
        "00:01:00;03",
        "00:09:00;03",
    ] {
        assert!(
            matches!(f.parse(text), Err(TimeError::InvalidTimecode(_))),
            "{text}"
        );
    }
    assert!(f.parse("00:01:00;04").is_ok());
    // `.` and `,` mark drop-frame too (parseDropFrameTimecode); dropped labels
    // stay invalid whatever the separator.
    let f = df29();
    assert_eq!(f.parse("00:01:00.02").unwrap(), 1800);
    assert_eq!(f.parse("00:01:00,02").unwrap(), 1800);
    assert!(f.parse("00:01:00.00").is_err());
    assert!(f.parse("00:01:00,01").is_err());
    // Malformed strings.
    let f = df29();
    for text in [
        "00:00:00:00",  // NDF separator in DF format
        "00:00:00;30",  // frame >= base
        "00:60:00;00",  // minutes
        "00:00:60;00",  // seconds
        "24:00:00;00",  // hours
        "0:00:00;00",   // width
        "00:00:00;0",   // width
        "00:00:00;000", // width
        "00-00-00;00",  // separators
        "aa:00:00;00",  // digits
        "-0:00:00;00",  // sign
        "",
    ] {
        assert!(f.parse(text).is_err(), "{text:?}");
    }
    // DF separator in an NDF format.
    assert!(
        TimecodeFormat::for_rate(Rate::FPS_25)
            .parse("00:00:00;00")
            .is_err()
    );
}

/// An independent label generator: counts labels one at a time, skipping the
/// dropped ones, and checks every frame of the day against `timecode_of`.
fn check_whole_day(format: TimecodeFormat) {
    let base = format.base();
    let drop = format.dropped_per_minute();
    let (mut h, mut m, mut s, mut ff) = (0u32, 0u32, 0u32, 0u32);
    for frame in 0..format.frames_per_day() {
        let tc = format.timecode_of(frame);
        assert_eq!(
            (tc.hours, tc.minutes, tc.seconds, tc.frames),
            (h, m, s, ff),
            "frame {frame}"
        );
        assert_eq!(format.frame_of(&tc).unwrap(), frame);
        ff += 1;
        if ff == base {
            ff = 0;
            s += 1;
            if s == 60 {
                s = 0;
                m += 1;
                if m == 60 {
                    m = 0;
                    h += 1;
                }
                if m % 10 != 0 {
                    ff = drop;
                }
            }
        }
    }
    assert_eq!((h, m, s, ff), (24, 0, 0, 0));
}

#[test]
fn every_label_of_the_day_matches_an_independent_counter() {
    check_whole_day(df29());
    check_whole_day(df59());
    check_whole_day(TimecodeFormat::non_drop(Rate::FPS_29_97));
    check_whole_day(TimecodeFormat::for_rate(Rate::FPS_23_976));
    check_whole_day(TimecodeFormat::for_rate(Rate::FPS_25));
}

/// The export rates of the Time bar: 10,000 consecutive frames per rate plus
/// 100-frame windows at N = 10^6 and 10^7 round-trip through text.
#[test]
fn export_rate_windows_round_trip() {
    let formats = [
        TimecodeFormat::for_rate(Rate::FPS_23_976),
        TimecodeFormat::for_rate(Rate::FPS_24),
        TimecodeFormat::for_rate(Rate::FPS_25),
        df29(),
        TimecodeFormat::non_drop(Rate::FPS_29_97),
        TimecodeFormat::for_rate(Rate::FPS_47_952),
        df59(),
        TimecodeFormat::for_rate(Rate::FPS_60),
        TimecodeFormat::for_rate(Rate::FPS_119_88),
        TimecodeFormat::for_rate(Rate::FPS_239_76),
    ];
    for format in formats {
        let windows = (0..10_000)
            .chain(1_000_000..1_000_100)
            .chain(10_000_000..10_000_100);
        let mut previous: Option<i64> = None;
        for frame in windows {
            let text = format.format(frame);
            let parsed = format.parse(&text).unwrap();
            assert_eq!(parsed, frame.rem_euclid(format.frames_per_day()), "{text}");
            if let Some(p) = previous
                && p + 1 == frame
            {
                assert_ne!(format.format(p), text);
            }
            previous = Some(frame);
        }
    }
}

#[test]
fn frame_math_agrees_across_supported_rates_near_zero() {
    for rate in SUPPORTED_RATES {
        let ticks = rate.exact_frame_ticks().unwrap();
        for n in -100_000..=100_000i64 {
            let start = rate.frame_start(n).unwrap();
            assert_eq!(start, n * ticks);
            assert_eq!(rate.frame_of(start), n);
            assert_eq!(rate.frame_of(start - 1), n - 1);
        }
    }
}
