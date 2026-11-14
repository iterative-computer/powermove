//! What Play plays: the work area, round and round while Playback ▸ Loop
//! Playback is on (the shipped app always looped it), once to its end when
//! off. The loop belongs to a run: a stopped playhead steps and seeks
//! through the work area's ends like anywhere else.
use crate::session::Controller;
use gpui::Context;
use pm_protocol::clock::TransportCommand;
use pm_protocol::common::{PlaybackRate, TICKS_PER_SECOND, TickRange, Ticks};
use std::path::{Path, PathBuf};

/// A run started at `from` seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Run {
    /// The seconds the run repeats.
    pub repeat: Option<[f64; 2]>,
    /// Where a run that does not repeat stops, short of the composition's end.
    pub stop: Option<f64>,
}
/// `work` is the composition's stored work area; an empty or missing one is
/// the whole composition, as the shipped app read it.
pub fn run(work: Option<[f64; 2]>, duration: f64, from: f64, looping: bool) -> Run {
    let [start, end] = work
        .map(|[a, b]| [a.clamp(0., duration), b.clamp(0., duration)])
        .filter(|[a, b]| b > a)
        .unwrap_or([0., duration]);
    if looping {
        return Run {
            repeat: Some([start, end]),
            stop: None,
        };
    }
    // Started at or past the work area's end, it plays on to the end.
    let frame = 1e-6;
    Run {
        repeat: None,
        stop: (from < end - frame && end < duration - frame).then_some(end),
    }
}
/// The open composition's stored work area in `p`.
pub fn work_area(p: &pm_kernel::Published) -> Option<[f64; 2]> {
    let root = p.snapshot.root()?;
    let comp = pm_kernel::model::comp_node(root, Some(&p.snapshot.render_comp()))?;
    let work = comp.get("work")?;
    let number = |i| work.at(i).and_then(|n| n.to_value().as_f64());
    Some([number(0)?, number(1)?])
}
fn ticks(seconds: f64) -> Ticks {
    Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).unwrap_or(Ticks::ZERO)
}
/// The engine's loop for a run.
pub fn loop_command(run: &Run) -> TransportCommand {
    TransportCommand::SetLoop {
        range: run.repeat.map(|[start, end]| TickRange {
            start: ticks(start),
            end: ticks(end),
        }),
    }
}

fn prefs_path(profile: &Path) -> PathBuf {
    profile.join("Viewer").join("playback.json")
}
/// The person's saved choice (`None`: never chosen, Loop is on).
pub fn load(profile: &Path) -> Option<bool> {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(prefs_path(profile)).ok()?).ok()?;
    v["loop"].as_bool()
}
pub fn save(profile: &Path, looping: bool) -> std::io::Result<()> {
    let path = prefs_path(profile);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::json!({ "loop": looping }).to_string())
}

/// Loop Playback and the run it shaped.
pub struct Playback {
    pub looping: bool,
    /// A loop is set in the engine for the run in progress.
    armed: bool,
    /// Where the run in progress stops (Loop off).
    pub stop: Option<f64>,
}
impl Default for Playback {
    fn default() -> Self {
        Self {
            looping: true,
            armed: false,
            stop: None,
        }
    }
}

impl Controller {
    /// Reads the saved choice off the main thread and applies it.
    pub fn load_playback(&mut self, cx: &mut Context<Self>) {
        let profile = self.profile.clone();
        let read = cx
            .background_executor()
            .spawn(async move { load(&profile) });
        cx.spawn(async move |this, cx| {
            if let Some(looping) = read.await {
                let _ = this.update(cx, |c, cx| {
                    c.playback.looping = looping;
                    cx.notify();
                });
            }
        })
        .detach();
    }
    pub fn toggle_loop(&mut self, cx: &mut Context<Self>) {
        self.playback.looping = !self.playback.looping;
        let (profile, looping) = (self.profile.clone(), self.playback.looping);
        std::thread::spawn(move || {
            let _ = save(&profile, looping);
        });
        // A run in progress takes the new choice from where it is.
        if self.running() {
            self.shape_run();
        }
        // The viewer bar's Loop shows the choice.
        self.update_builtins(cx);
        cx.notify();
    }
    /// A run is asked for or in progress. The clock reads stopped for a
    /// moment while a new run waits for the audio device; this does not.
    pub fn running(&self) -> bool {
        self.viewer.as_ref().is_some_and(|v| v.running())
    }
    /// The open composition's stored work area.
    pub(crate) fn work_area(&self) -> Option<[f64; 2]> {
        work_area(self.published.as_deref()?)
    }
    /// Before a run starts (or as Loop changes during one): its loop in the
    /// engine, and where it stops.
    pub fn shape_run(&mut self) {
        let run = run(
            self.work_area(),
            self.duration(),
            self.time(),
            self.playback.looping,
        );
        if let Some(v) = &self.viewer {
            let _ = v.command(loop_command(&run));
        }
        self.playback.armed = run.repeat.is_some();
        self.playback.stop = run.stop;
    }
    /// The run is over: a stopped playhead never wraps.
    pub fn end_run(&mut self) {
        self.playback.stop = None;
        if std::mem::take(&mut self.playback.armed)
            && let Some(v) = &self.viewer
        {
            let _ = v.command(TransportCommand::SetLoop { range: None });
        }
    }
    /// A transport command from a person, with the run shaped around it.
    pub fn transport(&mut self, command: TransportCommand) -> bool {
        match command {
            TransportCommand::Play { rate } if rate != PlaybackRate::STOPPED => {
                if !self.running() {
                    self.shape_run();
                }
            }
            TransportCommand::Pause | TransportCommand::Step { .. } => self.end_run(),
            _ => {}
        }
        let Some(v) = &self.viewer else { return false };
        // Measured through the frame it presents.
        match v.command_at(command, Self::event_time()) {
            Ok(()) => true,
            Err(e) => {
                self.message = e.to_string();
                false
            }
        }
    }
    /// Each display tick: a run stops at the work area's end when Loop is
    /// off, and one that ended by itself gives up its loop.
    pub fn watch_run(&mut self, clock: &pm_protocol::clock::ClockSample) {
        if !self.running() {
            if self.playback.armed || self.playback.stop.is_some() {
                self.end_run();
            }
            return;
        }
        let seconds = clock.media_time.get() as f64 / TICKS_PER_SECOND as f64;
        if let Some(stop) = self.playback.stop
            && clock.rate.num > 0
            && seconds >= stop
        {
            self.transport(TransportCommand::Pause);
            self.seek(stop);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_engine_service::transport::Transport;

    #[test]
    fn play_repeats_the_work_area_while_loop_is_on() {
        assert_eq!(
            run(Some([1., 3.]), 10., 0., true).repeat,
            Some([1., 3.]),
            "from before it, the run plays in and stays"
        );
        assert_eq!(run(Some([1., 3.]), 10., 5., true).repeat, Some([1., 3.]));
        // No work area, an empty one, one past the end: the whole composition.
        for work in [None, Some([4., 4.]), Some([6., 2.])] {
            assert_eq!(run(work, 10., 0., true).repeat, Some([0., 10.]), "{work:?}");
        }
        assert_eq!(run(Some([8., 30.]), 10., 0., true).repeat, Some([8., 10.]));
    }

    #[test]
    fn with_loop_off_play_stops_at_the_work_areas_end() {
        let once = run(Some([1., 3.]), 10., 1., false);
        assert_eq!((once.repeat, once.stop), (None, Some(3.)));
        assert_eq!(run(Some([1., 3.]), 10., 0., false).stop, Some(3.));
        // From its end or later the run goes on to the composition's end,
        // which is the engine's own stop.
        assert_eq!(run(Some([1., 3.]), 10., 3., false).stop, None);
        assert_eq!(run(Some([1., 3.]), 10., 7., false).stop, None);
        assert_eq!(run(None, 10., 2., false).stop, None);
    }

    /// The engine's transport under the commands the session sends for Space.
    #[test]
    fn a_looped_run_wraps_inside_the_work_area_and_a_stopped_step_does_not() {
        let frame = TICKS_PER_SECOND / 30;
        let mut t = Transport::new(frame);
        t.clock.set_end(Some(10 * TICKS_PER_SECOND));
        let seek = |t: &mut Transport, seconds: i32, now| {
            t.command(
                TransportCommand::Seek {
                    time: Ticks::from_whole_seconds(seconds),
                    mode: pm_protocol::clock::SeekMode::Exact,
                },
                now,
            )
        };
        let play = TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        };
        // Space at 1 s with a 1–3 s work area and Loop on.
        seek(&mut t, 1, 0);
        t.command(loop_command(&run(Some([1., 3.]), 10., 1., true)), 0);
        t.command(play, 0);
        let start = pm_engine_service::transport::PREROLL_US;
        let at = |t: &Transport, after: i64| t.clock.media_at(start + after);
        assert_eq!(at(&t, 1_500_000), 5 * TICKS_PER_SECOND / 2);
        // 2.5 s in, it is half a second into its second pass.
        assert_eq!(at(&t, 2_500_000), 3 * TICKS_PER_SECOND / 2);
        t.poll(start + 2_500_000);
        assert_ne!(t.rate(), PlaybackRate::STOPPED, "it keeps going");
        // Space again: paused, and the loop goes with the run.
        t.command(TransportCommand::Pause, start + 2_500_000);
        t.command(TransportCommand::SetLoop { range: None }, start + 2_500_000);
        // Stepping from the work area's last frame goes on to 3 s, not back.
        seek(&mut t, 3, start + 3_000_000);
        t.command(TransportCommand::Step { frames: -1 }, start + 3_000_000);
        t.command(TransportCommand::Step { frames: 1 }, start + 3_000_000);
        assert_eq!(t.clock.media_at(start + 3_000_000), 3 * TICKS_PER_SECOND);
        // Loop off: the same Space has no loop, so the run passes 3 s (the
        // session stops it there, `Controller::watch_run`).
        seek(&mut t, 1, start + 4_000_000);
        t.command(
            loop_command(&run(Some([1., 3.]), 10., 1., false)),
            start + 4_000_000,
        );
        t.command(play, start + 4_000_000);
        let again = start + 4_000_000 + pm_engine_service::transport::PREROLL_US;
        assert_eq!(
            t.clock.media_at(again + 2_500_000),
            7 * TICKS_PER_SECOND / 2
        );
    }

    /// A new project's duration changed from 10 s to 90 s: the run the next
    /// Space shapes reads the composition as it is now, not as it was bound.
    #[test]
    fn a_run_reads_the_duration_the_composition_has_now() {
        use pm_kernel::{Call, Caller, KernelActor, KernelConfig, OpenSource, Request};
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let project = pm_model::json::parse(&crate::v4_view_tests::project_json(1)).unwrap();
        kernel
            .open("p", OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        let shape = |kernel: &pm_kernel::KernelHandle| {
            let p = kernel.published("p").unwrap();
            let duration = crate::session::composition_duration(&p);
            (duration, run(work_area(&p), duration, 12., true).repeat)
        };
        assert_eq!(shape(&kernel), (10., Some([0., 10.])));
        let revision = kernel.published("p").unwrap().revision();
        let request = Request::new(
            Caller::native(),
            Call::Transact {
                label: Some("Duration".into()),
                expected_revision: Some(revision),
                body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Comp(
                    pm_kernel::CompCommand::Set {
                        patch: Box::new(pm_kernel::commands::comps::CompPatch {
                            duration: Some(90.),
                            ..Default::default()
                        }),
                    },
                )]),
            },
        );
        kernel.call("p", request).wait().unwrap().unwrap();
        assert_eq!(shape(&kernel), (90., Some([0., 90.])));
    }

    #[test]
    fn the_loop_choice_is_kept_in_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), None, "never chosen: Loop is on");
        assert!(Playback::default().looping);
        save(dir.path(), false).unwrap();
        assert_eq!(load(dir.path()), Some(false));
        save(dir.path(), true).unwrap();
        assert_eq!(load(dir.path()), Some(true));
    }

    #[test]
    fn the_playback_menu_has_loop_playback() {
        let (_, items) = pm_shell::menu::menus()
            .into_iter()
            .find(|(title, _)| *title == "Playback")
            .expect("a Playback menu");
        let item = items.iter().find(|i| i.command == "toggleLoop").unwrap();
        assert_eq!(item.label, "Loop Playback");
        // Its checkmark and its key equivalents are AppKit's, so the menu
        // is not one of the display-only ones.
        assert!(!pm_shell::menu::DISPLAY_ONLY.contains(&"Playback"));
    }
}
