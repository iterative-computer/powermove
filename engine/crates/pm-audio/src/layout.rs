//! Channel layouts and the matrices that fold a voice into a bus.
//!
//! A voice keeps its source's channels up to the strip it feeds; the
//! strips and the master use the mix layout (stereo today, plan section 16:
//! "stereo as today"). [`matrix`] maps one to the other.
//!
//! The cases the legacy engine meets keep its numbers exactly: Web Audio's
//! speaker rules (mono copies to both sides; quad folds as ½(L + Ls);
//! 5.1 folds as L + √½·(C + Ls), dropping LFE; stereo to mono is ½(L + R)).
//! Layouts Web Audio does not know (it mixes them "discretely", keeping the
//! first channels and dropping the rest) fold by ITU-R BS.775 instead:
//! centers at −3 dB to both sides, surround and back channels at −3 dB to
//! their side, back center at −6 dB to both, LFE dropped. That is a
//! deliberate improvement: a 7.1 file keeps its center and surrounds.
//!
//! Streams whose channels have no speaker meaning (ambisonics, unknown
//! orders) fold as Web Audio folds every stream, by channel count: 1, 2,
//! 4, and 6 channels by its speaker rules, other counts "discretely" (the
//! first two channels are left and right). A channel without a meaning
//! inside a named layout folds as a center.

use pm_media::audio::Speaker;

/// √½, the −3 dB fold coefficient.
pub const FOLD: f32 = std::f32::consts::FRAC_1_SQRT_2;

pub fn stereo() -> Vec<Speaker> {
    vec![Speaker::FrontLeft, Speaker::FrontRight]
}

pub fn mono() -> Vec<Speaker> {
    vec![Speaker::FrontCenter]
}

/// Which side a speaker sits on: −1 left, 0 center, 1 right.
fn side(s: Speaker) -> i8 {
    use Speaker::*;
    match s {
        FrontLeft | BackLeft | FrontLeftOfCenter | SideLeft | TopFrontLeft | TopBackLeft
        | StereoLeft | WideLeft | SurroundDirectLeft | TopSideLeft | BottomFrontLeft
        | SideSurroundLeft | TopSurroundLeft => -1,
        FrontRight | BackRight | FrontRightOfCenter | SideRight | TopFrontRight | TopBackRight
        | StereoRight | WideRight | SurroundDirectRight | TopSideRight | BottomFrontRight
        | SideSurroundRight | TopSurroundRight => 1,
        _ => 0,
    }
}

/// Gain of a speaker folded onto its own side (or both sides, for centers)
/// of a stereo pair, by ITU-R BS.775.
fn stereo_fold(s: Speaker) -> (f32, f32) {
    use Speaker::*;
    match s {
        FrontLeft | StereoLeft => (1.0, 0.0),
        FrontRight | StereoRight => (0.0, 1.0),
        FrontCenter | TopCenter | TopFrontCenter | BottomFrontCenter => (FOLD, FOLD),
        BackCenter | TopBackCenter => (0.5, 0.5),
        LowFrequency | LowFrequency2 => (0.0, 0.0),
        Other(_) => (FOLD, FOLD),
        s => match side(s) {
            -1 => (FOLD, 0.0),
            1 => (0.0, FOLD),
            _ => (FOLD, FOLD),
        },
    }
}

/// `to.len()` rows of `from.len()` gains: `out[r] = Σ_c m[r][c] · in[c]`.
pub fn matrix(from: &[Speaker], to: &[Speaker]) -> Vec<Vec<f32>> {
    use Speaker::*;
    let mut m = vec![vec![0.0f32; from.len()]; to.len()];
    if from.is_empty() || to.is_empty() {
        return m;
    }
    if from == to {
        for (i, row) in m.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        return m;
    }
    let is = |l: &[Speaker], want: &[Speaker]| l == want;
    let st = [FrontLeft, FrontRight];
    let quad = [FrontLeft, FrontRight, BackLeft, BackRight];
    let five_one_side = [
        FrontLeft,
        FrontRight,
        FrontCenter,
        LowFrequency,
        SideLeft,
        SideRight,
    ];
    let five_one_back = [
        FrontLeft,
        FrontRight,
        FrontCenter,
        LowFrequency,
        BackLeft,
        BackRight,
    ];
    let single = from.len() == 1;
    // Channels without meaning: Web Audio's rules by count.
    if from.len() > 1 && from.iter().all(|s| matches!(s, Other(_))) {
        let named: Option<Vec<Speaker>> = match from.len() {
            2 => Some(st.to_vec()),
            4 => Some(quad.to_vec()),
            6 => Some(five_one_back.to_vec()),
            _ => None,
        };
        return match named {
            Some(named) => matrix(&named, to),
            None => {
                // Discrete: the first two channels are left and right.
                let mut d = matrix(&st, to);
                for row in &mut d {
                    row.resize(from.len(), 0.0);
                }
                d
            }
        };
    }
    if is(to, &st) {
        // Web Audio's rules where it has them.
        if single {
            m[0][0] = 1.0;
            m[1][0] = 1.0;
            return m;
        }
        if is(from, &quad) {
            m[0][0] = 0.5;
            m[0][2] = 0.5;
            m[1][1] = 0.5;
            m[1][3] = 0.5;
            return m;
        }
        if is(from, &five_one_side) || is(from, &five_one_back) {
            m[0][0] = 1.0;
            m[0][2] = FOLD;
            m[0][4] = FOLD;
            m[1][1] = 1.0;
            m[1][2] = FOLD;
            m[1][5] = FOLD;
            return m;
        }
        for (c, &s) in from.iter().enumerate() {
            let (l, r) = stereo_fold(s);
            m[0][c] = l;
            m[1][c] = r;
        }
        return m;
    }
    if is(to, &[FrontCenter]) {
        if single {
            m[0][0] = 1.0;
        } else if is(from, &st) {
            m[0] = vec![0.5, 0.5];
        } else if is(from, &quad) {
            m[0] = vec![0.25; 4];
        } else if is(from, &five_one_side) || is(from, &five_one_back) {
            m[0] = vec![FOLD, FOLD, 1.0, 0.0, 0.5, 0.5];
        } else {
            for (c, &s) in from.iter().enumerate() {
                let (l, r) = stereo_fold(s);
                m[0][c] = 0.5 * (l + r);
            }
        }
        return m;
    }
    // A general bus: same-named speakers pass; a mono source feeds the
    // center (or both fronts); anything else folds to the bus's left and
    // right fronts by side, or to its center.
    let find = |s: Speaker| to.iter().position(|&t| t == s);
    let (left, right, center) = (find(FrontLeft), find(FrontRight), find(FrontCenter));
    for (c, &s) in from.iter().enumerate() {
        if let Some(r) = find(s) {
            m[r][c] = 1.0;
            continue;
        }
        if single {
            match (center, left, right) {
                (Some(cc), _, _) => m[cc][c] = 1.0,
                (None, Some(l), Some(r)) => {
                    m[l][c] = 1.0;
                    m[r][c] = 1.0;
                }
                _ => m[0][c] = 1.0,
            }
            continue;
        }
        let (l, r) = stereo_fold(s);
        match (left, right, center) {
            (Some(li), Some(ri), _) => {
                m[li][c] += l;
                m[ri][c] += r;
            }
            (_, _, Some(ci)) => m[ci][c] += 0.5 * (l + r),
            _ => {}
        }
    }
    m
}

/// Applies a matrix to one frame (`input` has `from` channels).
#[inline]
pub fn apply(m: &[Vec<f32>], input: &[f32], output: &mut [f32]) {
    for (o, row) in output.iter_mut().zip(m) {
        let mut acc = 0.0f32;
        for (g, x) in row.iter().zip(input) {
            acc += g * x;
        }
        *o = acc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Speaker::*;

    #[test]
    fn web_audio_rules() {
        assert_eq!(matrix(&mono(), &stereo()), vec![vec![1.0], vec![1.0]]);
        assert_eq!(
            matrix(&stereo(), &stereo()),
            vec![vec![1.0, 0.0], vec![0.0, 1.0]]
        );
        assert_eq!(matrix(&stereo(), &mono()), vec![vec![0.5, 0.5]]);
        let quad = [FrontLeft, FrontRight, BackLeft, BackRight];
        assert_eq!(
            matrix(&quad, &stereo()),
            vec![vec![0.5, 0.0, 0.5, 0.0], vec![0.0, 0.5, 0.0, 0.5]]
        );
        let s51 = [
            FrontLeft,
            FrontRight,
            FrontCenter,
            LowFrequency,
            SideLeft,
            SideRight,
        ];
        assert_eq!(
            matrix(&s51, &stereo()),
            vec![
                vec![1.0, 0.0, FOLD, 0.0, FOLD, 0.0],
                vec![0.0, 1.0, FOLD, 0.0, 0.0, FOLD]
            ]
        );
    }

    #[test]
    fn seven_one_keeps_center_and_surrounds() {
        let s71 = [
            FrontLeft,
            FrontRight,
            FrontCenter,
            LowFrequency,
            BackLeft,
            BackRight,
            SideLeft,
            SideRight,
        ];
        let m = matrix(&s71, &stereo());
        assert_eq!(m[0], vec![1.0, 0.0, FOLD, 0.0, FOLD, 0.0, FOLD, 0.0]);
        assert_eq!(m[1], vec![0.0, 1.0, FOLD, 0.0, 0.0, FOLD, 0.0, FOLD]);
    }

    #[test]
    fn layouts_pass_through_a_wider_bus() {
        let s51 = vec![
            FrontLeft,
            FrontRight,
            FrontCenter,
            LowFrequency,
            SideLeft,
            SideRight,
        ];
        let m = matrix(&stereo(), &s51);
        assert_eq!(m[0], vec![1.0, 0.0]);
        assert_eq!(m[1], vec![0.0, 1.0]);
        assert!(m[2..].iter().all(|r| r.iter().all(|&g| g == 0.0)));
        let m = matrix(&mono(), &s51);
        assert_eq!(m[2], vec![1.0]);
        let m = matrix(&s51, &s51);
        for (i, row) in m.iter().enumerate() {
            assert_eq!(row[i], 1.0);
        }
    }
}
