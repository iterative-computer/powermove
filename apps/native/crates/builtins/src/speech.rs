//! Settings › Transcription (S092) over the app's speech model store: every
//! catalog model with its facts, download progress and errors, the model in
//! use, the spoken language and the space models take. The app installs the
//! store once; each Settings host wakes when it changes.
use pm_transcribe::store::{Store, format_bytes};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicU64, Ordering},
};

static STORE: OnceLock<Store> = OnceLock::new();
static REVISION: AtomicU64 = AtomicU64::new(0);
type Wake = dyn Fn() + Send + Sync;
static WAKERS: Mutex<Vec<Weak<Wake>>> = Mutex::new(Vec::new());

/// The app's store (profile `models/transcription`), set once at launch.
pub fn install(store: Store) {
    let _ = STORE.set(store);
    changed();
}
pub fn store() -> Option<&'static Store> {
    STORE.get()
}
/// The store's change callback: every Settings host publishes again.
pub fn changed() {
    REVISION.fetch_add(1, Ordering::AcqRel);
    let mut wakers = WAKERS.lock().unwrap_or_else(|e| e.into_inner());
    wakers.retain(|w| w.strong_count() > 0);
    for wake in wakers.iter().filter_map(Weak::upgrade) {
        wake();
    }
}
pub(crate) fn watch(wake: &Arc<Wake>) {
    WAKERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Arc::downgrade(wake));
}
pub(crate) fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

/// A language tag's English name (the catalog's codes; others as given).
pub fn language_name(code: &str) -> String {
    const NAMES: &[(&str, &str)] = &[
        ("af", "Afrikaans"),
        ("am", "Amharic"),
        ("ar", "Arabic"),
        ("as", "Assamese"),
        ("az", "Azerbaijani"),
        ("ba", "Bashkir"),
        ("be", "Belarusian"),
        ("bg", "Bulgarian"),
        ("bn", "Bangla"),
        ("bo", "Tibetan"),
        ("br", "Breton"),
        ("bs", "Bosnian"),
        ("ca", "Catalan"),
        ("cs", "Czech"),
        ("cy", "Welsh"),
        ("da", "Danish"),
        ("de", "German"),
        ("el", "Greek"),
        ("en", "English"),
        ("es", "Spanish"),
        ("et", "Estonian"),
        ("eu", "Basque"),
        ("fa", "Persian"),
        ("fi", "Finnish"),
        ("fo", "Faroese"),
        ("fr", "French"),
        ("gl", "Galician"),
        ("gu", "Gujarati"),
        ("ha", "Hausa"),
        ("haw", "Hawaiian"),
        ("he", "Hebrew"),
        ("hi", "Hindi"),
        ("hr", "Croatian"),
        ("ht", "Haitian Creole"),
        ("hu", "Hungarian"),
        ("hy", "Armenian"),
        ("id", "Indonesian"),
        ("is", "Icelandic"),
        ("it", "Italian"),
        ("ja", "Japanese"),
        ("jw", "Javanese"),
        ("ka", "Georgian"),
        ("kk", "Kazakh"),
        ("km", "Khmer"),
        ("kn", "Kannada"),
        ("ko", "Korean"),
        ("la", "Latin"),
        ("lb", "Luxembourgish"),
        ("ln", "Lingala"),
        ("lo", "Lao"),
        ("lt", "Lithuanian"),
        ("lv", "Latvian"),
        ("mg", "Malagasy"),
        ("mi", "Māori"),
        ("mk", "Macedonian"),
        ("ml", "Malayalam"),
        ("mn", "Mongolian"),
        ("mr", "Marathi"),
        ("ms", "Malay"),
        ("mt", "Maltese"),
        ("my", "Burmese"),
        ("nb", "Norwegian Bokmål"),
        ("ne", "Nepali"),
        ("nl", "Dutch"),
        ("nn", "Norwegian Nynorsk"),
        ("no", "Norwegian"),
        ("oc", "Occitan"),
        ("pa", "Punjabi"),
        ("pl", "Polish"),
        ("ps", "Pashto"),
        ("pt", "Portuguese"),
        ("ro", "Romanian"),
        ("ru", "Russian"),
        ("sa", "Sanskrit"),
        ("sd", "Sindhi"),
        ("si", "Sinhala"),
        ("sk", "Slovak"),
        ("sl", "Slovenian"),
        ("sn", "Shona"),
        ("so", "Somali"),
        ("sq", "Albanian"),
        ("sr", "Serbian"),
        ("su", "Sundanese"),
        ("sv", "Swedish"),
        ("sw", "Swahili"),
        ("ta", "Tamil"),
        ("te", "Telugu"),
        ("tg", "Tajik"),
        ("th", "Thai"),
        ("tk", "Turkmen"),
        ("tl", "Filipino"),
        ("tr", "Turkish"),
        ("tt", "Tatar"),
        ("uk", "Ukrainian"),
        ("ur", "Urdu"),
        ("uz", "Uzbek"),
        ("vi", "Vietnamese"),
        ("yi", "Yiddish"),
        ("yo", "Yoruba"),
        ("zh", "Chinese"),
    ];
    NAMES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or_else(|| code.to_owned(), |(_, n)| (*n).to_owned())
}
/// "English only", or how many languages a model transcribes.
fn reach(model: &Value) -> String {
    if model["languages"] == "en" {
        return "English only".into();
    }
    match model["languageCodes"].as_array().map_or(0, Vec::len) {
        n if n > 1 => format!("{n} languages"),
        _ => "Many languages".into(),
    }
}
/// The facts line: size, languages, and when it can't make captions.
pub fn facts(model: &Value) -> String {
    let mut parts = vec![
        format_bytes(model["size"].as_u64().unwrap_or(0)),
        reach(model),
    ];
    if model["wordTimestamps"] != true {
        parts.push("Not for captions".into());
    }
    parts.join(" · ")
}
/// The Settings rows for each catalog model (`SpeechModel`), downloaded first.
pub(crate) fn models(status: &Value) -> Vec<Value> {
    let active = status["activeModelId"].as_str();
    let mut rows: Vec<(bool, Value)> = status["models"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let state = match m["state"].as_str() {
                Some("ready") => "installed",
                Some("downloading") => "downloading",
                Some("error") => "error",
                _ => "available",
            };
            let mut detail = facts(m);
            if state == "downloading" || m["downloadedBytes"].as_u64().is_some_and(|b| b > 0) {
                detail = format!(
                    "{} of {}",
                    format_bytes(m["downloadedBytes"].as_u64().unwrap_or(0)),
                    format_bytes(m["size"].as_u64().unwrap_or(0))
                );
            }
            let mut row = json!({"id":m["id"],"name":m["name"],"size":detail,"state":state,
                "inUse": active.is_some() && m["id"].as_str() == active,
                "recommended": m["recommended"] == true});
            if let Some(p) = m["progress"].as_f64() {
                row["progress"] = json!(p);
            }
            if state == "error" {
                row["error"] = json!(
                    m["error"]
                        .as_str()
                        .unwrap_or("The download failed. Try again.")
                );
            }
            (state == "installed", row)
        })
        .collect();
    rows.sort_by_key(|(installed, _)| !installed);
    rows.into_iter().map(|(_, r)| r).collect()
}
/// Spoken Language and Downloaded Models rows (generic Settings rows).
pub(crate) fn rows(status: &Value) -> Vec<Value> {
    let models = status["models"].as_array().cloned().unwrap_or_default();
    let active = models
        .iter()
        .find(|m| m["id"].as_str().is_some() && m["id"] == status["activeModelId"]);
    let language = status["language"].as_str().unwrap_or("auto");
    let mut codes: Vec<String> = active
        .and_then(|m| m["languageCodes"].as_array())
        .map(|c| {
            c.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    codes.sort_by_key(|c| language_name(c));
    let mut options = vec![json!({"v":"auto","label":"Detect Automatically"})];
    options.extend(
        codes
            .iter()
            .map(|c| json!({"v":c,"label":language_name(c)})),
    );
    if language != "auto" && !codes.iter().any(|c| c == language) {
        options.push(json!({"v":language,"label":language_name(language)}));
    }
    let detail = match active {
        Some(m) if m["detectsLanguage"] == false && m["languages"] != "en" => format!(
            "{} can’t detect the language, so it hears English until you choose another.",
            m["name"].as_str().unwrap_or("This model")
        ),
        _ => "Detected automatically. Choose one to label transcripts for captions and subtitles."
            .into(),
    };
    let storage = status["storageBytes"].as_u64().unwrap_or(0);
    vec![
        json!({"id":"speech.language","section":"transcription","group":"Language",
            "label":"Spoken Language","control":"choice","value":language,"options":options,
            "detail":detail,"disabled":active.is_none() || active.is_some_and(|m| m["languages"] == "en")}),
        json!({"id":"speech.storage","section":"transcription","group":"Storage",
            "label":"Downloaded Models","control":"status",
            "value": if storage > 0 { format!("{} in Powermove’s data folder.", format_bytes(storage)) } else { "No models on this Mac yet.".into() },
            "detail":"Transcription runs on this Mac and your audio is never uploaded."}),
    ]
}
/// The store's status, or `None` before the app installs it.
pub(crate) fn status() -> Option<Value> {
    store().map(Store::status)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_name_facts_progress_and_the_model_in_use() {
        let status = json!({"activeModelId":"a","language":"auto","storageBytes":731_000_000u64,"models":[
            {"id":"b","name":"B","size":218_000_000u64,"languages":"multi","languageCodes":["en","de"],"wordTimestamps":false,"state":"downloading","progress":0.5,"downloadedBytes":109_000_000u64},
            {"id":"a","name":"A","size":731_000_000u64,"languages":"en","languageCodes":["en"],"wordTimestamps":true,"state":"ready","detectsLanguage":false},
            {"id":"c","name":"C","size":1_770_000_000u64,"languages":"multi","languageCodes":["en"],"wordTimestamps":false,"state":"error","error":"Network down"}
        ]});
        let rows = models(&status);
        assert_eq!(rows[0]["id"], "a");
        assert_eq!(rows[0]["state"], "installed");
        assert_eq!(rows[0]["inUse"], true);
        assert_eq!(rows[0]["size"], "731 MB · English only");
        assert_eq!(rows[1]["size"], "109 MB of 218 MB");
        assert_eq!(rows[1]["progress"], 0.5);
        assert_eq!(rows[2]["error"], "Network down");
        assert_eq!(
            facts(&status["models"][2]),
            "1.8 GB · Many languages · Not for captions"
        );
        let settings = rows_of(&status);
        assert_eq!(settings[0]["disabled"], true);
        assert_eq!(settings[1]["value"], "731 MB in Powermove’s data folder.");
    }
    fn rows_of(status: &Value) -> Vec<Value> {
        rows(status)
    }
    #[test]
    fn languages_read_by_name() {
        assert_eq!(language_name("de"), "German");
        assert_eq!(language_name("xx"), "xx");
    }
}
