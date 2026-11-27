//! The loaded theme's `nativePatch`, applied every time the shell builds tokens
//! (launch, appearance and accessibility changes), so a theme survives a switch
//! between light and dark and Increase Contrast is always re-applied on top
//! (`Tokens::apply_patch`).
use pm_design::{Accessibility, PatchError, Scheme, Tokens};
use std::cell::RefCell;

/// Field name (`"colors.accent"`) → value, in the order the theme gave them.
pub type Patch = Vec<(String, f64)>;

thread_local! {
    /// The patch for dark and for light (a v1–3 theme has one per scheme).
    static THEME: RefCell<[Patch; 2]> = const { RefCell::new([Vec::new(), Vec::new()]) };
}

/// Parses the theme pipeline's output: either `{ "nativePatch": { … } }`
/// (`evaluateTheme`) or the bare field map.
pub fn parse(json: &str) -> Result<Patch, PatchError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| PatchError(format!("theme is not JSON: {e}")))?;
    let map = value
        .get("nativePatch")
        .unwrap_or(&value)
        .as_object()
        .ok_or_else(|| PatchError("theme has no nativePatch object".into()))?;
    map.iter()
        .map(|(k, v)| {
            v.as_f64()
                .map(|n| (k.clone(), n))
                .ok_or_else(|| PatchError(format!("{k} needs a number")))
        })
        .collect()
}

/// Makes `patch` the current theme for both schemes. A patch that pm-design
/// rejects changes nothing (the previous theme stays).
pub fn set(patch: Patch) -> Result<(), PatchError> {
    set_schemes(patch.clone(), patch)
}

/// Makes `dark` and `light` the current theme, each used by windows in that
/// appearance, so switching appearance needs no new theme.
pub fn set_schemes(dark: Patch, light: Patch) -> Result<(), PatchError> {
    for (scheme, patch) in [(Scheme::Dark, &dark), (Scheme::Light, &light)] {
        Tokens::new(scheme, Accessibility::default())
            .apply_patch(patch.iter().map(|(k, v)| (k.as_str(), *v)))?;
    }
    THEME.with(|t| *t.borrow_mut() = [dark, light]);
    Ok(())
}

/// Clears the theme (pm-design defaults).
pub fn clear() {
    THEME.with(|t| *t.borrow_mut() = [Vec::new(), Vec::new()]);
}

/// Tokens for `scheme` with the current theme applied.
pub fn tokens(scheme: Scheme, accessibility: Accessibility) -> Tokens {
    let mut t = Tokens::new(scheme, accessibility);
    THEME.with(|p| {
        let p = &p.borrow()[usize::from(scheme == Scheme::Light)];
        // `set` validated the patch; a failure here would leave `t` untouched.
        let _ = t.apply_patch(p.iter().map(|(k, v)| (k.as_str(), *v)));
    });
    t
}

/// The current tokens as the theme pipeline's `environment.nativeTokens`
/// (`{"metrics.gap": 6, …}`), so length patches are relative to native values.
pub fn native_tokens(t: &Tokens) -> serde_json::Map<String, serde_json::Value> {
    Tokens::PATCHABLE
        .iter()
        .filter_map(|name| Some((name.to_string(), t.field(name)?.into())))
        .collect()
}

/// Loads `PM_NATIVE_THEME` (a JSON file from the theme pipeline) at launch.
/// Returns the error text when the file is unusable (logged at launch).
pub fn load_from_env() -> Option<String> {
    let path = std::env::var_os("PM_NATIVE_THEME")?;
    let result = std::fs::read_to_string(&path)
        .map_err(|e| PatchError(e.to_string()))
        .and_then(|s| parse(&s))
        .and_then(set);
    result
        .err()
        .map(|e| format!("The theme could not be loaded: {}", e.0))
}
