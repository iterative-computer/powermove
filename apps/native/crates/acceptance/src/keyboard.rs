//! Linux can exercise the actual shell's pure resolver without linking the
//! macOS-only pm-shell crate. This compiles its source in place, never a copy or
//! alternative dispatcher. Command effects and AppKit focus still need macOS.
#[path = "../../shell/src/keyboard.rs"]
pub mod production;
use crate::catalog::Binding;
use production::{Effect, Key, Resolver};

pub const CONTEXTS: &[&str] = &[
    "normal",
    "held",
    "extra-modifier",
    "input",
    "contenteditable",
    "prior-decline",
    "key-up",
];
pub fn check(binding: &Binding, context: &str) -> Result<(), String> {
    let real = production::shipped()
        .into_iter()
        .find(|b| b.id == binding.id)
        .ok_or("binding missing from real shell")?;
    assert_eq!(real.chord, binding.chord);
    assert_eq!(real.command, binding.command);
    assert_eq!(real.args, *binding.args.as_array().ok_or("args not array")?);
    assert_eq!(i64::from(real.priority), binding.priority);
    assert_eq!(real.in_fields, binding.in_fields);
    assert_eq!(real.repeat, binding.repeat);
    assert_eq!(real.loose, binding.loose);
    let mut resolver = Resolver::new();
    resolver.bindings = vec![real.clone()];
    let mut key = Key {
        token: 73,
        chord: binding.chord.clone(),
        field: false,
        repeat: false,
        up: false,
        compat: false,
    };
    let expected = match context {
        "normal" => true,
        "held" => {
            key.repeat = true;
            binding.repeat
        }
        "input" | "contenteditable" => {
            key.field = true;
            binding.in_fields
        }
        "key-up" => {
            key.up = true;
            false
        }
        "extra-modifier" => {
            let modifier = ["alt", "shift", "ctrl", "cmd"]
                .into_iter()
                .find(|m| !binding.chord.split('+').any(|p| p == *m))
                .ok_or("no unused modifier")?;
            key.chord = format!("{modifier}+{}", key.chord);
            binding.loose
        }
        "prior-decline" => {
            let mut prior = real.clone();
            prior.id = "synthetic.decline".into();
            prior.command = "synthetic.decline".into();
            prior.priority -= 1;
            resolver.bindings.push(prior);
            true
        }
        _ => return Err("unknown routing context".into()),
    };
    let mut effects = resolver.submit(key);
    if context == "prior-decline" {
        assert!(
            matches!(&effects[..],[Effect::Dispatch{command,..}] if command=="synthetic.decline")
        );
        effects = resolver.resolve(73, false);
    }
    if expected {
        assert!(
            matches!(&effects[..],[Effect::Dispatch{token:73,command,args,..}] if command==&binding.command && args==binding.args.as_array().unwrap()),
            "{} {context}: {effects:?}",
            binding.id
        );
        let next = resolver.resolve(73, true);
        assert!(
            !next
                .iter()
                .any(|effect| matches!(effect, Effect::Dispatch { .. })),
            "no double execution"
        );
    } else {
        assert_eq!(
            effects,
            vec![Effect::Replay(73)],
            "{} {context}",
            binding.id
        );
        assert!(resolver.replayed(73).is_empty());
    }
    // Exercise the physical-key production entry without substituting it for
    // keyboard-layout/IME verification on macOS.
    assert_eq!(production::physical_fallback("!".into(), 18, true), "1");
    Ok(())
}
