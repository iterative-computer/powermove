//! Consent and installation for the inert packages in a portable project.
use gpui::{
    AppContext, AsyncApp, Bounds, WindowBounds, WindowKind, WindowOptions, point, px, size,
};
use pm_compat_runner::host::ProjectScope;
use pm_mcp::project_extensions::{Bundle, Entry, Status, available_locally, installed};
use pm_shell::welcome::{Button, Frame, Row, Severity};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

const ORIGIN: &str = ".powermove-project-origin.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Choice {
    Project,
    Library,
    Without,
}

pub struct Open {
    bundle: Bundle,
    required: Vec<Entry>,
    excluded: std::collections::BTreeSet<String>,
    warnings: Vec<String>,
}

impl Open {
    pub fn prepare(extra: &pm_model::Map, profile: &Path) -> Self {
        let library = installed(&crate::extensions::roots(profile));
        let mut warnings = vec![];
        let bundle = Bundle::read(extra).unwrap_or_else(|error| {
            warnings.push(format!(
                "The bundled extension manifest couldn’t be read. {error}"
            ));
            Bundle::default()
        });
        let excluded = bundle.exclusions(&library);
        let mut required = vec![];
        for entry in &bundle.entries {
            let installed = available_locally(entry, &library);
            match entry.status {
                Status::Invalid => warnings.push(format!(
                    "{} couldn’t be used because its integrity hash or package did not match.",
                    entry.name
                )),
                Status::StoreOnly if !installed => warnings.push(format!(
                    "{} is needed to render. Install it from the Store ({}).",
                    entry.name,
                    entry.store_id.as_deref().unwrap_or(&entry.id)
                )),
                Status::Unavailable | Status::Excluded if !installed => warnings.push(format!(
                    "{} is needed to render and is unavailable.",
                    entry.name
                )),
                _ => {}
            }
            if bundle.needs_install(entry, &library) {
                let valid = bundle
                    .files(&entry.id)
                    .and_then(|files| files.get("manifest.json"))
                    .and_then(|raw| serde_json::from_slice(raw).ok())
                    .and_then(|raw| pm_compat_runner::manifest::parse(&raw).ok())
                    .is_some();
                if valid {
                    required.push(entry.clone());
                } else {
                    warnings.push(format!("{} has an invalid extension manifest.", entry.name));
                }
            }
        }
        Self {
            bundle,
            required,
            excluded,
            warnings,
        }
    }

    pub fn frame(&self) -> Option<Frame> {
        let count = self.required.len();
        if count == 0 && self.warnings.is_empty() {
            return None;
        }
        let rows = self
            .required
            .iter()
            .map(|entry| Row {
                symbol: "puzzlepiece.extension",
                severity: Severity::Note,
                text: format!("{} {}", entry.name, entry.version),
                detail: Some(format!(
                    "{} · {}",
                    if entry.author.is_empty() {
                        "Unknown author"
                    } else {
                        &entry.author
                    },
                    if entry.source == "store" {
                        "Store"
                    } else {
                        "Local extension"
                    }
                )),
            })
            .chain(self.warnings.iter().map(|warning| Row {
                symbol: "exclamationmark.triangle.fill",
                severity: Severity::Warning,
                text: warning.clone(),
                detail: None,
            }))
            .collect();
        let (title, line, buttons, default) = if count > 0 {
            (
                if count == 1 {
                    "This project uses 1 extension to render correctly.".into()
                } else {
                    format!("This project uses {count} extensions to render correctly.")
                },
                "Choose where to install the bundled extensions. You can keep editing if you open without them.",
                vec![
                    Button::new("without", "Open without").leading(),
                    Button::new("library", "Install to Library"),
                    Button::new("project", "Install for this project"),
                ],
                "project",
            )
        } else {
            (
                "Some project extensions are unavailable".into(),
                "You can open the project and keep editing.",
                vec![Button::new("without", "Open without")],
                "without",
            )
        };
        Some(Frame {
            surface: "project-extensions".into(),
            title,
            line: line.into(),
            rows,
            buttons,
            default: Some(default.into()),
            ..Frame::default()
        })
    }

    pub fn install(
        self,
        choice: Choice,
        profile: &Path,
        name: &str,
    ) -> (ProjectScope, Vec<String>) {
        let mut scope = ProjectScope {
            excluded: self.excluded,
            ..Default::default()
        };
        let mut failures = vec![];
        if choice == Choice::Without {
            return (scope, failures);
        }
        let mut library_changed = false;
        for entry in self.required {
            let result = (|| -> std::io::Result<()> {
                let files = self.bundle.files(&entry.id).expect("verified package");
                if choice == Choice::Project {
                    let root = if let Some(root) = &scope.root {
                        root.clone()
                    } else {
                        let root = Arc::new(
                            tempfile::Builder::new()
                                .prefix("pm-project-extensions-")
                                .tempdir()?,
                        );
                        scope.root = Some(root.clone());
                        root
                    };
                    let dest = root.path().join(&entry.id);
                    if let Err(error) = unpack(&dest, files).and_then(|()| origin(&dest, name)) {
                        let _ = fs::remove_dir_all(dest);
                        return Err(error);
                    }
                    scope.installed.insert(entry.id.clone());
                } else {
                    install_library(profile, name, &entry.id, files)?;
                    library_changed = true;
                }
                scope.excluded.remove(&entry.id);
                Ok(())
            })();
            if let Err(error) = result {
                failures.push(format!("{} couldn’t be installed. {error}", entry.name));
            }
        }
        if library_changed {
            pm_compat_runner::host::rescan_all();
        }
        (scope, failures)
    }
}

fn unpack(
    directory: &Path,
    files: &std::collections::BTreeMap<String, Vec<u8>>,
) -> std::io::Result<()> {
    fs::create_dir(directory)?;
    for (path, data) in files {
        let dest = directory.join(path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        // Hash verification validates the portable path spelling. create_new
        // also rejects collisions on case-insensitive filesystems.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)?
            .write_all(data)?;
    }
    Ok(())
}

fn install_library(
    profile: &Path,
    project: &str,
    id: &str,
    files: &std::collections::BTreeMap<String, Vec<u8>>,
) -> std::io::Result<()> {
    let library = profile.join("extensions");
    fs::create_dir_all(&library)?;
    let stage = tempfile::Builder::new()
        .prefix(".project-extension-")
        .tempdir_in(&library)?;
    let incoming = stage.path().join(id);
    unpack(&incoming, files)?;
    origin(&incoming, project)?;
    let destination = library.join(id);
    let archive = profile.join("project-extension-replacements");
    let replace = match destination.symlink_metadata() {
        Ok(meta) if meta.is_dir() => true,
        Ok(_) => {
            return Err(std::io::Error::other(
                "The Library destination is occupied.",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if replace {
        fs::create_dir_all(&archive)?;
    }
    // Swap folders atomically, so running readers either see the old install
    // or the verified replacement. An intervening new install is refused.
    rename_install(&incoming, &destination, replace)?;
    if replace {
        let previous = archive.join(pm_kernel::new_id(id));
        if fs::rename(&incoming, &previous).is_err() {
            // The installed copy is already complete. Keep its predecessor
            // in the hidden staging folder if archiving it fails.
            let _ = stage.keep();
        }
    }
    Ok(())
}

fn origin(directory: &Path, project: &str) -> std::io::Result<()> {
    let marker = serde_json::json!({"project":project}).to_string();
    fs::write(directory.join(ORIGIN), marker)
}

fn rename_install(source: &Path, destination: &Path, replace: bool) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    let flags = if replace {
        libc::RENAME_SWAP
    } else {
        libc::RENAME_EXCL
    };
    // Paths are NUL-terminated, and the verified source is a private
    // directory on the same volume as the Library.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn sheet(
    frame: Frame,
    tokens: pm_design::Tokens,
    parent: gpui::AnyWindowHandle,
    cx: &mut AsyncApp,
) -> Option<async_channel::Receiver<Choice>> {
    if gpui_macos::child_window::would_hold() {
        return None;
    }
    let (tx, rx) = async_channel::bounded(1);
    let options = WindowOptions {
        parent: Some(parent),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(tokens.metrics.sheet_width_wide), px(360.)),
        ))),
        ..WindowOptions::new().kind(WindowKind::Dialog)
    };
    cx.open_window(options, move |window, cx| {
        let cancel = tx.clone();
        window.on_window_should_close(cx, move |_, _| {
            let _ = cancel.try_send(Choice::Without);
            true
        });
        let handle = window.window_handle();
        let action: pm_shell::welcome_view::OnAction = Rc::new(move |id, cx| {
            let choice = match id {
                "project" => Choice::Project,
                "library" => Choice::Library,
                _ => Choice::Without,
            };
            if tx.try_send(choice).is_ok() {
                let _ = cx.update_window(handle, |_, window, _| window.remove_window());
            }
        });
        let view = cx.new(|cx| pm_shell::welcome_view::WelcomeView::new(tokens, frame, action, cx));
        let focus = view.read(cx).focus_handle();
        focus.focus(window, cx);
        view
    })
    .ok()?;
    Some(rx)
}

pub fn roots(profile: &Path, scope: &ProjectScope) -> Vec<PathBuf> {
    scope.roots(&crate::extensions::roots(profile))
}

/// Captures the provider attribution from this window. Only the disk read
/// runs on a worker, so the list is tied to the same project as the sheet.
#[derive(Clone, Default)]
pub struct Inventory {
    owners: std::collections::BTreeMap<(String, String), String>,
    unavailable: std::collections::BTreeSet<String>,
    roots: Vec<PathBuf>,
    _scope: ProjectScope,
}
impl Inventory {
    pub fn new(controller: &crate::session::Controller, snapshot: &pm_kernel::Snapshot) -> Self {
        let state = controller.kernel.state();
        let (mut owners, mut unavailable) = pm_mcp::project_extensions::catalog(&state);
        if let Some(host) = &controller.extensions.host {
            let published = host.publication();
            for (id, _, _) in &published.unavailable {
                unavailable.insert(id.clone());
            }
            if let Some(value) = &published.contributions {
                for (group, kind) in [
                    ("effects", "effect"),
                    ("transitions", "transition"),
                    ("layers", "layer"),
                ] {
                    for item in value[group].as_array().into_iter().flatten() {
                        if let (Some(id), Some(owner)) =
                            (item["id"].as_str(), item["owner"].as_str())
                        {
                            owners.insert((kind.into(), id.into()), owner.into());
                            unavailable.remove(owner);
                        }
                    }
                }
            }
        }
        if let Some(runtime) = &controller.builtins.runtime {
            for (owner, definition) in runtime.extension_effects() {
                owners.insert(
                    ("effect".into(), definition.id.as_str().into()),
                    owner.clone(),
                );
                unavailable.remove(&owner);
            }
        }
        let scope = controller
            .project_extension_scope
            .as_ref()
            .filter(|(document, _)| *document == snapshot.doc_id)
            .map(|(_, scope)| scope.clone())
            .unwrap_or_else(|| ProjectScope {
                excluded: controller.mcp.extension_exclusions(snapshot.doc_id),
                ..Default::default()
            });
        unavailable.extend(scope.excluded.iter().cloned());
        Self {
            owners,
            unavailable,
            roots: roots(&controller.profile, &scope),
            _scope: scope,
        }
    }

    pub fn collect(&self, snapshot: &pm_kernel::Snapshot, include: bool) -> Bundle {
        let Some(root) = snapshot.root() else {
            return Bundle::default();
        };
        let mut extensions = installed(&self.roots);
        for found in pm_compat_runner::discovery::scan(&self.roots) {
            if let Some(extension) = extensions.get_mut(&found.id) {
                extension.manifest = found.manifest.ok();
            }
        }
        Bundle::collect(
            pm_mcp::project_extensions::references(root),
            &extensions,
            &self.owners,
            builtin_definitions(),
            &self.unavailable,
            include,
        )
    }
}

fn builtin_definitions() -> &'static std::collections::BTreeSet<(String, String)> {
    use pm_protocol::definitions::DefinitionKind;
    static SET: std::sync::LazyLock<std::collections::BTreeSet<(String, String)>> =
        std::sync::LazyLock::new(|| {
            pm_render::effects::versions::builtin_snapshot()
                .definitions
                .into_iter()
                .map(|item| {
                    let kind = match item.kind {
                        DefinitionKind::Effect => "effect",
                        DefinitionKind::Transition => "transition",
                        DefinitionKind::Layer => "layer",
                    };
                    (kind.into(), item.id.as_str().into())
                })
                .collect()
        });
    &SET
}

#[cfg(test)]
mod tests;
