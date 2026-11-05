//! Platform-neutral qualification contract. A Linux test double validates the
//! checker; only the Mac adapter can provide native default-path pixels.
pub trait PixelRenderer {
    fn frame(&mut self, path: &str, size: (u32, u32), export: bool) -> Result<Vec<f32>, String>;
}

pub fn qualify(
    renderer: &mut impl PixelRenderer,
    plain: &str,
    rotated: &str,
    coded: (u32, u32),
    turns: u32,
) -> Result<(), String> {
    if !(1..=3).contains(&turns) || coded.0 == 0 || coded.1 == 0 {
        return Err("invalid synthetic rotation".into());
    }
    let (w, h) = coded;
    let (ow, oh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    for export in [false, true] {
        let base = renderer.frame(plain, coded, export)?;
        let ours = renderer.frame(rotated, (ow, oh), export)?;
        let length = w as usize * h as usize * 4;
        if base.len() != length || ours.len() != length {
            return Err("rotation pixel count".into());
        }
        if !base.iter().chain(&ours).all(|x| x.is_finite()) {
            return Err("nonfinite rotation pixels".into());
        }
        if !base.chunks_exact(4).any(|p| p[0] != p[1]) {
            return Err("blank rotation witness".into());
        }
        for y in 0..oh {
            for x in 0..ow {
                let (u, v) = match turns {
                    1 => (w - 1 - y, x),
                    2 => (w - 1 - x, h - 1 - y),
                    _ => (y, h - 1 - x),
                };
                let a = ((y * ow + x) * 4) as usize;
                let b = ((v * w + u) * 4) as usize;
                if ours[a..a + 4] != base[b..b + 4] {
                    return Err(format!(
                        "rotation pixel mismatch ({x},{y}), export={export}"
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        rotated: Result<Vec<f32>, String>,
        base: Vec<f32>,
        calls: Vec<bool>,
    }
    fn pixels(indices: &[usize]) -> Vec<f32> {
        indices
            .iter()
            .flat_map(|i| [*i as f32 + 1., *i as f32 * 0.25, 0.4, 1.])
            .collect()
    }
    impl PixelRenderer for Fake {
        fn frame(
            &mut self,
            path: &str,
            _size: (u32, u32),
            export: bool,
        ) -> Result<Vec<f32>, String> {
            self.calls.push(export);
            if path == "base" {
                Ok(self.base.clone())
            } else {
                self.rotated.clone()
            }
        }
    }
    #[test]
    fn exact_permutations_are_required_in_preview_and_export() {
        for (turns, indices) in [
            (1, vec![1, 3, 5, 0, 2, 4]),
            (2, vec![5, 4, 3, 2, 1, 0]),
            (3, vec![4, 2, 0, 5, 3, 1]),
        ] {
            let mut fake = Fake {
                base: pixels(&[0, 1, 2, 3, 4, 5]),
                rotated: Ok(pixels(&indices)),
                calls: vec![],
            };
            qualify(&mut fake, "base", "rotated", (2, 3), turns).unwrap();
            assert_eq!(fake.calls, [false, false, true, true]);
        }
    }
    #[test]
    fn default_path_guard_is_propagated_without_a_pass() {
        let mut fake = Fake {
            base: pixels(&[0, 1, 2, 3, 4, 5]),
            rotated: Err("rotated video guard".into()),
            calls: vec![],
        };
        assert_eq!(
            qualify(&mut fake, "base", "rotated", (2, 3), 1),
            Err("rotated video guard".into())
        );
    }
    #[test]
    fn wrong_direction_and_blank_output_fail() {
        let mut fake = Fake {
            base: pixels(&[0, 1, 2, 3, 4, 5]),
            rotated: Ok(pixels(&[4, 2, 0, 5, 3, 1])),
            calls: vec![],
        };
        assert!(
            qualify(&mut fake, "base", "rotated", (2, 3), 1)
                .unwrap_err()
                .contains("mismatch")
        );
        fake.base = vec![0.; 24];
        fake.rotated = Ok(vec![0.; 24]);
        assert!(
            qualify(&mut fake, "base", "rotated", (2, 3), 1)
                .unwrap_err()
                .contains("blank")
        );
    }
    #[test]
    fn truncated_and_nonfinite_frames_fail() {
        let mut fake = Fake {
            base: pixels(&[0, 1, 2, 3, 4, 5]),
            rotated: Ok(vec![]),
            calls: vec![],
        };
        assert!(
            qualify(&mut fake, "base", "rotated", (2, 3), 1)
                .unwrap_err()
                .contains("count")
        );
        fake.rotated = Ok(vec![f32::NAN; 24]);
        assert!(
            qualify(&mut fake, "base", "rotated", (2, 3), 1)
                .unwrap_err()
                .contains("nonfinite")
        );
    }
}
