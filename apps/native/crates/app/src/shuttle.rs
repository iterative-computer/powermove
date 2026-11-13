//! J/K/L while keys stay down, as Premiere, Final Cut and Resolve play it:
//! with K held, J and L step one frame, and held with it they run in slow
//! motion until either key comes up. The speeds themselves are the engine's
//! (`pm_engine_service::transport::shuttle`).
use pm_engine_service::transport::{SHUTTLE_SLOW, ShuttleKey};
use pm_protocol::{clock::TransportCommand, common::PlaybackRate};

#[derive(Default)]
pub struct Shuttle {
    stop_held: bool,
    /// J or L, while it stays down.
    travel: Option<ShuttleKey>,
    slow: bool,
}
/// What a shuttle key going down asks for.
#[derive(Debug, PartialEq)]
pub enum Press {
    /// The engine's speed steps decide (they decline while stopped unless
    /// the binding starts from a stop).
    Speeds,
    Send(TransportCommand),
    /// The auto-repeat of a key that already acted.
    Held,
}
impl Shuttle {
    pub fn press(&mut self, key: ShuttleKey) -> Press {
        let direction = if key == ShuttleKey::J { -1 } else { 1 };
        match key {
            ShuttleKey::K => Press::Speeds,
            _ if self.travel == Some(key) && self.stop_held && !self.slow => {
                self.slow = true;
                Press::Send(TransportCommand::Play {
                    rate: PlaybackRate {
                        num: direction,
                        den: SHUTTLE_SLOW,
                    },
                })
            }
            _ if self.travel == Some(key) => Press::Held,
            _ if self.stop_held => Press::Send(TransportCommand::Step { frames: direction }),
            _ => Press::Speeds,
        }
    }
    /// The press acted and its key's release will be reported.
    pub fn hold(&mut self, key: ShuttleKey) {
        match key {
            ShuttleKey::K => self.stop_held = true,
            key => self.travel = Some(key),
        }
    }
    /// A held key came up. Slow motion ends with either of its keys.
    pub fn release(&mut self, key: ShuttleKey) -> Option<TransportCommand> {
        match key {
            ShuttleKey::K => self.stop_held = false,
            key if self.travel == Some(key) => self.travel = None,
            _ => {}
        }
        std::mem::take(&mut self.slow).then_some(TransportCommand::Pause)
    }
}
/// The key a shuttle command (or the release owed for it) is about.
pub fn key(command: &str, args: &[serde_json::Value]) -> Option<ShuttleKey> {
    match (command, args.first().and_then(|a| a.as_str())) {
        ("shuttleBackward", _) | ("shuttleRelease", Some("shuttleBackward")) => Some(ShuttleKey::J),
        ("shuttleStop", _) | ("shuttleRelease", Some("shuttleStop")) => Some(ShuttleKey::K),
        ("shuttleForward", _) | ("shuttleRelease", Some("shuttleForward")) => Some(ShuttleKey::L),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ShuttleKey::*;
    use pm_engine_service::transport::{Transport, shuttle};
    use pm_shell::keyboard::{Effect, Key, Resolver};

    /// The commands a chord offers, in the order the shell tries them.
    fn offered(chord: &str, repeat: bool) -> Vec<(String, Vec<serde_json::Value>)> {
        let mut resolver = Resolver::new();
        let mut effects = resolver.submit(Key {
            token: 1,
            chord: chord.into(),
            field: false,
            repeat,
            up: false,
            compat: false,
        });
        let mut out = vec![];
        while let Some(Effect::Dispatch { command, args, .. }) = effects.into_iter().next() {
            out.push((command, args));
            effects = resolver.resolve(1, false);
        }
        out
    }
    /// A key press as the session runs it against a transport at `now` µs.
    fn press(
        s: &mut Shuttle,
        t: &mut Transport,
        key: ShuttleKey,
        from_pause: bool,
        now: i64,
    ) -> bool {
        let handled = match s.press(key) {
            Press::Speeds => match shuttle(key, t.rate(), from_pause) {
                Some(command) => {
                    t.command(command, now);
                    true
                }
                None => key == K && from_pause,
            },
            Press::Send(command) => {
                t.command(command, now);
                true
            }
            Press::Held => true,
        };
        if handled {
            s.hold(key);
        }
        handled
    }
    fn release(s: &mut Shuttle, t: &mut Transport, key: ShuttleKey, now: i64) {
        if let Some(command) = s.release(key) {
            t.command(command, now);
        }
    }
    fn rate(t: &Transport) -> (i32, u32) {
        (t.rate().num, t.rate().den)
    }

    #[test]
    fn the_shuttle_keys_are_tried_before_the_keys_other_meanings() {
        let names = |chord: &str, repeat: bool| -> Vec<String> {
            offered(chord, repeat).into_iter().map(|(c, _)| c).collect()
        };
        // Stopped, the shuttle declines and the shipped meaning runs.
        assert_eq!(names("j", false), ["shuttleBackward", "prevVisibleEvent"]);
        assert_eq!(names("k", false), ["shuttleStop", "nextVisibleEvent"]);
        assert_eq!(
            names("l", false),
            ["shuttleForward", "timeline.revealProperty:l"]
        );
        // A held J or L reaches the shuttle (slow motion with K) and nothing else.
        assert_eq!(names("j", true), ["shuttleBackward"]);
        assert_eq!(names("l", true), ["shuttleForward"]);
        assert!(names("k", true).is_empty());
        // Control-J starts in reverse from a stop.
        assert_eq!(
            offered("ctrl+j", false),
            [("shuttleBackward".to_owned(), vec![serde_json::json!(true)])]
        );
    }

    #[test]
    fn stopped_keys_decline_unless_the_binding_starts_from_a_stop() {
        let (mut s, mut t) = (Shuttle::default(), Transport::new(1000));
        for key in [J, K, L] {
            assert!(!press(&mut s, &mut t, key, false, 0), "{key:?}");
        }
        assert_eq!(rate(&t), (0, 1));
        assert!(press(&mut s, &mut t, L, true, 0));
        assert_eq!(rate(&t), (1, 1));
    }

    #[test]
    fn presses_step_the_speed_turn_around_and_k_stops() {
        let (mut s, mut t) = (Shuttle::default(), Transport::new(1000));
        let mut now = 0;
        let mut tap = |s: &mut Shuttle, t: &mut Transport, key| {
            now += 100_000;
            assert!(press(s, t, key, true, now));
            release(s, t, key, now + 10_000);
        };
        for expected in [1, 2, 4, 8, 8] {
            tap(&mut s, &mut t, L);
            assert_eq!(rate(&t), (expected, 1));
        }
        for expected in [-1, -2, -4, -8, -8] {
            tap(&mut s, &mut t, J);
            assert_eq!(rate(&t), (expected, 1));
        }
        tap(&mut s, &mut t, L);
        assert_eq!(rate(&t), (1, 1));
        tap(&mut s, &mut t, K);
        assert_eq!(rate(&t), (0, 1));
    }

    #[test]
    fn a_held_key_does_not_climb_the_speeds_by_repeating() {
        let (mut s, mut t) = (Shuttle::default(), Transport::new(1000));
        assert!(press(&mut s, &mut t, L, true, 0));
        for now in 1..6 {
            assert!(press(&mut s, &mut t, L, false, now * 50_000));
        }
        assert_eq!(rate(&t), (1, 1));
        release(&mut s, &mut t, L, 400_000);
        assert_eq!(rate(&t), (1, 1), "letting go keeps playing");
    }

    #[test]
    fn k_held_makes_j_and_l_step_a_frame_and_run_slowly_while_held() {
        let frame = 1000;
        let (mut s, mut t) = (Shuttle::default(), Transport::new(frame));
        assert!(press(&mut s, &mut t, L, true, 0));
        release(&mut s, &mut t, L, 10_000);
        // K stops the run and stays down.
        assert!(press(&mut s, &mut t, K, false, 1_000_000));
        assert_eq!(rate(&t), (0, 1));
        let at = |t: &Transport, now| t.clock.snap(t.clock.media_at(now));
        let stopped = at(&t, 1_000_000);
        // A tap of L, then of J: one frame each way, still stopped.
        assert!(press(&mut s, &mut t, L, false, 1_100_000));
        release(&mut s, &mut t, L, 1_150_000);
        assert_eq!(at(&t, 1_150_000), stopped + frame);
        assert!(press(&mut s, &mut t, J, false, 1_200_000));
        release(&mut s, &mut t, J, 1_250_000);
        assert_eq!(at(&t, 1_250_000), stopped);
        assert_eq!(rate(&t), (0, 1));
        // L held: its first repeat starts slow motion, later ones change nothing.
        assert!(press(&mut s, &mut t, L, false, 1_300_000));
        assert!(press(&mut s, &mut t, L, false, 1_800_000));
        assert_eq!(rate(&t), (1, SHUTTLE_SLOW));
        assert!(press(&mut s, &mut t, L, false, 1_850_000));
        assert_eq!(rate(&t), (1, SHUTTLE_SLOW));
        // Letting L go stops it; K is still down, so J steps back.
        release(&mut s, &mut t, L, 2_500_000);
        assert_eq!(rate(&t), (0, 1));
        assert!(press(&mut s, &mut t, J, false, 2_600_000));
        assert_eq!(rate(&t), (0, 1));
        release(&mut s, &mut t, J, 2_650_000);
        // Slow reverse ends when K comes up, too.
        assert!(press(&mut s, &mut t, J, false, 2_700_000));
        assert!(press(&mut s, &mut t, J, false, 3_200_000));
        assert_eq!(rate(&t), (-1, SHUTTLE_SLOW));
        release(&mut s, &mut t, K, 3_300_000);
        assert_eq!(rate(&t), (0, 1));
        // With K up, the still-held J is spent; a fresh L plays again.
        assert!(press(&mut s, &mut t, J, false, 3_350_000));
        assert_eq!(rate(&t), (0, 1));
        release(&mut s, &mut t, J, 3_400_000);
        assert!(press(&mut s, &mut t, L, true, 3_500_000));
        assert_eq!(rate(&t), (1, 1));
    }

    #[test]
    fn the_menu_lists_the_shuttle_with_its_keys() {
        let (_, items) = pm_shell::menu::menus()
            .into_iter()
            .find(|(title, _)| *title == "Timeline")
            .unwrap();
        for (label, command, chord) in [
            ("Shuttle Reverse", "shuttleBackward", "j"),
            ("Shuttle Stop", "shuttleStop", "k"),
            ("Shuttle Forward", "shuttleForward", "l"),
        ] {
            let item = items.iter().find(|i| i.command == command).unwrap();
            assert_eq!((item.label.as_str(), item.chord.as_str()), (label, chord));
            assert!(key(command, &[]).is_some());
        }
    }
}
