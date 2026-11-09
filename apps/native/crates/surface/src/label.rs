//! A readable name for a stored identifier when no schema names it:
//! `backgroundColor` → `Background Color`, `fx.glow.dot_grid` → `Dot Grid`.

/// The last path segment, split at case changes, `_`, `-` and spaces, each
/// word capitalised. Digits stay attached; an empty id stays empty.
pub fn humanize(id: &str) -> String {
    let last = id.rsplit('.').next().unwrap_or(id);
    let mut words: Vec<String> = vec![];
    let mut word = String::new();
    let mut previous: Option<char> = None;
    for c in last.chars() {
        let boundary = matches!(c, '_' | '-' | ' ');
        let case_change =
            c.is_uppercase() && previous.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit());
        if boundary || case_change {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            if boundary {
                previous = None;
                continue;
            }
        }
        word.push(c);
        previous = Some(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
        .iter()
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|f| f.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::humanize;
    #[test]
    fn stored_ids_read_as_words() {
        assert_eq!(humanize("backgroundColor"), "Background Color");
        assert_eq!(humanize("fx.glow.dot_grid"), "Dot Grid");
        assert_eq!(humanize("opacity"), "Opacity");
        assert_eq!(humanize("position.x"), "X");
        assert_eq!(humanize("blendMode"), "Blend Mode");
        assert_eq!(humanize("wght"), "Wght");
        assert_eq!(humanize("HDR"), "HDR");
        assert_eq!(humanize(""), "");
    }
}
