//! Audio levels as the Mixer's faders and the timeline's level line draw
//! them: one console law from rail position to dB, and gains as dB.
/// The fader's console law (`FADER_MARKS`): dB and rail position, top down.
pub const MARKS: [(f64, f64); 9] = [
    (12., 1.),
    (6., 0.86),
    (0., 0.72),
    (-6., 0.58),
    (-12., 0.45),
    (-20., 0.33),
    (-30., 0.22),
    (-40., 0.13),
    (-60., 0.04),
];
pub const FLOOR_DB: f64 = -100.;
pub const DETENT_DB: f64 = 0.35;

/// Rail position (0 bottom, 1 top) of a level in dB (`faderPosition`).
pub fn fader_position(db: f64) -> f64 {
    if db.is_nan() || db <= FLOOR_DB {
        return 0.;
    }
    if db >= MARKS[0].0 {
        return 1.;
    }
    let (low_db, low_pos) = MARKS[MARKS.len() - 1];
    if db < low_db {
        return (db - FLOOR_DB) / (low_db - FLOOR_DB) * low_pos;
    }
    for pair in MARKS.windows(2) {
        let ((high_db, high_pos), (low_db, low_pos)) = (pair[0], pair[1]);
        if db >= low_db {
            return low_pos + (db - low_db) / (high_db - low_db) * (high_pos - low_pos);
        }
    }
    0.
}
/// The level at a rail position (`faderDb`).
pub fn fader_db(position: f64) -> f64 {
    if position.is_nan() || position <= 0. {
        return f64::NEG_INFINITY;
    }
    if position >= 1. {
        return MARKS[0].0;
    }
    let (low_db, low_pos) = MARKS[MARKS.len() - 1];
    if position < low_pos {
        return FLOOR_DB + position / low_pos * (low_db - FLOOR_DB);
    }
    for pair in MARKS.windows(2) {
        let ((high_db, high_pos), (low_db, low_pos)) = (pair[0], pair[1]);
        if position >= low_pos {
            return low_db + (position - low_pos) / (high_pos - low_pos) * (high_db - low_db);
        }
    }
    f64::NEG_INFINITY
}
/// Readout text (`formatDb`): `0.0`, `+2.5`, `−3.2`, `−∞`.
pub fn format_db(db: f64) -> String {
    if db.is_nan() || db <= FLOOR_DB {
        return "−∞".into();
    }
    let rounded = (db * 10.).round() / 10.;
    if rounded == 0. {
        return "0.0".into();
    }
    let text = format!("{:.1}", rounded.abs());
    if rounded > 0. {
        format!("+{text}")
    } else {
        format!("−{text}")
    }
}

/// A gain factor (1 = 0 dB) as dB.
pub fn gain_db(gain: f64) -> f64 {
    if gain > 0. { 20. * gain.log10() } else { f64::NEG_INFINITY }
}
/// The gain factor of `db`, within the 0–4 an audio clip takes (+12 dB).
pub fn db_gain(db: f64) -> f64 {
    if db.is_finite() { 10f64.powf(db / 20.).clamp(0., 4.) } else { 0. }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A clip's gain and the level line's height are one law, both ways.
    #[test]
    fn gains_and_rail_positions_round_trip() {
        for gain in [0.1, 0.5, 1., 2., 3.9] {
            let back = db_gain(fader_db(fader_position(gain_db(gain))));
            assert!((back - gain).abs() < 1e-9, "{gain} {back}");
        }
        assert_eq!(fader_position(gain_db(0.)), 0.);
        // The most a clip takes (4, +12.04 dB) is the rail's top.
        assert_eq!(fader_position(gain_db(4.)), 1.);
        assert_eq!(db_gain(fader_db(1.)), db_gain(12.));
    }
}
