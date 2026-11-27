//! Electron text-stagger.ts: one editable timing band per stagger animator.
use pm_kernel::{Command, PropertyCommand, commands::properties::SetProperty};
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Band {
    pub layer: String,
    pub animator: String,
    pub delay: f64,
    pub duration_value: f64,
    pub stagger: f64,
    pub start: f64,
    pub duration: f64,
    pub cascade: f64,
    pub end: f64,
    pub out: bool,
    pub legacy: bool,
}
impl Band {
    /// v1 dragStagger: body moves Start; the end scales Duration and Stagger.
    pub fn commands(&self, end: bool, dt: f64, fps: f64, time: f64) -> Vec<Command> {
        let frame = |v: f64| (v * fps + 0.5).floor() / fps;
        let round = |v: f64| (v * 1000. + 0.5).floor() / 1000.;
        let mut values = Vec::new();
        if !end {
            values.push((
                "delay",
                frame(self.delay + if self.out && !self.legacy { -dt } else { dt }),
            ));
        } else {
            let span = self.duration + self.cascade;
            if span <= 0. {
                let duration = frame(dt).max(0.);
                values.push(("duration", duration));
                if self.out && !self.legacy {
                    values.push(("delay", frame(self.delay - duration)));
                }
            } else {
                let scale = frame(span + dt).max(1. / fps) / span;
                values.push(("duration", round(self.duration_value * scale)));
                values.push(("stagger", round(self.stagger * scale)));
                if self.out && !self.legacy {
                    values.push(("delay", frame(self.delay - (span * scale - span))));
                }
            }
        }
        values
            .into_iter()
            .map(|(key, value)| {
                let mut set = SetProperty::new(
                    &self.layer,
                    format!("ta.{}.{key}", self.animator),
                    value,
                    time,
                );
                set.preserve_hand_edits = false;
                PropertyCommand::Set(set).into()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn band(out: bool, legacy: bool) -> Band {
        Band {
            layer: "text".into(),
            animator: "a".into(),
            delay: 0.5,
            duration_value: 1.,
            stagger: 0.1,
            start: 0.5,
            duration: 1.,
            cascade: 0.4,
            end: 1.9,
            out,
            legacy,
        }
    }
    fn values(band: &Band, end: bool, dt: f64) -> Vec<(String, f64)> {
        band.commands(end, dt, 30., 0.)
            .into_iter()
            .map(|c| {
                let Command::Property(PropertyCommand::Set(s)) = c else {
                    panic!("property command")
                };
                (s.channel, s.value.as_f64().unwrap())
            })
            .collect()
    }
    #[test]
    fn body_moves_start_with_out_and_legacy_signs() {
        assert_eq!(values(&band(false, false), false, 0.1)[0].1, 0.6);
        assert_eq!(values(&band(true, false), false, 0.1)[0].1, 0.4);
        assert_eq!(values(&band(true, true), false, 0.1)[0].1, 0.6);
    }
    #[test]
    fn end_scales_both_timing_values_and_keeps_out_start_fixed() {
        let values = values(&band(true, false), true, 1.4);
        assert_eq!(
            values,
            vec![
                ("ta.a.duration".into(), 2.),
                ("ta.a.stagger".into(), 0.2),
                ("ta.a.delay".into(), -0.9)
            ]
        );
    }
    #[test]
    fn an_empty_band_can_be_stretched_and_negative_drags_keep_one_frame() {
        let mut b = band(false, false);
        b.duration = 0.;
        b.cascade = 0.;
        assert_eq!(values(&b, true, 0.1)[0].1, 0.1);
        assert_eq!(values(&b, true, -0.1)[0].1, 0.);
        let b = band(false, false);
        assert!(values(&b, true, -10.)[0].1 > 0.);
    }
}
