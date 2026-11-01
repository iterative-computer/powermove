//! Platform-independent search/navigation state for the two pooled child windows.
#[derive(Clone, Debug)]
pub struct Choice {
    pub label: String,
    pub command: String,
}
#[derive(Clone, Debug, Default)]
pub struct Picker {
    pub choices: Vec<Choice>,
    pub query: String,
    pub selected: usize,
    pub open: bool,
}
impl Picker {
    pub fn filtered(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        self.choices
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.label.to_lowercase().contains(&q) || c.command.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }
    pub fn search(&mut self, q: String) {
        self.query = q;
        self.selected = 0;
    }
    pub fn navigate(&mut self, key: &str) -> Option<String> {
        let n = self.filtered().len();
        match key {
            "escape" | "tab" => self.open = false,
            "up" if n > 0 => self.selected = (self.selected + n - 1) % n,
            "down" if n > 0 => self.selected = (self.selected + 1) % n,
            "home" => self.selected = 0,
            "end" if n > 0 => self.selected = n - 1,
            "enter" if n > 0 => {
                self.open = false;
                return Some(self.choices[self.filtered()[self.selected]].command.clone());
            }
            _ => {}
        }
        None
    }
}
/// Damped oscillator in absolute scale units; retargeting preserves velocity.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub from: f64,
    pub velocity: f64,
    pub target: f64,
    pub mass: f64,
    pub stiffness: f64,
    pub damping: f64,
}
impl Spring {
    pub fn sample(self, seconds: f64) -> (f64, f64) {
        let alpha = self.damping / (2. * self.mass);
        let omega2 = self.stiffness / self.mass;
        let x = self.from - self.target;
        let d = omega2 - alpha * alpha;
        if d > f64::EPSILON {
            let w = d.sqrt();
            let b = (self.velocity + alpha * x) / w;
            let (s, c) = (w * seconds).sin_cos();
            let decay = (-alpha * seconds).exp();
            let y = x * c + b * s;
            let v = -alpha * y - x * w * s + b * w * c;
            (self.target + decay * y, decay * v)
        } else if d.abs() <= f64::EPSILON {
            let b = self.velocity + alpha * x;
            let decay = (-alpha * seconds).exp();
            let y = x + b * seconds;
            (self.target + decay * y, decay * (b - alpha * y))
        } else {
            let root = (-d).sqrt();
            let r1 = -alpha + root;
            let r2 = -alpha - root;
            let a = (self.velocity - r2 * x) / (r1 - r2);
            let b = x - a;
            let (e1, e2) = ((r1 * seconds).exp(), (r2 * seconds).exp());
            (self.target + a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
        }
    }
}
