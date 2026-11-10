//! Writes a synthetic large project (`pm_kernel::synthetic`) as a `.pmv`.
//! Keep the output out of the repository.
//!
//! cargo run --release -p pm-kernel --features synthetic --example scale_project -- 5000 /private/tmp/scale-5000.pmv
use pm_kernel::DocumentStore;
use pm_kernel::project_io::{SaveOptions, SaveTarget};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(layers), Some(path)) = (args.next().and_then(|n| n.parse().ok()), args.next()) else {
        eprintln!("usage: scale_project LAYERS OUT.pmv");
        std::process::exit(2);
    };
    let mut store = DocumentStore::new_untitled(pm_kernel::synthetic::project(layers));
    let plan = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: path.clone().into(),
                replace: true,
            },
            SaveOptions::default(),
        )
        .expect("prepare save");
    let receipt = plan.run().expect("save");
    store.finish_save(receipt).expect("finish save");
    println!("{path}: {layers} layers");
}
