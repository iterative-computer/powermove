//! The two shipped starter extensions, installed into the current profile.
use crate::{
    dialogs::{DialogId, Field, NativePiece, Surface, WindowId, sheet_layout::native::colour},
    sheets::{OpenSheet, SheetParts},
};
use gpui::App;
use objc2::{AnyThread, runtime::AnyObject};
use objc2_app_kit::{NSBezelStyle, NSButton, NSColor, NSForegroundColorAttributeName, NSWindow};
use objc2_foundation::{NSAttributedString, NSDictionary, NSString};
use pm_compat_runner::manifest;
use pm_design::Tokens;
use serde_json::{Value, json};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Panel,
    Effect,
}

const PANEL_MANIFEST: &str =
    include_str!("../../../js/v4/api-pack/samples/hello-panel/manifest.json");
const PANEL_SOURCE: &str = include_str!("../../../js/v4/api-pack/samples/hello-panel/index.tsx");
const EFFECT_MANIFEST: &str =
    include_str!("../../../js/compat/api-pack/samples/gradient-tint/manifest.json");
const EFFECT_SOURCE: &str =
    include_str!("../../../js/compat/api-pack/samples/gradient-tint/index.ts");

fn folder_stem(name: &str) -> String {
    let words = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty());
    let mut stem = words.collect::<Vec<_>>().join("-").to_ascii_lowercase();
    if stem.len() < 2 {
        return "extension".into();
    }
    if stem.as_bytes()[0].is_ascii_digit() {
        stem.insert_str(0, "extension-");
    }
    stem[..stem.len().min(53)].trim_end_matches('-').into()
}

pub(crate) fn create(profile: &Path, name: &str, kind: Kind) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.encode_utf16().count() > 80 {
        return Err("Enter a name between 1 and 80 characters.".into());
    }
    let (raw, entry, source) = match kind {
        Kind::Panel => (PANEL_MANIFEST, "index.tsx", PANEL_SOURCE),
        Kind::Effect => (EFFECT_MANIFEST, "index.ts", EFFECT_SOURCE),
    };
    let mut value: Value = serde_json::from_str(raw).expect("shipped sample manifest");
    value["name"] = json!(name);
    value["author"] = json!("user");
    value["entry"] = json!(entry);

    let root = profile.join("extensions");
    let failed =
        |error: std::io::Error| format!("Couldn’t save the extension in this profile. {error}");
    fs::create_dir_all(&root).map_err(failed)?;
    let stem = folder_stem(name);
    for number in 1.. {
        let id = if number == 1 {
            stem.clone()
        } else {
            format!("{stem}-{number}")
        };
        let folder = root.join(&id);
        match fs::create_dir(&folder) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(failed(error)),
        }
        value["id"] = json!(id);
        let source = match kind {
            Kind::Panel => source.to_owned(),
            Kind::Effect => source
                .replace("'gradient-tint.effect'", &format!("'{id}.effect'"))
                .replace("'Gradient Tint'", &json!(name).to_string()),
        };
        let result = write_template(&folder, &value, entry, &source);
        if result.is_err() {
            let _ = fs::remove_dir_all(&folder);
        }
        result?;
        return Ok(id);
    }
    unreachable!()
}

fn write_template(folder: &Path, value: &Value, entry: &str, source: &str) -> Result<(), String> {
    manifest::parse(value)?;
    let bytes = serde_json::to_vec_pretty(value).expect("serializable manifest");
    // Publish manifest last. Discovery can only see complete starter folders.
    let result = (|| {
        fs::write(folder.join(entry), source)?;
        let mut file = tempfile::NamedTempFile::new_in(folder)?;
        file.write_all(&bytes)?;
        file.persist_noclobber(folder.join("manifest.json"))?;
        Ok::<_, std::io::Error>(())
    })();
    result.map_err(|error| format!("Couldn’t write the starter files. {error}"))
}

pub(crate) fn surface() -> Surface {
    let name = Field {
        placeholder: Some("My Extension"),
        ..Field::new("name", "Name", NativePiece::TextField, vec![String::new()])
    };
    let kind = Field::new(
        "kind",
        "Kind",
        NativePiece::PopUpButton,
        vec!["Panel".into(), "Effect".into()],
    );
    let mut surface = Surface::sheet(
        DialogId("new-extension".into()),
        WindowId("extension-owner".into()),
        "New Extension",
        vec![name, kind],
        "Create",
    );
    surface.buttons[1].enabled = false;
    surface
}

pub(crate) fn native_sheet(tokens: &Tokens) -> SheetParts {
    let parts = crate::sheets::from_surface(&surface(), tokens);
    parts.0.buttons["apply"].setBezelStyle(NSBezelStyle::Push);
    parts
}

pub(crate) fn set_create_enabled(button: &NSButton, enabled: bool, tokens: &Tokens) {
    button.setEnabled(enabled);
    // AppKit mutes the default bezel when the sheet is inactive. Keep the same
    // native button and give its enabled state the accent and white title.
    button.setBordered(!enabled);
    button.setWantsLayer(true);
    if let Some(layer) = button.layer() {
        layer.setCornerRadius(tokens.metrics.control_radius.into());
        let accent = enabled.then(|| colour(tokens.colors.accent).CGColor());
        layer.setBackgroundColor(accent.as_deref());
    }
    let title = button.title();
    if enabled {
        let white = NSColor::whiteColor();
        let attrs = NSDictionary::from_slices(
            &[unsafe { NSForegroundColorAttributeName }],
            &[&*white as &AnyObject],
        );
        let label = unsafe {
            NSAttributedString::initWithString_attributes(
                NSAttributedString::alloc(),
                &title,
                Some(&attrs),
            )
        };
        button.setAttributedTitle(&label);
    } else {
        button.setTitle(&title);
    }
}

fn name_text(sheet: &OpenSheet) -> String {
    sheet
        .sheet
        .fields
        .get("name")
        .and_then(|field| field.currentEditor())
        .map_or_else(|| sheet.text("name"), |editor| editor.string().to_string())
}

pub(crate) fn open(
    profile: PathBuf,
    owner: &NSWindow,
    tokens: &Tokens,
    cx: &mut App,
    created: impl FnOnce(&mut App) + 'static,
) {
    if owner.attachedSheet().is_some() {
        return;
    }
    let sheet = OpenSheet::present(native_sheet(tokens), owner, Some("name"));
    sheet.use_sheet_keys();
    let tokens = tokens.clone();
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        while let Ok(action) = sheet.actions.recv().await {
            match action.as_str() {
                "cancel" => break,
                "changed:name" => {
                    sheet.sheet.set_footer_message("", tokens.colors.warning);
                    set_create_enabled(
                        &sheet.sheet.buttons["apply"],
                        !name_text(&sheet).trim().is_empty(),
                        &tokens,
                    );
                }
                "apply" => {
                    let name = name_text(&sheet);
                    if name.trim().is_empty() {
                        continue;
                    }
                    let kind = if sheet.popup("kind") == Some(1) {
                        Kind::Effect
                    } else {
                        Kind::Panel
                    };
                    let profile = profile.clone();
                    match executor
                        .spawn(async move { create(&profile, &name, kind) })
                        .await
                    {
                        Ok(_) => {
                            sheet.close();
                            pm_compat_runner::host::rescan_all();
                            cx.update(created);
                            return;
                        }
                        Err(error) => sheet.sheet.set_footer_message(&error, tokens.colors.warning),
                    }
                }
                _ => {}
            }
        }
        sheet.close();
    })
    .detach();
}
