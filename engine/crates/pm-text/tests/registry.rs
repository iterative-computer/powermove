//! The native font registry: enumeration, project fonts, variable sources,
//! the fallback manifest, family faces, and subsets with the amended table
//! list. Runs without the test harness so the registry lives on the main
//! thread (AppKit's font manager).

#[cfg(all(target_os = "macos", feature = "system"))]
fn main() {
    use pm_text::font::{FontDesc, FontResolver};
    use pm_text::registry::{CoreTextRegistry, ProjectDescriptors, ProjectStyle, content_key};
    use read_fonts::types::Tag;
    use read_fonts::{FontRef, TableProvider};

    let desc = FontDesc { weight: 400.0, italic: false, size: 16.0 };
    let mut r = CoreTextRegistry::new();
    let mut checks = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        assert!(ok, "{name}: {detail}");
        println!("ok {name}");
        checks += 1;
    };

    // Enumeration.
    let names = r.family_names();
    check("installed families", names.iter().any(|n| n == "Helvetica Neue") && names.iter().any(|n| n == "Georgia"), format!("{} families", names.len()));
    let info = r.family_info("Helvetica Neue").expect("Helvetica Neue");
    check(
        "family styles",
        info.styles.len() >= 8 && info.styles.windows(2).all(|w| w[0].weight <= w[1].weight) && !info.project,
        format!("{:?}", info.styles.iter().map(|s| (&s.name, s.weight)).collect::<Vec<_>>()),
    );
    check("variable family", r.family_info("Skia").is_some_and(|i| i.variable), String::new());
    check("static family", r.family_info("Georgia").is_some_and(|i| !i.variable), String::new());

    // Project fonts shadow installed families and leave with the project.
    let georgia = std::fs::read("/System/Library/Fonts/Supplemental/Georgia.ttf").unwrap();
    let installed = r.family("Chalkduster", &desc).expect("Chalkduster is installed");
    let generation = r.generation();
    r.add_project_font("Chalkduster", &georgia, 0, ProjectDescriptors { weight: (400.0, 400.0), style: ProjectStyle::Normal, ..Default::default() }).unwrap();
    let shadow = r.family("Chalkduster", &desc).unwrap();
    check("project font shadows", shadow != installed && r.face(shadow).custom && r.face(shadow).postscript_name == "Georgia", r.face(shadow).postscript_name.clone());
    check("generation bumps", r.generation() > generation, String::new());
    let bold = r.family("Chalkduster", &FontDesc { weight: 700.0, ..desc }).unwrap();
    check("synthetic bold for a lighter project face", r.face(bold).synthetic_bold, String::new());
    check("project families listed", r.family_info("Chalkduster").is_some_and(|i| i.project), String::new());
    r.clear_project_fonts();
    check("cleared project fonts", r.family("Chalkduster", &desc) == Some(installed), String::new());
    check("garbage is not a font", r.add_project_font("Bad", b"not a font", 0, ProjectDescriptors::default()).is_err(), String::new());

    // Variable sources resolve as inspectFont does.
    for (family, path) in [("Skia", "/System/Library/Fonts/Supplemental/Skia.ttf"), ("STIX Two Text", "/System/Library/Fonts/Supplemental/STIXTwoText.ttf")] {
        let face = r.variable_face(family).unwrap_or_else(|| panic!("{family} has a variable source"));
        let want = content_key(&pm_text::sanitize::sanitize(&std::fs::read(path).unwrap(), 0).unwrap());
        check(&format!("variable source {family}"), r.face(face).blob == want && r.face(face).custom, r.face(face).postscript_name.clone());
    }
    check("static family has no variable source", r.variable_face("Georgia").is_none(), String::new());
    let system = r.variable_face("System Font");
    println!("  System Font variable source: {:?}", system.map(|f| r.face(f).postscript_name.clone()));

    // Fallback manifest.
    let manifest = r.fallback_manifest("system-ui");
    let has = |script: &str| manifest.scripts.iter().any(|s| s.script == script && !s.families.is_empty());
    check("manifest scripts", has("Hani") && has("Arab") && has("Hebr") && has("Deva") && has("Thai") && has("Hang"), format!("{manifest:?}"));
    check("manifest emoji", manifest.emoji.as_deref() == Some("Apple Color Emoji"), format!("{:?}", manifest.emoji));
    check("manifest base covers Latin", manifest.scripts.iter().any(|s| s.script == "Latn" && s.families == ["system-ui"]), String::new());
    let named: std::collections::BTreeSet<&str> = manifest.faces.iter().map(|f| f.family.as_str()).collect();
    let every_family_has_faces = manifest.scripts.iter().flat_map(|s| &s.families).all(|f| named.contains(f.as_str())) && named.contains("Apple Color Emoji");
    check("manifest faces", every_family_has_faces, format!("{named:?}"));
    println!("  manifest: {} scripts, {} faces", manifest.scripts.len(), manifest.faces.len());
    for s in manifest.scripts.iter().filter(|s| ["Hani", "Zyyy"].contains(&s.script.as_str())) {
        println!("    {} {:?}: {:?}", s.script, s.language, s.families);
    }

    // Family faces for level 2.
    let faces = r.family_faces("Georgia");
    check("family faces", faces.len() == 4, format!("{faces:?}"));
    let system_faces = r.family_faces("system-ui");
    check("hidden system family faces", !system_faces.is_empty(), format!("{system_faces:?}"));

    // Subsets carry the amended table list and nothing else.
    let allowed = pm_text::subset::TABLES;
    for (family, must) in [("Helvetica Neue", &[b"feat", b"morx", b"maxp"][..]), ("system-ui", &[b"STAT", b"trak", b"maxp"][..]), ("Skia", &[b"HVAR", b"fvar", b"maxp"][..])] {
        let face = r.family(family, &desc).unwrap();
        let (blob, index) = (r.face(face).blob.clone(), r.face(face).index);
        let source = r.blob(&blob).unwrap();
        let subset = r.subset(&blob, index).unwrap();
        let font = FontRef::new(&subset).unwrap();
        let tags: Vec<[u8; 4]> = font.table_directory().table_records().iter().map(|t| t.tag().to_be_bytes()).collect();
        let missing: Vec<String> = must.iter().filter(|t| !tags.contains(t)).map(|t| String::from_utf8_lossy(&t[..]).into_owned()).collect();
        let extra: Vec<String> = tags.iter().filter(|t| !allowed.contains(t)).map(|t| String::from_utf8_lossy(&t[..]).into_owned()).collect();
        check(
            &format!("subset {family}"),
            missing.is_empty() && extra.is_empty() && subset.len() < source.as_slice().len() && font.table_data(Tag::new(b"glyf")).is_none(),
            format!("missing {missing:?} extra {extra:?}, {} of {} bytes", subset.len(), source.as_slice().len()),
        );
        let _ = font.maxp().unwrap();
        println!("  {family}: subset {} bytes from {} ({} tables)", subset.len(), source.as_slice().len(), tags.len());
    }
    check("unknown blob", r.subset("nope", 0).is_err(), String::new());
    println!("{checks} checks passed");
}

#[cfg(not(all(target_os = "macos", feature = "system")))]
fn main() {
    println!("the registry test needs the macOS CoreText registry; skipped");
}
