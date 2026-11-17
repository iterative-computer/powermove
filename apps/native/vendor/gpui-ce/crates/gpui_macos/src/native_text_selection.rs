// AppKit ranges count UTF-16 code units, as JavaScript string lengths do.
// Keep the range calculation independent of AppKit so the app's test target
// can exercise it without the vendored macOS font-fixture test dependencies.
pub(super) fn selection_range(start: usize, end: usize, length: usize) -> (usize, usize) {
    let (start, end) = (start.min(length), end.min(length));
    (start.min(end), start.max(end) - start.min(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_selection_leaves_the_extension_and_clamps_all_selection() {
        let name = "Film 🎬.mov";
        let length = name.encode_utf16().count();
        let stem = "Film 🎬".encode_utf16().count();
        assert_eq!(selection_range(0, stem, length), (0, stem));
        assert_eq!(selection_range(0, usize::MAX, length), (0, length));
        assert_eq!(selection_range(length + 3, 2, length), (2, length - 2));
    }
}
