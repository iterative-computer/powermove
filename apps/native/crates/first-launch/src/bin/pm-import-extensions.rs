//! Background-only maintenance command; close the destination app first.
use anyhow::{Result, ensure};
use pm_first_launch::{Config, extension_transfer};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 4 && args[0] == "--source" && args[2] == "--profile",
        "Usage: pm-import-extensions --source <Electron profile> --profile <closed native profile>"
    );
    let result = extension_transfer::run(
        &Config {
            electron_profile: args[1].clone().into(),
            native_profile: args[3].clone().into(),
        },
        |_| {},
        |_, _| Ok("Copied; runtime activation has not been checked.".into()),
    )?;
    println!(
        "{} copied; {} already present; {} errors. Runtime activation has NOT been checked.",
        result.brought_in, result.already_present, result.could_not_load
    );
    for entry in result.entries {
        println!("{}: {}", entry.id, entry.reason);
    }
    Ok(())
}
