//! Inputs for a private, native effect sweep.
use crate::{Error, Result, extensions::Stage};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub effect_id: String,
    pub extension_id: Option<String>,
    pub backdrop: Backdrop,
    #[serde(default)]
    pub shape: Shape,
    pub width: u32,
    pub height: u32,
    pub parameter_sets: Vec<Map<String, Value>>,
    pub reference_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Backdrop {
    Checker {
        tile_size: u32,
        colors: [String; 2],
    },
    Solid {
        color: String,
    },
    Gradient {
        colors: [String; 2],
        angle: Option<f64>,
    },
    Image {
        path: PathBuf,
    },
    Composition {
        composition_id: String,
        time: f64,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Shape {
    #[default]
    Full,
    Rect {
        width: u32,
        height: u32,
        corner_radius: Option<f64>,
    },
    Capsule {
        width: u32,
        height: u32,
    },
    Circle {
        diameter: u32,
    },
}

pub struct Project {
    pub id: String,
    pub document: u64,
    pub revision: u64,
}

pub struct Context {
    pub project: Option<Project>,
    pub workspace: Option<PathBuf>,
    pub agent_workspace: Option<PathBuf>,
    pub stage: Option<Stage>,
}

impl Request {
    pub fn parse(args: &Value) -> Result<Self> {
        crate::catalog::validate("render_effect_lab", args)?;
        let request: Self = serde_json::from_value(args.clone())
            .map_err(|error| Error::invalid(error.to_string()))?;
        let pixels = u64::from(request.width) * u64::from(request.height);
        if pixels * (request.parameter_sets.len() as u64) > 8 * 1024 * 1024 {
            return Err(Error::invalid(
                "This sweep exceeds the 8-megapixel limit. Reduce the size or use fewer parameter sets.",
            ));
        }
        if args["parameterSets"].to_string().len() > 128 * 1024 {
            return Err(Error::invalid("Effect parameter sets exceed 128 KiB."));
        }
        let (width, height) = match request.shape {
            Shape::Full => (request.width, request.height),
            Shape::Rect { width, height, .. } | Shape::Capsule { width, height } => (width, height),
            Shape::Circle { diameter } => (diameter, diameter),
        };
        if width > request.width || height > request.height {
            return Err(Error::invalid("The shape must fit inside the output."));
        }
        Ok(request)
    }
}

impl Context {
    /// Open each path component relative to the already-open parent directory.
    /// A workspace symlink (including one swapped in during the call) cannot
    /// expose a file outside the run.
    pub fn read_image(&self, path: &Path) -> Result<Vec<u8>> {
        let mut error =
            Error::invalid("Use a regular image file inside the agent or session workspace.");
        for root in [self.agent_workspace.as_deref(), self.workspace.as_deref()]
            .into_iter()
            .flatten()
        {
            match read_workspace_image(root, path) {
                Ok(bytes) => return Ok(bytes),
                Err(failure) => error = failure,
            }
        }
        Err(error)
    }
}

fn read_workspace_image(root: &Path, path: &Path) -> Result<Vec<u8>> {
    let outside =
        || Error::invalid("Use a regular image file inside the agent or session workspace.");
    let relative = if path.is_absolute() {
        path.strip_prefix(root).map_err(|_| outside())?
    } else {
        path
    };
    let mut parts = relative.components().peekable();
    let mut parent = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(root)?;
    while let Some(part) = parts.next() {
        let Component::Normal(name) = part else {
            return Err(outside());
        };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| outside())?;
        let flags = if parts.peek().is_some() {
            libc::O_DIRECTORY
        } else {
            0
        };
        // SAFETY: name is NUL-terminated and parent stays open across the call.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC | flags,
            )
        };
        if fd < 0 {
            return Err(outside());
        }
        // SAFETY: openat returned a new owned descriptor.
        parent = unsafe { File::from_raw_fd(fd) };
    }
    const MAX_BYTES: u64 = 32 * 1024 * 1024;
    let metadata = parent.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(Error::invalid(
            "Use a regular workspace image up to 32 MiB.",
        ));
    }
    let mut bytes = Vec::new();
    parent.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Error::invalid("Use a workspace image up to 32 MiB."));
    }
    Ok(bytes)
}

/// Preserve both image content and structured scores over the existing
/// loopback response limit, including framing and JSON escapes.
pub(crate) fn result(content: Value) -> Value {
    let Ok(metadata) =
        serde_json::from_str::<Value>(content[0]["text"].as_str().unwrap_or_default())
    else {
        return crate::failure(Error::invalid("Invalid effect lab metadata"));
    };
    let result =
        serde_json::json!({"content":content,"structuredContent":metadata,"isError":false});
    if result.to_string().len() + 128 > crate::transport::MAX_LINE {
        return crate::failure(Error::invalid(
            "The contact sheet is too large to send. Reduce the output size or the number of parameter sets.",
        ));
    }
    result
}
