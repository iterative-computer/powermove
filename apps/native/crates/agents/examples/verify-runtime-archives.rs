//! Verify/install public pinned macOS archives on Linux without executing them.
//! Run with the directory produced by scripts/capture-runtime-layout.py.
use pm_agents::runtime::{self, Arch, Pin, Probe, Registry, Runtime};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};
struct Archive {
    pin: Pin,
    path: PathBuf,
}
impl Registry for Archive {
    fn metadata(&mut self, _: &str) -> io::Result<Value> {
        Ok(
            json!({"version":self.pin.package_version(),"dist":{"tarball":"https://registry.npmjs.org/verified-fixture.tgz","integrity":self.pin.integrity}}),
        )
    }
    fn download(&mut self, _: &str) -> io::Result<Box<dyn Read>> {
        Ok(Box::new(File::open(&self.path)?))
    }
}
struct HeaderProbe(String);
impl Probe for HeaderProbe {
    fn version(&mut self, binary: &Path) -> io::Result<String> {
        let mut magic = [0; 4];
        File::open(binary)?.read_exact(&mut magic)?;
        if magic != [0xcf, 0xfa, 0xed, 0xfe] {
            return Err(io::Error::other("Expected Mach-O 64-bit native executable"));
        }
        Ok(self.0.clone())
    }
}
fn main() -> io::Result<()> {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| io::Error::other("Pass the public tarball fixture directory"))?,
    );
    for provider in [Runtime::Codex, Runtime::Claude] {
        for (arch, name) in [(Arch::Arm64, "arm64"), (Arch::X64, "x64")] {
            let pin = runtime::bundled_pin(provider, arch);
            let mut source = Archive {
                pin: pin.clone(),
                path: dir.join(format!("{}-{name}.tgz", provider.name())),
            };
            let root = dir.join(format!("verified-runtimes-{}-{name}", provider.name()));
            let binary = runtime::install(
                &root,
                "layout-check",
                &pin,
                &mut source,
                &mut HeaderProbe(pin.version.clone()),
            )?;
            if provider == Runtime::Codex {
                let tree = binary.parent().unwrap().parent().unwrap();
                for relative in [
                    "bin/codex-code-mode-host",
                    "codex-path/rg",
                    "codex-resources/zsh/bin/zsh",
                    "codex-resources/voice/bin/codex-voice-host",
                ] {
                    if !fs::metadata(tree.join(relative))?.is_file() {
                        return Err(io::Error::other("Native helper/resource is missing"));
                    }
                }
            }
            println!(
                "{} {name}: pinned SRI, extraction and Mach-O header verified; no executable launched",
                provider.name()
            );
        }
    }
    Ok(())
}
