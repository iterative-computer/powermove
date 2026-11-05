//! The frozen evaluator/font corpus gate requires macOS CoreText. Keep an
//! explicit failed target on other platforms, so --no-fail-fast can run the
//! remaining suites without reporting unavailable corpus proof as success.

#[cfg(target_os = "macos")]
#[path = "support/corpus_gate.rs"]
mod corpus_gate;

fn main() {
    #[cfg(target_os = "macos")]
    corpus_gate::run();

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!(
            "BLOCKED: pm-eval corpus requires macOS CoreText and the frozen PM_ENGINE_CORPUS witness"
        );
        std::process::exit(1);
    }
}
