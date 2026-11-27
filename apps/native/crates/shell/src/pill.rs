//! The composer pill's split hover (w110): one quiet control of two parts,
//! `[mark] Model  Effort ⌄`, each a menu of its own. At rest it is text. The
//! part under the pointer (or whose menu is open) gets the pill, a hover
//! wash that fades in around it and glides to the other part when the
//! pointer crosses. The model part lit opens its own chevron beside its
//! name, pushing the effort along, so the two read as separate; it stays
//! open while the pointer crosses to the effort (which therefore never moves
//! from under the pointer) and closes when the pill goes. Entering the
//! effort part straight moves nothing.
//! Panels mark the control with `metadata.pill`: `"group"` on the row,
//! `"part"` on each target, `"reveal"` on what shows only with the pill.

/// What a node is in a split pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The row holding the parts: the pill is drawn behind them.
    Group,
    /// One target (a button): the pill stands on it, it draws no wash.
    Part,
    /// Shown only with the pill (the model's chevron): its width and
    /// leading margin open up with the pill's amount.
    Reveal,
}
impl Role {
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "group" => Self::Group,
            "part" => Self::Part,
            "reveal" => Self::Reveal,
            _ => return None,
        })
    }
}

/// One part as the group sees it this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part<K> {
    pub key: K,
    /// Its menu is open.
    pub open: bool,
    pub disabled: bool,
}

/// The part the pill stands on: the one whose menu is open, else the one
/// under the pointer; none for a disabled part.
pub fn lit<K: Copy + PartialEq>(parts: &[Part<K>], hovered: Option<K>) -> Option<K> {
    let usable = parts.iter().filter(|p| !p.disabled);
    usable
        .clone()
        .find(|p| p.open)
        .or_else(|| usable.clone().find(|p| Some(p.key) == hovered))
        .map(|p| p.key)
}

/// Whether a reveal inside the part `holder` shows: from the moment its
/// part is lit, for as long as any part of the pill is (`was`: it showed).
pub fn revealed<K: Copy + PartialEq>(lit: Option<K>, holder: Option<K>, was: bool) -> bool {
    lit.is_some() && (lit == holder || was)
}

/// A revealed node's width and leading margin at `amount` (0 hidden … 1
/// shown) of its full `width` and `margin`. A spring may overshoot: the
/// node never grows past its own size or below nothing.
pub fn reveal(width: f32, margin: f32, amount: f32) -> (f32, f32) {
    let a = amount.clamp(0., 1.);
    (width * a, margin * a)
}

/// How far a turning chevron is turned at `amount` (0 pointing down … 1
/// pointing up), in degrees, quantised so the drawn steps stay cached.
pub fn turn_degrees(amount: f32) -> f32 {
    const STEP: f32 = 6.;
    ((amount * 180.) / STEP).round() * STEP
}

#[cfg(test)]
mod tests {
    use super::*;
    fn part(key: u32, open: bool, disabled: bool) -> Part<u32> {
        Part {
            key,
            open,
            disabled,
        }
    }
    #[test]
    fn the_pill_stands_on_the_open_part_else_the_hovered_one() {
        let parts = [part(1, false, false), part(2, false, false)];
        assert_eq!(lit(&parts, None), None);
        assert_eq!(lit(&parts, Some(1)), Some(1));
        assert_eq!(lit(&parts, Some(2)), Some(2));
        // A part the pointer is not over (a node elsewhere) lights nothing.
        assert_eq!(lit(&parts, Some(9)), None);
        // An open menu keeps its part lit whatever the pointer is over.
        let open = [part(1, false, false), part(2, true, false)];
        assert_eq!(lit(&open, Some(1)), Some(2));
        // A locked pill (a run working) shows none.
        let locked = [part(1, false, true), part(2, false, true)];
        assert_eq!(lit(&locked, Some(1)), None);
    }
    #[test]
    fn the_models_chevron_opens_from_its_part_and_holds_while_the_pill_shows() {
        // Resting: hidden. The pointer straight onto the effort: still hidden
        // (nothing moves under the pointer).
        assert!(!revealed(None, Some(1), false));
        assert!(!revealed(Some(2), Some(1), false));
        // Onto the model: shown; across to the effort: held.
        assert!(revealed(Some(1), Some(1), false));
        assert!(revealed(Some(2), Some(1), true));
        // Off the pill: gone.
        assert!(!revealed(None, Some(1), true));
    }
    #[test]
    fn a_reveal_opens_with_the_pill_and_never_overshoots() {
        assert_eq!(reveal(15., 4., 0.), (0., 0.));
        assert_eq!(reveal(15., 4., 1.), (15., 4.));
        assert_eq!(reveal(15., 4., 0.5), (7.5, 2.));
        assert_eq!(reveal(15., 4., 1.2), (15., 4.));
        assert_eq!(reveal(15., 4., -0.1), (0., 0.));
    }
    #[test]
    fn a_chevron_turns_half_a_turn_in_cached_steps() {
        assert_eq!(turn_degrees(0.), 0.);
        assert_eq!(turn_degrees(1.), 180.);
        assert_eq!(turn_degrees(0.5), 90.);
        // An overshooting spring turns a little past and comes back.
        assert_eq!(turn_degrees(1.1), 198.);
        assert_eq!(turn_degrees(0.51) % 6., 0.);
    }
}
