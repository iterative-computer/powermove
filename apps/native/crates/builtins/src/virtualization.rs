//! Fixed-row host geometry for the frozen virtual-list primitive. The model
//! still owns semantic IDs, selection and paging; no full list is materialized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub count: usize,
    pub range: (usize, usize),
    pub item_size: f64,
}
impl Window {
    pub fn new(count: usize, range: (usize, usize), item_size: f64) -> Result<Self, &'static str> {
        if range.0 > range.1
            || range.1 > count
            || !item_size.is_finite()
            || item_size <= 0.
            || !(count as f64 * item_size).is_finite()
        {
            return Err("Invalid virtual window");
        }
        Ok(Self {
            count,
            range,
            item_size,
        })
    }
    pub fn spacers(self) -> (f64, f64) {
        (
            self.range.0 as f64 * self.item_size,
            (self.count - self.range.1) as f64 * self.item_size,
        )
    }
    pub fn visible(self, offset: f64, height: f64) -> Result<(usize, usize), &'static str> {
        if !offset.is_finite() || !height.is_finite() || height < 0. {
            return Err("Invalid virtual viewport");
        }
        let start = (offset.max(0.) / self.item_size).floor() as usize;
        let end = ((offset.max(0.) + height) / self.item_size).ceil() as usize;
        Ok((start.min(self.count), end.min(self.count)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sixty_thousand_rows_have_exact_spacers_and_bounded_middle_page() {
        let w = Window::new(60_000, (30_000, 30_020), 24.).unwrap();
        assert_eq!(w.spacers(), (720_000., 719_520.));
        assert_eq!(w.visible(720_000., 480.).unwrap(), (30_000, 30_020));
        assert_eq!(w.visible(1_440_000., 480.).unwrap(), (60_000, 60_000));
        // Re-publishing the same stable model window preserves the offset.
        assert_eq!(Window::new(w.count, w.range, w.item_size).unwrap(), w);
    }
    #[test]
    fn malformed_ranges_and_geometry_refuse_without_losing_previous_window() {
        assert!(Window::new(2, (2, 1), 24.).is_err());
        assert!(Window::new(2, (0, 3), 24.).is_err());
        assert!(Window::new(2, (0, 1), f64::NAN).is_err());
        let w = Window::new(0, (0, 0), 24.).unwrap();
        assert_eq!(w.spacers(), (0., 0.));
        assert_eq!(w.visible(-10., 100.).unwrap(), (0, 0));
        assert!(w.visible(0., f64::NAN).is_err());
    }
}
