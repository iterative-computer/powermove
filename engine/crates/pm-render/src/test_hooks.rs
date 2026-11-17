//! Failures a test injects into a definition's compile or program build,
//! keyed by definition id so tests running side by side never see each
//! other's (test builds and the `test-hooks` feature only).
//!
//! A child process (an export job's engine) takes them from
//! `PM_RENDER_TEST_FAILURES`: `;`-separated `compile|build:<id>:<stage>[*n]`,
//! the stage by its name (`compiler-busy`), read once.
use pm_glsl::{Failure, FailureStage};
use std::{collections::HashMap, sync::Mutex};

/// The variable a test sets for a child process's injected failures.
pub const ENV: &str = "PM_RENDER_TEST_FAILURES";

fn from_env() {
    static READ: std::sync::Once = std::sync::Once::new();
    READ.call_once(|| {
        let Ok(spec) = std::env::var(ENV) else { return };
        for item in spec.split(';').filter(|s| !s.is_empty()) {
            let mut parts = item.splitn(3, ':');
            let (Some(what), Some(id), Some(stage)) = (parts.next(), parts.next(), parts.next())
            else {
                panic!("{ENV}: {item}");
            };
            let (stage, n) = match stage.split_once('*') {
                Some((stage, n)) => (stage, n.parse().expect("a count")),
                None => (stage, 1),
            };
            let stage = [
                FailureStage::CompilerBusy,
                FailureStage::CompilerProcess,
                FailureStage::CompilerResources,
                FailureStage::Deadline,
            ]
            .into_iter()
            .find(|s| s.name() == stage)
            .unwrap_or_else(|| panic!("{ENV}: stage {stage}"));
            let failures = vec![Failure::new(stage, format!("injected {}", stage.name())); n];
            match what {
                "compile" => fail_compiles(id, failures),
                "build" => fail_builds(id, failures),
                _ => panic!("{ENV}: {item}"),
            }
        }
    });
}

static PREPARES: Mutex<Option<HashMap<String, Vec<Failure>>>> = Mutex::new(None);
static BUILDS: Mutex<Option<HashMap<String, Vec<Failure>>>> = Mutex::new(None);

fn push(map: &Mutex<Option<HashMap<String, Vec<Failure>>>>, id: &str, failures: Vec<Failure>) {
    let mut map = map.lock().unwrap();
    let list = map
        .get_or_insert_with(HashMap::new)
        .entry(id.to_owned())
        .or_default();
    list.extend(failures);
}
fn pop(map: &Mutex<Option<HashMap<String, Vec<Failure>>>>, id: &str) -> Option<Failure> {
    let mut map = map.lock().unwrap();
    let list = map.as_mut()?.get_mut(id)?;
    (!list.is_empty()).then(|| list.remove(0))
}
/// The next compiles of definition `id` fail with `failures`, in order.
pub fn fail_compiles(id: &str, failures: Vec<Failure>) {
    push(&PREPARES, id, failures);
}
/// The next program builds of definition `id` fail with `failures`.
pub fn fail_builds(id: &str, failures: Vec<Failure>) {
    push(&BUILDS, id, failures);
}
/// Injected failures of `id` not taken yet (compiles, builds).
pub fn left(id: &str) -> (usize, usize) {
    let n = |m: &Mutex<Option<HashMap<String, Vec<Failure>>>>| {
        m.lock()
            .unwrap()
            .as_ref()
            .and_then(|m| m.get(id))
            .map_or(0, Vec::len)
    };
    (n(&PREPARES), n(&BUILDS))
}
pub(crate) fn next_compile(id: &str) -> Option<Failure> {
    from_env();
    pop(&PREPARES, id)
}
pub(crate) fn next_build(id: &str) -> Option<Failure> {
    from_env();
    pop(&BUILDS, id)
}
