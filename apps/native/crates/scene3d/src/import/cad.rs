//! Native OpenCascade/XDE route, never occt-import-js, Node or Blender. The
//! helper runs on an import worker. Missing installation is an explicit error.
use crate::{Error, Result, fail};
use std::{
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Step,
    Iges,
    Brep,
}
impl Format {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "step" | "stp" => Some(Self::Step),
            "iges" | "igs" => Some(Self::Iges),
            "brep" => Some(Self::Brep),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Iges => "iges",
            Self::Brep => "brep",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub linear_deflection: f64,
    pub angular_deflection: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            linear_deflection: 0.0006,
            angular_deflection: 0.25,
        }
    }
}
pub trait Tessellator: Send + Sync {
    fn convert(&self, format: Format, bytes: &[u8], settings: Settings) -> Result<Vec<u8>>;
}
/// The application supplies its installed, signed helper path. No PATH lookup
/// and no shell. The private scratch directory contains copies, never originals.
pub struct OpenCascade {
    pub helper: PathBuf,
    pub scratch_root: PathBuf,
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Tessellator for OpenCascade {
    fn convert(&self, format: Format, bytes: &[u8], settings: Settings) -> Result<Vec<u8>> {
        if bytes.is_empty() || bytes.len() > super::MAX_BYTES {
            return fail("Invalid CAD input size");
        }
        if !settings.linear_deflection.is_finite()
            || settings.linear_deflection <= 0.
            || !settings.angular_deflection.is_finite()
            || settings.angular_deflection <= 0.
            || settings.angular_deflection > std::f64::consts::PI
        {
            return fail("Invalid CAD tessellation settings");
        }
        if !self.helper.is_absolute() {
            return fail("CAD helper must be an explicit absolute path");
        }
        let helper = std::fs::canonicalize(&self.helper)
            .map_err(|e| Error(format!("Native OpenCascade CAD worker unavailable: {e}")))?;
        let root = std::fs::canonicalize(&self.scratch_root)
            .map_err(|e| Error(format!("CAD scratch root: {e}")))?;
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let scratch = loop {
            let n = SERIAL.fetch_add(1, Ordering::Relaxed);
            let p = root.join(format!("pm-scene3d-cad-{}-{n}", std::process::id()));
            match std::fs::create_dir(&p) {
                Ok(()) => break Scratch(p),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return fail(format!("CAD scratch: {e}")),
            }
        };
        let input = scratch.0.join(format!("source.{}", format.name()));
        let output = scratch.0.join("converted.glb");
        std::fs::write(&input, bytes).map_err(|e| Error(e.to_string()))?;
        let status = Command::new(helper)
            .arg(format.name())
            .arg(&input)
            .arg(&output)
            .arg(settings.linear_deflection.to_string())
            .arg(settings.angular_deflection.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| Error(format!("CAD worker: {e}")))?;
        if !status.success() {
            return fail(format!(
                "Native OpenCascade CAD conversion failed ({status})"
            ));
        }
        let metadata = std::fs::symlink_metadata(&output)
            .map_err(|e| Error(format!("Missing CAD result: {e}")))?;
        if !metadata.is_file() || metadata.len() > super::MAX_BYTES as u64 {
            return fail("Invalid or oversized CAD result");
        }
        let bytes = std::fs::read(output).map_err(|e| Error(e.to_string()))?;
        super::load("converted.glb", &bytes)?;
        Ok(bytes)
    }
}
