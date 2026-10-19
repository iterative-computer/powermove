//! First-use ANGLE parity decisions. No source or pixels are persisted.
//!
//! Decisions are immutable, content-addressed files, so independent viewer
//! and job processes cannot overwrite each other's pins. A damaged entry is
//! unknown and is probed again. Unknown programs always present ANGLE's frame.
use std::collections::HashMap;
use std::path::PathBuf;

use crate::shader_layers::FallbackPixels;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Unknown,
    NativeVerified,
    Runner,
}

#[derive(Debug, Default)]
pub struct Parity {
    environment: Option<String>,
    directory: Option<PathBuf>,
    decisions: HashMap<String, Verdict>,
}

impl Parity {
    /// `environment` includes the engine, GPU/driver, OS and browser versions.
    /// None disables probing (standalone renderers without a browser reference).
    pub fn configure(&mut self, environment: Option<String>, directory: Option<PathBuf>) {
        self.environment = environment;
        self.directory = directory;
        self.decisions.clear();
    }

    pub fn environment(&self) -> Option<&str> {
        self.environment.as_deref()
    }
    pub fn directory(&self) -> Option<&std::path::Path> {
        self.directory.as_deref()
    }

    pub fn enabled(&self) -> bool {
        self.environment.is_some()
    }

    fn key(&self, source: &str) -> String {
        let mut hash = blake3::Hasher::new();
        hash.update(b"pm ANGLE parity 1\0");
        hash.update(self.environment.as_deref().unwrap_or_default().as_bytes());
        hash.update(&[0]);
        hash.update(pm_glsl::VERTEX.as_bytes());
        hash.update(&[0]);
        hash.update(source.as_bytes());
        hash.finalize().to_hex().to_string()
    }

    pub fn verdict(&mut self, source: &str) -> Verdict {
        if !self.enabled() {
            return Verdict::NativeVerified;
        }
        let key = self.key(source);
        *self.decisions.entry(key.clone()).or_insert_with(|| {
            self.directory
                .as_ref()
                .map(|dir| {
                    if std::fs::read(dir.join(format!("{key}.runner")))
                        .ok()
                        .as_deref()
                        == Some(b"runner\n")
                    {
                        Verdict::Runner
                    } else if std::fs::read(dir.join(format!("{key}.native")))
                        .ok()
                        .as_deref()
                        == Some(b"native-verified\n")
                    {
                        Verdict::NativeVerified
                    } else {
                        Verdict::Unknown
                    }
                })
                .unwrap_or(Verdict::Unknown)
        })
    }

    /// Diagnostic corpus sweeps exercise every parameter set. Re-probe a
    /// verified program there; a pin is monotonic. Production uses first use.
    pub fn recheck(&mut self, source: &str) {
        if self.verdict(source) == Verdict::NativeVerified {
            let key = self.key(source);
            self.decisions.insert(key, Verdict::Unknown);
        }
    }

    /// Any channel over one code value pins the whole program. This is an
    /// absolute bound, also for HDR; non-finite disagreements pin too.
    pub fn judge(
        &mut self,
        source: &str,
        native: &FallbackPixels,
        angle: &FallbackPixels,
    ) -> Verdict {
        let verdict = if self.verdict(source) == Verdict::Runner {
            Verdict::Runner
        } else if matches(native, angle) {
            Verdict::NativeVerified
        } else {
            Verdict::Runner
        };
        let key = self.key(source);
        self.decisions.insert(key.clone(), verdict);
        if let Some(dir) = &self.directory {
            let bytes: &[u8] = if verdict == Verdict::Runner {
                b"runner\n"
            } else {
                b"native-verified\n"
            };
            // Atomic markers with exact payloads: an interrupted write
            // cannot be accepted as native verification.
            let write = || -> std::io::Result<()> {
                std::fs::create_dir_all(dir)?;
                // Separate markers: a simultaneous native verification can
                // never overwrite an ANGLE pin from another process.
                static WRITE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let suffix = if verdict == Verdict::Runner {
                    "runner"
                } else {
                    "native"
                };
                let serial = WRITE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let temp = dir.join(format!(".{key}.{}.{serial}", std::process::id()));
                std::fs::write(&temp, bytes)?;
                let result = std::fs::rename(&temp, dir.join(format!("{key}.{suffix}")));
                if result.is_err() {
                    let _ = std::fs::remove_file(temp);
                }
                result
            };
            if let Err(error) = write() {
                eprintln!("shader parity decision could not be persisted: {error}");
            }
        }
        verdict
    }
}

pub fn matches(native: &FallbackPixels, angle: &FallbackPixels) -> bool {
    native.width == angle.width
        && native.height == angle.height
        && native.rgba16f.len() == angle.rgba16f.len()
        && native.rgba16f.len() == native.width as usize * native.height as usize * 4
        && native.rgba16f.iter().zip(&angle.rgba16f).all(|(n, a)| {
            if n == a {
                return true;
            }
            let n = pm_gpu::readback::half_to_f32(*n);
            let a = pm_gpu::readback::half_to_f32(*a);
            n.is_finite() && a.is_finite() && (n - a).abs() * 255.0 <= 1.0
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixels(v: f32) -> FallbackPixels {
        FallbackPixels {
            width: 1,
            height: 1,
            rgba16f: vec![pm_gpu::readback::f32_to_half(v); 4],
        }
    }

    #[test]
    fn every_channel_is_judged_without_a_pixel_budget() {
        assert!(matches(&pixels(0.5), &pixels(0.503)));
        assert!(!matches(&pixels(0.5), &pixels(0.505)));
        assert!(!matches(&pixels(2.0), &pixels(2.01)));
        assert!(!matches(&pixels(f32::INFINITY), &pixels(1.0)));
        assert!(!matches(
            &pixels(0.5),
            &FallbackPixels {
                width: 2,
                ..pixels(0.5)
            }
        ));
    }

    #[test]
    fn pins_and_verifications_survive_restart_and_versions_invalidate_them() {
        let dir = std::env::temp_dir().join(format!("pm-parity-{}", std::process::id()));
        let mut p = Parity::default();
        p.configure(
            Some("engine1; gpu1; driver1; browser1".into()),
            Some(dir.clone()),
        );
        assert_eq!(p.verdict("unstable"), Verdict::Unknown);
        assert_eq!(
            p.judge("unstable", &pixels(0.5), &pixels(0.7)),
            Verdict::Runner
        );
        assert_eq!(
            p.judge("stable", &pixels(0.5), &pixels(0.5)),
            Verdict::NativeVerified
        );
        let mut restarted = Parity::default();
        restarted.configure(p.environment.clone(), Some(dir.clone()));
        assert_eq!(restarted.verdict("unstable"), Verdict::Runner);
        assert_eq!(restarted.verdict("stable"), Verdict::NativeVerified);
        // Different inputs probed concurrently cannot demote a pin on disk.
        p.judge("unstable", &pixels(0.5), &pixels(0.5));
        restarted.configure(p.environment.clone(), Some(dir.clone()));
        assert_eq!(restarted.verdict("unstable"), Verdict::Runner);
        assert_eq!(
            restarted.verdict("stable + changed constant"),
            Verdict::Unknown
        );
        restarted.configure(
            Some("engine1; gpu1; driver1; browser2".into()),
            Some(dir.clone()),
        );
        assert_eq!(restarted.verdict("stable"), Verdict::Unknown);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
