//! Immutable native import objects. Call on an import worker, before enqueuing
//! a kernel command. The store never overwrites or collects an existing object.
use super::{Model, Prepared};
use crate::{Error, Result, fail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    path: PathBuf,
    hash: String,
    length: u64,
    name: String,
    mime: String,
}
impl Blob {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn hash(&self) -> &str {
        &self.hash
    }
    pub fn length(&self) -> u64 {
        self.length
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn mime(&self) -> &str {
        &self.mime
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Receipt {
    primary: Blob,
    originals: Vec<Blob>,
    model: Arc<Model>,
}
impl Receipt {
    pub fn primary(&self) -> &Blob {
        &self.primary
    }
    pub fn originals(&self) -> &[Blob] {
        &self.originals
    }
    pub fn model(&self) -> Arc<Model> {
        self.model.clone()
    }
}
pub struct Archive {
    root: PathBuf,
}
fn io(e: std::io::Error) -> Error {
    Error(format!("Native 3D import storage: {e}"))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
impl Archive {
    /// The host passes its explicit native-profile destination. Resolve system
    /// temp aliases once, then refuse symlinks within this owned destination.
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root).map_err(io)?;
        let root = std::fs::canonicalize(root).map_err(io)?;
        let objects = root.join("objects");
        match std::fs::create_dir(&objects) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if !std::fs::symlink_metadata(&objects).map_err(io)?.is_dir() {
                    return fail("3D archive objects must be an ordinary directory");
                }
            }
            Err(e) => return Err(io(e)),
        }
        Ok(Self { root: objects })
    }
    pub fn admit(&self, prepared: Prepared) -> Result<Receipt> {
        let model = super::load(&prepared.name, &prepared.bytes)?;
        let mut originals = vec![];
        for f in &prepared.original {
            originals.push(self.put(&f.name, &f.mime, &f.bytes)?);
        }
        let primary = self.put(&prepared.name, &mime(&prepared.name), &prepared.bytes)?;
        Ok(Receipt {
            primary,
            originals,
            model: Arc::new(model),
        })
    }
    /// Recover only content-addressed objects inside this admitted archive.
    /// Saved project metadata never grants access to an arbitrary source path.
    pub fn recover(&self, metadata: &Value) -> Result<Receipt> {
        let get = |name: &str, mime: &str, hash: &str, length: u64| -> Result<Blob> {
            if hash.len() != 64
                || !hash.bytes().all(|b| b.is_ascii_hexdigit())
                || length > super::MAX_BYTES as u64
            {
                return fail("Invalid retained 3D object identity");
            }
            let path = self.root.join(hash);
            let meta = std::fs::symlink_metadata(&path).map_err(io)?;
            if !meta.is_file() || meta.len() != length {
                return fail("A retained 3D object is missing or changed");
            }
            let bytes = std::fs::read(&path).map_err(io)?;
            if hex(&Sha256::digest(&bytes)) != hash {
                return fail("A retained 3D object failed checksum verification");
            }
            Ok(Blob {
                path,
                hash: hash.into(),
                length,
                name: name.into(),
                mime: mime.into(),
            })
        };
        let name = metadata["name"]
            .as_str()
            .ok_or_else(|| Error("Missing 3D import name".into()))?;
        let hash = metadata["revision"]
            .as_str()
            .ok_or_else(|| Error("Missing 3D import hash".into()))?;
        let length = metadata["size"]
            .as_u64()
            .ok_or_else(|| Error("Missing 3D import length".into()))?;
        let primary = get(name, &mime(name), hash, length)?;
        let mut originals = vec![];
        for source in metadata["native3dSources"]
            .as_array()
            .ok_or_else(|| Error("Missing retained 3D sources".into()))?
        {
            originals.push(get(
                source["name"]
                    .as_str()
                    .ok_or_else(|| Error("Missing 3D source name".into()))?,
                source["mime"]
                    .as_str()
                    .unwrap_or("application/octet-stream"),
                source["sha256"]
                    .as_str()
                    .ok_or_else(|| Error("Missing 3D source hash".into()))?,
                source["size"]
                    .as_u64()
                    .ok_or_else(|| Error("Missing 3D source length".into()))?,
            )?);
        }
        let bytes = std::fs::read(primary.path()).map_err(io)?;
        let model = super::load(primary.name(), &bytes)?;
        Ok(Receipt {
            primary,
            originals,
            model: Arc::new(model),
        })
    }
    fn put(&self, name: &str, mime: &str, bytes: &[u8]) -> Result<Blob> {
        let hash = hex(&Sha256::digest(bytes));
        let destination = self.root.join(&hash);
        let verify = || -> Result<()> {
            let meta = std::fs::symlink_metadata(&destination).map_err(io)?;
            if !meta.is_file() || meta.len() != bytes.len() as u64 {
                return fail(
                    "A retained 3D object conflicts with its content hash; it was preserved",
                );
            }
            let mut file = std::fs::File::open(&destination).map_err(io)?;
            let mut h = Sha256::new();
            let mut buf = [0; 65536];
            loop {
                let n = file.read(&mut buf).map_err(io)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            if hex(&h.finalize()) != hash {
                return fail(
                    "A retained 3D object conflicts with its content hash; it was preserved",
                );
            }
            Ok(())
        };
        if destination.try_exists().map_err(io)? {
            verify()?;
        } else {
            static SERIAL: AtomicU64 = AtomicU64::new(0);
            let (tmp, mut file) = loop {
                let p = self.root.join(format!(
                    ".import-{}-{}",
                    std::process::id(),
                    SERIAL.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&p)
                {
                    Ok(f) => break (p, f),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(io(e)),
                }
            };
            let result = (|| -> Result<()> {
                file.write_all(bytes).map_err(io)?;
                file.sync_all().map_err(io)?;
                drop(file);
                match std::fs::hard_link(&tmp, &destination) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => verify()?,
                    Err(e) => return Err(io(e)),
                }
                sync_directory(&self.root)?;
                Ok(())
            })();
            let _ = std::fs::remove_file(tmp);
            result?;
        }
        Ok(Blob {
            path: destination,
            hash,
            length: bytes.len() as u64,
            name: name.into(),
            mime: mime.into(),
        })
    }
}
#[cfg(unix)]
fn sync_directory(p: &Path) -> Result<()> {
    std::fs::File::open(p).map_err(io)?.sync_all().map_err(io)
}
#[cfg(not(unix))]
fn sync_directory(_: &Path) -> Result<()> {
    Ok(())
}
fn mime(name: &str) -> String {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "glb" => "model/gltf-binary",
        "gltf" => "model/gltf+json",
        "obj" => "model/obj",
        "ply" => "application/x-ply",
        _ => "application/octet-stream",
    }
    .into()
}
