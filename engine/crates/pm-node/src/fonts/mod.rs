//! The interactive engine's font service (plan section 12; Phase 2 "Font
//! registry and subsetting move into the engine").
//!
//! With "Use New Engine" on, the native font registry
//! (`pm_text::registry::CoreTextRegistry`) lives in the interactive engine
//! instead of Electron main. It
//!
//! - answers `pm-wasm`'s registry queries and produces the metrics-and-
//!   shaping subsets the renderer lays text out over. Main relays them to
//!   the renderer (section 12: "Main serves subsets to `pm-wasm` in both
//!   cases") over a dedicated font port ([`wire`]); main keeps only the
//!   project fonts it must replay to a restarted engine;
//! - holds project fonts and variable-font sources;
//! - lays text out natively for the engine's own rendering
//!   ([`FontService::line_runs`]) over the same registry, with the full font
//!   bytes ([`FontService::face`]) for outlines, so engine and `pm-wasm`
//!   layouts agree glyph for glyph.
//!
//! With the flag off, main's own registry (`pm-bridge`'s `FontRegistry`)
//! serves the renderer as before.
//!
//! The registry uses AppKit's shared font manager, so the service is created
//! and called on the utility process's JavaScript thread, which is the
//! process's main thread.

pub mod hub;
pub mod render;
pub mod wire;

use serde::Deserialize;
use wire::{Op, Status};

/// Largest JSON argument a request may carry. Main limits a renderer's
/// queries to 4 Mi UTF-16 code units before wrapping them, which is at most
/// 12 MiB of UTF-8.
pub const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;
/// Largest font a request may carry (large CJK collections run to about 70 MB).
pub const MAX_FONT_BYTES: usize = 128 * 1024 * 1024;
/// Longest family name.
pub const MAX_FAMILY_CHARS: usize = 512;
/// Longest `FontFace` descriptor string.
pub const MAX_DESCRIPTOR_CHARS: usize = 128;
/// Largest face index in a collection.
pub const MAX_FACE_INDEX: u32 = 0xffff;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenArgs {
    generation_base: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerArgs {
    queries: Vec<pm_text::answers::Query>,
    #[serde(default)]
    known: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubsetArgs {
    blob: String,
    index: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestArgs {
    base: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyArgs {
    family: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFontArgs {
    family: String,
    #[serde(default)]
    weight: Option<String>,
    #[serde(default)]
    style: Option<String>,
    #[serde(default)]
    stretch: Option<String>,
    #[serde(default)]
    index: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedsArgs {
    /// The seeds of work no window is known for.
    seeds: Vec<pm_text::compat::FontCacheSeed>,
    /// Each window's own seeds.
    #[serde(default)]
    owners: Vec<OwnerSeedsArgs>,
    /// The window each session main opened is for (its exports).
    #[serde(default)]
    sessions: Vec<SessionOwnerArgs>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerSeedsArgs {
    owner: u32,
    seeds: Vec<pm_text::compat::FontCacheSeed>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionOwnerArgs {
    session: u64,
    owner: u32,
}

/// Most seeds a message may carry per window (the app sends a few per
/// scale factor).
pub const MAX_SEEDS: usize = 256;
/// Most windows and sessions a seeds message may name.
pub const MAX_SEED_OWNERS: usize = 256;
pub const MAX_SEED_SESSIONS: usize = 4096;

/// The editor renderers' font cache seeds: each window is its own renderer
/// with its own font cache, so a layout measures with the seeds of the
/// window it is drawn for (`docs/engine/text.md`, "Seeds per window").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeedTable {
    /// Work no window is known for (every window's seeds, as main sends
    /// them).
    pub fallback: Vec<pm_text::compat::FontCacheSeed>,
    /// Each window's seeds, by its owner id (main's webContents id).
    pub owners: std::collections::BTreeMap<u32, Vec<pm_text::compat::FontCacheSeed>>,
    /// The window each session main opened is for.
    pub sessions: std::collections::BTreeMap<u64, u32>,
}

impl SeedTable {
    /// The seeds of the window `owner`, or the fallback when main named no
    /// such window.
    pub fn for_owner(&self, owner: Option<u32>) -> &[pm_text::compat::FontCacheSeed] {
        owner
            .and_then(|o| self.owners.get(&o))
            .unwrap_or(&self.fallback)
    }

    /// The seeds of work on a port of window `port_owner` (when the port
    /// belongs to one) or in `session`.
    pub fn resolve(
        &self,
        port_owner: Option<u32>,
        session: Option<u64>,
    ) -> &[pm_text::compat::FontCacheSeed] {
        let owner = port_owner.or_else(|| session.and_then(|s| self.sessions.get(&s).copied()));
        self.for_owner(owner)
    }
}

fn seed_table(args: SeedsArgs) -> Result<SeedTable, String> {
    if args.owners.len() > MAX_SEED_OWNERS || args.sessions.len() > MAX_SEED_SESSIONS {
        return Err("too many font cache seed owners".into());
    }
    let mut table = SeedTable {
        fallback: seeds(args.seeds)?,
        ..SeedTable::default()
    };
    for o in args.owners {
        table.owners.insert(o.owner, seeds(o.seeds)?);
    }
    for s in args.sessions {
        table.sessions.insert(s.session, s.owner);
    }
    Ok(table)
}

/// Checks seeds main sent: a family name within the limits and finite,
/// positive sizes and weights.
fn seeds(
    list: Vec<pm_text::compat::FontCacheSeed>,
) -> Result<Vec<pm_text::compat::FontCacheSeed>, String> {
    if list.len() > MAX_SEEDS {
        return Err("too many font cache seeds".into());
    }
    for s in &list {
        family(&s.family)?;
        let ok = |v: f32| v.is_finite() && v > 0.0;
        if !(ok(s.size) && ok(s.specified) && ok(s.weight)) {
            return Err("invalid font cache seed".into());
        }
    }
    Ok(list)
}

fn parse<'a, T: Deserialize<'a>>(json: &'a str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|e| format!("invalid font request arguments: {e}"))
}

fn family(name: &str) -> Result<&str, String> {
    if name.is_empty() || name.chars().count() > MAX_FAMILY_CHARS {
        return Err("invalid family".into());
    }
    Ok(name)
}

fn blob_key(key: &str) -> Result<&str, String> {
    if key.len() == 32
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(key)
    } else {
        Err("invalid subset key".into())
    }
}

fn face_index(index: u32) -> Result<u32, String> {
    if index <= MAX_FACE_INDEX {
        Ok(index)
    } else {
        Err("invalid face index".into())
    }
}

fn font_bytes(tail: &[u8]) -> Result<&[u8], String> {
    if tail.is_empty() || tail.len() > MAX_FONT_BYTES {
        return Err("invalid font data".into());
    }
    Ok(tail)
}

fn descriptor(value: &Option<String>) -> Result<&str, String> {
    match value {
        Some(v) if v.chars().count() > MAX_DESCRIPTOR_CHARS => {
            Err("invalid font descriptor".into())
        }
        Some(v) => Ok(v),
        None => Ok(""),
    }
}

/// What a request produced.
enum Answer {
    Json(String),
    Bytes(Vec<u8>),
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Answer, String> {
    serde_json::to_string(value)
        .map(Answer::Json)
        .map_err(|e| e.to_string())
}

fn generation_json(generation: u64) -> Answer {
    Answer::Json(format!("{{\"generation\":{generation}}}"))
}

/// The registry behind the service: the CoreText registry on macOS.
#[cfg(target_os = "macos")]
type Registry = pm_text::registry::CoreTextRegistry;

/// The font service of one engine launch.
#[cfg(target_os = "macos")]
pub struct FontService {
    layout: pm_text::compat::Compat<Registry>,
    generation_base: u64,
    /// The editor renderers' font cache seeds, as main last sent them.
    seeds: SeedTable,
    /// Main finished replaying to a port (its seeds come last, after the
    /// open and the project fonts).
    opened: bool,
}

#[cfg(target_os = "macos")]
impl FontService {
    /// Creates the registry. Fails off the main thread (AppKit).
    pub fn new() -> Result<Self, String> {
        let registry =
            Registry::try_new().ok_or("the font registry must be created on the main thread")?;
        Ok(FontService {
            layout: pm_text::compat::Compat::new(registry),
            generation_base: 0,
            seeds: SeedTable::default(),
            opened: false,
        })
    }

    fn registry(&mut self) -> &mut Registry {
        self.layout.resolver_mut()
    }

    /// The registry generation as main and the renderer see it: the launch's
    /// base plus the registry's own count of project font changes.
    pub fn generation(&self) -> u64 {
        self.generation_base + self.layout.resolver().generation()
    }

    /// Retained registry payload lengths, excluding CoreText's opaque caches.
    pub fn memory_usage(&self) -> pm_text::registry::FontMemoryUsage {
        self.layout.resolver().memory_usage()
    }

    /// Handles one message from the font port and returns the reply.
    pub fn handle(&mut self, bytes: &[u8]) -> Vec<u8> {
        let request = match wire::decode_request(bytes) {
            Ok(r) => r,
            Err(e) => {
                return wire::encode_reply(e.id.unwrap_or(0), Status::Error, e.reason.as_bytes());
            }
        };
        match self.run(request.op, request.json, request.tail) {
            Ok(Answer::Json(json)) => wire::encode_reply(request.id, Status::Json, json.as_bytes()),
            Ok(Answer::Bytes(bytes)) => wire::encode_reply(request.id, Status::Bytes, &bytes),
            Err(message) => wire::encode_reply(request.id, Status::Error, message.as_bytes()),
        }
    }

    fn run(&mut self, op: Op, json: &str, tail: &[u8]) -> Result<Answer, String> {
        if json.len() > MAX_JSON_BYTES {
            return Err("font request arguments are too large".into());
        }
        match op {
            Op::Open => {
                let args: OpenArgs = parse(json)?;
                self.generation_base = args.generation_base;
                Ok(generation_json(self.generation()))
            }
            Op::Seeds => {
                let args: SeedsArgs = parse(json)?;
                self.seeds = seed_table(args)?;
                self.opened = true;
                Ok(generation_json(self.generation()))
            }
            Op::Answer => {
                let args: AnswerArgs = parse(json)?;
                let generation = self.generation();
                let queries = if args.known.is_some_and(|k| k != generation) {
                    // The requester's answers predate this registry (an
                    // earlier launch, or project fonts changed): skip queries
                    // naming faces this registry never handed out. The new
                    // generation makes the renderer start again from empty
                    // answers, so it never asks for them again.
                    let known = self.layout.resolver().faces().count() as u32;
                    args.queries
                        .into_iter()
                        .filter(|q| names_known_faces(q, known))
                        .collect()
                } else {
                    args.queries
                };
                let answers = self
                    .registry()
                    .answer(&queries)
                    .map_err(|e| e.to_string())?;
                // The generation goes last: main reads it off the end.
                answers
                    .reply_json(generation)
                    .map(Answer::Json)
                    .map_err(|e| e.to_string())
            }
            Op::Subset => {
                let args: SubsetArgs = parse(json)?;
                let bytes = self
                    .registry()
                    .subset(blob_key(&args.blob)?, face_index(args.index)?)
                    .map_err(|e| e.to_string())?;
                Ok(Answer::Bytes(bytes.as_ref().clone()))
            }
            Op::Manifest => {
                let args: ManifestArgs = parse(json)?;
                let manifest = self.registry().fallback_manifest(family(&args.base)?);
                to_json(&manifest)
            }
            Op::FamilyFaces => {
                let args: FamilyArgs = parse(json)?;
                let faces = self.registry().family_faces(family(&args.family)?);
                to_json(&faces)
            }
            Op::AddProjectFont => {
                let args: ProjectFontArgs = parse(json)?;
                let descriptors = pm_text::registry::ProjectDescriptors::from_css(
                    descriptor(&args.weight)?,
                    descriptor(&args.style)?,
                    descriptor(&args.stretch)?,
                )
                .map_err(|e| e.to_string())?;
                let index = face_index(args.index.unwrap_or(0))?;
                let data = font_bytes(tail)?;
                self.registry()
                    .add_project_font(family(&args.family)?, data, index, descriptors)
                    .map_err(|e| e.to_string())?;
                Ok(generation_json(self.generation()))
            }
            Op::SetFontSource => {
                let args: FamilyArgs = parse(json)?;
                let data = font_bytes(tail)?;
                self.registry()
                    .set_font_source(family(&args.family)?, data)
                    .map_err(|e| e.to_string())?;
                Ok(generation_json(self.generation()))
            }
            Op::ClearProjectFonts => {
                let _: NoArgs = parse(json)?;
                self.registry().clear_project_fonts();
                Ok(generation_json(self.generation()))
            }
            Op::Generation => {
                let _: NoArgs = parse(json)?;
                Ok(generation_json(self.generation()))
            }
            Op::FamilyNames => {
                let _: NoArgs = parse(json)?;
                let names = self.layout.resolver().family_names();
                to_json(&names)
            }
        }
    }

    /// The editor renderers' font cache seeds main sent (`Op::Seeds`).
    pub fn seeds(&self) -> &SeedTable {
        &self.seeds
    }

    /// Whether main has finished its replay on a port: the open, the
    /// project fonts, then the seeds (`Op::Seeds` goes last), so render
    /// threads never lay text out before the fonts and seeds main is about
    /// to send.
    pub fn opened(&self) -> bool {
        self.opened
    }

    /// Answers a render thread's queries (`hub::FontHub::drain`): the
    /// registry's answers and the full bytes of every face they name.
    pub fn serve(
        &mut self,
        queries: &[pm_text::answers::Query],
    ) -> Result<pm_text::served::Served, String> {
        let generation = self.generation();
        let answers = self.registry().answer(queries).map_err(|e| e.to_string())?;
        let mut fonts: Vec<pm_text::served::ServedFont> = Vec::new();
        for rec in &answers.faces {
            if fonts
                .iter()
                .any(|f| f.blob == rec.blob && f.index == rec.index)
            {
                continue;
            }
            let info = self
                .face(rec.id)
                .ok_or_else(|| format!("unknown face {}", rec.id))?;
            fonts.push(pm_text::served::ServedFont {
                blob: rec.blob.clone(),
                index: rec.index,
                data: info.data.clone(),
                read_index: info.index,
            });
        }
        Ok(pm_text::served::Served {
            generation,
            answers,
            fonts,
        })
    }

    /// Level 1 glyph runs for the engine's own text rendering: the same
    /// layout `pm-wasm` computes over subsets, here over the full fonts.
    pub fn line_runs(
        &mut self,
        content: &pm_text::compat::TextContent,
    ) -> Vec<pm_text::compat::LineRuns> {
        self.layout.line_runs(content)
    }

    /// A face the layout named: its full font bytes (outlines for
    /// rendering), variation coordinates, and synthesis flags.
    pub fn face(&self, id: pm_text::font::FaceId) -> Option<&pm_text::font::FaceInfo> {
        use pm_text::font::FontResolver;
        let registry = self.layout.resolver();
        ((id as usize) < registry.faces().count()).then(|| registry.face(id))
    }
}

#[cfg(target_os = "macos")]
impl render::FontSource for FontService {
    fn generation(&self) -> u64 {
        FontService::generation(self)
    }

    fn answer(
        &mut self,
        queries: &[pm_text::answers::Query],
    ) -> Result<pm_text::answers::Answers, String> {
        self.registry().answer(queries).map_err(|e| e.to_string())
    }

    fn blob(&self, key: &str) -> Option<pm_text::font::FontBytes> {
        self.layout.resolver().blob(key)
    }
}

/// Whether every face a query names is one of the first `known` faces.
fn names_known_faces(query: &pm_text::answers::Query, known: u32) -> bool {
    use pm_text::answers::Query;
    match query {
        Query::Fallback { primary: id, .. } | Query::EmAdvance { face: id, .. } => *id < known,
        _ => true,
    }
}

/// Without a native registry (not macOS), every request is refused and the
/// renderer keeps the legacy text path.
#[cfg(not(target_os = "macos"))]
pub struct FontService;

#[cfg(not(target_os = "macos"))]
impl FontService {
    pub fn new() -> Result<Self, String> {
        Err("the native font registry is macOS only".into())
    }

    pub fn handle(&mut self, bytes: &[u8]) -> Vec<u8> {
        let id = wire::decode_request(bytes)
            .map(|r| r.id)
            .unwrap_or_else(|e| e.id.unwrap_or(0));
        wire::encode_reply(id, Status::Error, b"the native font registry is macOS only")
    }

    pub fn serve(
        &mut self,
        _queries: &[pm_text::answers::Query],
    ) -> Result<pm_text::served::Served, String> {
        Err("the native font registry is macOS only".into())
    }

    pub fn generation(&self) -> u64 {
        0
    }

    pub fn seeds(&self) -> &SeedTable {
        static NONE: std::sync::OnceLock<SeedTable> = std::sync::OnceLock::new();
        NONE.get_or_init(SeedTable::default)
    }

    pub fn opened(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_checks_mirror_mains() {
        assert!(family("").is_err());
        assert!(family(&"x".repeat(513)).is_err());
        assert_eq!(family("Brand Sans"), Ok("Brand Sans"));
        assert!(blob_key("0123456789abcdef0123456789abcdef").is_ok());
        assert!(blob_key("0123456789ABCDEF0123456789abcdef").is_err());
        assert!(blob_key("../etc").is_err());
        assert!(face_index(0x1_0000).is_err());
        assert!(font_bytes(&[]).is_err());
        assert!(descriptor(&Some("9".repeat(129))).is_err());
        assert_eq!(descriptor(&None), Ok(""));
    }

    #[test]
    fn stale_queries_are_recognized_by_face() {
        use pm_text::answers::Query;
        let fallback = Query::Fallback {
            weight: 0,
            italic: false,
            size: 0,
            ch: 0x4e00,
            primary: 7,
            emoji: false,
        };
        assert!(!names_known_faces(&fallback, 7));
        assert!(names_known_faces(&fallback, 8));
        assert!(names_known_faces(
            &Query::Variable { family: "X".into() },
            0
        ));
    }
}
