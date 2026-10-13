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
    let started = std::time::Instant::now();
    let manifest = r.fallback_manifest("system-ui");
    let built = started.elapsed();
    let started = std::time::Instant::now();
    check("manifest cached", r.fallback_manifest("system-ui") == manifest && started.elapsed() < built / 10, format!("{:?} then {:?}", built, started.elapsed()));
    println!("  manifest built in {built:?}, cached in {:?}", started.elapsed());
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

    // The incremental protocol main serves pm-wasm: a replay starts with no
    // answers and no fonts, reports misses and pending faces, main answers
    // them in batches and serves subsets, and the layout converges on the
    // native result.
    {
        use pm_text::answers::{Answers, Query, ReplayResolver};
        use pm_text::compat::{Compat, TextContent};
        use pm_text::corpus::{Case, run_case};
        use pm_text::font::FontBytes;
        let case = |id: &str, d: serde_json::Value| Case { id: id.into(), group: "protocol".into(), d: TextContent::from_json(&d) };
        let cases = [
            case("latin", serde_json::json!({ "text": "Hello, world. The quick brown fox", "font": "Helvetica Neue", "size": 48, "weight": 500, "boxWidth": 400 })),
            case("mixed", serde_json::json!({ "text": "Mixed 世界 مرحبا שלום 😀 नमस्ते", "font": "Avenir Next", "size": 32, "tracking": 4 })),
            case("system", serde_json::json!({ "text": "System UI, bold italic", "size": 24, "weight": 700, "italic": true })),
            case("variable", serde_json::json!({ "text": "Skia axes", "font": "Skia", "size": 40, "fontAxis.wght": 1.6 })),
        ];
        let mut native = Compat::new(CoreTextRegistry::new());
        let want: Vec<_> = cases.iter().map(|c| run_case(&mut native, c)).collect();

        let mut resolver = ReplayResolver::new(&Answers::default());
        let mut rounds = 0;
        let mut batches = Vec::new();
        let got = loop {
            rounds += 1;
            assert!(rounds <= 8, "the protocol did not converge");
            let mut compat = Compat::new(resolver);
            let out: Vec<_> = cases.iter().map(|c| run_case(&mut compat, c)).collect();
            resolver = compat.into_resolver();
            if resolver.misses.is_empty() && resolver.pending_blobs.is_empty() {
                break out;
            }
            let misses: Vec<Query> = resolver.misses.clone();
            let pending: Vec<(String, u32)> = resolver.pending_blobs.iter().cloned().collect();
            resolver.clear_reports();
            let reply = r.answer(&misses).expect("answers");
            batches.push((misses.len(), reply.faces.len(), pending.len()));
            resolver.add_answers(&reply);
            for (blob, index) in reply.sources().into_iter().chain(pending) {
                let subset = r.subset(&blob, index).expect("subset");
                resolver.add_font(&blob, index, FontBytes::new(subset.to_vec()), 0);
            }
        };
        check("incremental protocol converges on the native layout", got == want, format!("{batches:?}"));
        println!("  protocol: {} rounds, batches (misses, faces, pending) {batches:?}", rounds);
        let bogus = Query::Fallback { weight: 0, italic: false, size: 0, ch: 0x4e00, primary: u32::MAX, emoji: false };
        check("answers refuse unknown faces", r.answer(&[bogus]).is_err(), String::new());
    }
    println!("{checks} checks passed");
}

#[cfg(not(all(target_os = "macos", feature = "system")))]
fn main() {
    println!("the registry test needs the macOS CoreText registry; skipped");
}
