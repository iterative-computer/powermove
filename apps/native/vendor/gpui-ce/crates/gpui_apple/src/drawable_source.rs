//! A Metal display link owns acquisition for its layer, including while paused.
//! Never fall back to nextDrawable after binding a CAMetalDisplayLink.
#[derive(Default)]
pub enum DrawableSource<T> {
    #[default]
    Manual,
    DisplayLink(Option<T>),
}
impl<T> DrawableSource<T> {
    pub fn bind_display_link(&mut self) {
        *self = Self::DisplayLink(None);
    }
    pub fn is_display_link(&self) -> bool {
        matches!(self, Self::DisplayLink(_))
    }
    pub fn ready(&self) -> bool {
        !matches!(self, Self::DisplayLink(None))
    }
    pub fn supply(&mut self, drawable: Option<T>) {
        assert!(
            self.is_display_link(),
            "drawable supplied before binding display link"
        );
        *self = Self::DisplayLink(drawable);
    }
    pub fn take(&mut self, acquire: impl FnOnce() -> Option<T>) -> Option<T> {
        match self {
            Self::Manual => acquire(),
            Self::DisplayLink(drawable) => drawable.take(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appkit_paint_before_during_and_after_a_paused_metal_link() {
        let mut source = DrawableSource::default();
        assert_eq!(source.take(|| Some(1)), Some(1));
        source.bind_display_link();
        let forbidden = || panic!("nextDrawable on a layer owned by CAMetalDisplayLink");
        assert!(!source.ready());
        assert_eq!(source.take(forbidden), None);
        source.supply(Some(2));
        assert!(source.ready());
        assert_eq!(source.take(forbidden), Some(2));
        assert_eq!(source.take(forbidden), None, "one drawable per update");
        source.supply(None); // callback cleanup, also when GPUI didn't paint
        assert!(
            source.is_display_link(),
            "pausing does not release acquisition ownership"
        );
        assert_eq!(source.take(forbidden), None);
        source.supply(Some(3));
        source.supply(None); // unused callback drawable must be released
        assert_eq!(source.take(forbidden), None);
    }
}
