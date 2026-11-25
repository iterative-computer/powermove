//! Hard crashes (a segfault, an abort, a stack overflow) end the process
//! before anything can be sent. macOS writes a crash report for each
//! (`~/Library/Logs/DiagnosticReports/*.ips`). At the next launch, while
//! sharing is on, recent reports of this bundle are read (bounded: newest
//! [`MAX_SCAN`] files of the last week, each at most [`MAX_FILE_BYTES`]) and a
//! report is sent only when it belongs to a recorded Powermove session that
//! was sending at the time (same process id and launch time): a crash of an
//! MCP or engine process, a development run or an opted-out session never
//! leaves. Each report is keyed by its incident id (the event uuid derives
//! from it), so it is sent once even if the file is touched.
//!
//! What leaves: the exception and signal from a fixed list, the crashed
//! thread's frames in our own executable (offsets and our symbols), our
//! binary's UUID, and the app version when it is plain numbers. Nothing else
//! from the report: no subtype, termination reason, application text, paths
//! or other binaries.
use super::exception::{Frame, Image, Kind, crash_type, valid_uuid};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const MAX_SCAN: usize = 64;
pub const MAX_FILE_BYTES: u64 = 4 << 20;
const LOOKBACK_S: f64 = 7. * 86_400.;
const MAX_FRAMES: usize = 128;

pub struct Report {
    /// The report's incident id (a UUID, uppercase).
    pub incident: String,
    pub pid: u64,
    /// When the crashed process started (seconds since 1970).
    pub launched: f64,
    /// When it crashed.
    pub time: f64,
    pub properties: Map<String, Value>,
}

pub fn reports_folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Logs/DiagnosticReports"))
}

/// Report files worth reading, oldest first: `.ips`, from the last week, not
/// over the size limit, the newest [`MAX_SCAN`] of them.
pub fn candidates(folder: &Path, now: f64) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return vec![];
    };
    let mut found: Vec<(f64, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ips"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let time = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs_f64();
            (meta.len() <= MAX_FILE_BYTES && now - time <= LOOKBACK_S).then(|| (time, e.path()))
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    let skip = found.len().saturating_sub(MAX_SCAN);
    found.into_iter().skip(skip).map(|(_, p)| p).collect()
}

/// Reads one candidate (size checked again at read).
pub fn read(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= MAX_FILE_BYTES).then_some(text)
}

fn time(text: Option<&str>) -> Option<f64> {
    let t = chrono::DateTime::parse_from_str(text?, "%Y-%m-%d %H:%M:%S%.f %z").ok()?;
    Some(t.timestamp_millis() as f64 / 1000.)
}

/// The report, if it is `bundle_id`'s and well formed.
pub fn parse(text: &str, bundle_id: &str) -> Option<Report> {
    let (header, body) = text.split_once('\n')?;
    let header: Value = serde_json::from_str(header).ok()?;
    if header["bundleID"].as_str() != Some(bundle_id) {
        return None;
    }
    let incident = header["incident_id"].as_str()?.to_uppercase();
    if !valid_uuid(&incident) {
        return None;
    }
    let body: Value = serde_json::from_str(body).ok()?;
    let pid = body["pid"].as_u64()?;
    let launched = time(body["procLaunch"].as_str())?;
    let crashed = time(body["captureTime"].as_str()).unwrap_or(launched);
    // Our executable: the image whose UUID is the report's own slice.
    let slice = header["slice_uuid"].as_str()?.to_uppercase();
    let images = body["usedImages"].as_array()?;
    let (index, main) = images.iter().enumerate().find_map(|(i, image)| {
        let uuid = image["uuid"].as_str()?.to_uppercase();
        (uuid == slice && valid_uuid(&uuid)).then(|| {
            (
                i as u64,
                Image {
                    base: image["base"].as_u64().unwrap_or(0),
                    size: image["size"].as_u64().unwrap_or(0),
                    uuid,
                },
            )
        })
    })?;
    let faulting = body["faultingThread"].as_u64().unwrap_or(0) as usize;
    let frames: Vec<Frame> = body["threads"]
        .get(faulting)
        .and_then(|t| t["frames"].as_array())
        .map(|frames| {
            frames
                .iter()
                .take(MAX_FRAMES)
                .map(|f| {
                    let ours = f["imageIndex"].as_u64() == Some(index);
                    let addr = f["imageOffset"]
                        .as_u64()
                        .filter(|offset| ours && *offset < main.size)
                        .and_then(|offset| main.base.checked_add(offset));
                    match addr {
                        Some(addr) => Frame {
                            addr,
                            function: f["symbol"].as_str().map(str::to_owned),
                        },
                        None => Frame::default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let exception = &body["exception"];
    let mut properties = super::exception::properties(
        Kind {
            exception_type: &crash_type(exception["type"].as_str(), exception["signal"].as_str()),
            value: "crash".into(),
            mechanism: "crash",
        },
        &frames,
        Some(&main),
    );
    properties.insert("crash".into(), true.into());
    if let Some(version) = header["app_version"].as_str().filter(|v| plain_version(v)) {
        properties.insert("crashed_app_version".into(), version.into());
    }
    Some(Report {
        incident,
        pid,
        launched,
        time: crashed,
        properties,
    })
}

/// `1`, `1.2`, `1.2.3` or `1.2.3.4`, numbers of at most four digits.
fn plain_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() <= 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The event uuid of a report: the same for the same incident, always.
pub fn event_uuid(incident: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, incident.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    pub const REPORT: &str = concat!(
        r#"{"app_name":"powermove-native","bundleID":"com.zellzoi.powermove","app_version":"2.0.1","slice_uuid":"6030ab4f-66dd-3529-86e7-67196e0787b0","incident_id":"7CEEA3B7-C14D-4DD6-B2EC-BB8161C6EDEA","name":"powermove-native"}"#,
        "\n",
        r#"{"procPath":"/Users/jane/Applications/Powermove.app/Contents/MacOS/powermove-native",
          "pid":4242,"procLaunch":"2026-11-24 20:44:47.0651 -0700","captureTime":"2026-11-24 20:44:58.2811 -0700",
          "exception":{"type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS at 0x0 Jane Secret Launch","message":"Jane Secret Launch"},
          "termination":{"indicator":"Segmentation fault: 11 Jane Secret Launch"},
          "asi":{"libsystem_c.dylib":["abort() called for Jane Secret Launch.pmv"]},
          "faultingThread":1,
          "threads":[{"frames":[]},{"triggered":true,"name":"Jane Secret Launch","frames":[
            {"imageOffset":16,"symbol":"__pthread_kill","imageIndex":1},
            {"imageOffset":32,"symbol":"jane_secret_plugin","imageIndex":2},
            {"imageOffset":4096,"symbol":"powermove_native::render","imageIndex":0},
            {"imageOffset":18446744073709551615,"imageIndex":0}]}],
          "usedImages":[
            {"base":4294967296,"size":1048576,"uuid":"6030ab4f-66dd-3529-86e7-67196e0787b0",
             "path":"/Users/jane/Applications/Powermove.app/Contents/MacOS/powermove-native","name":"powermove-native"},
            {"base":8589934592,"size":4096,"uuid":"11111111-2222-3333-4444-555555555555",
             "path":"/usr/lib/system/libsystem_kernel.dylib","name":"libsystem_kernel.dylib"},
            {"base":12884901888,"size":4096,"uuid":"21111111-2222-3333-4444-555555555555",
             "path":"/Users/jane/Plugins/JaneSecret.dylib","name":"JaneSecret.dylib"}]}"#
    );

    /// Sol P0 (w99 review): a crash's text never leaves.
    #[test]
    fn panic_and_crash_text_never_leaves() {
        assert!(super::parse(REPORT, "com.powermove.native.dev.x").is_none());
        let report = super::parse(REPORT, "com.zellzoi.powermove").unwrap();
        assert_eq!(report.pid, 4242);
        let props = &report.properties;
        assert_eq!(props["crash"], true);
        let e = &props["$exception_list"][0];
        assert_eq!(e["type"], "EXC_BAD_ACCESS (SIGSEGV)");
        assert_eq!(e["value"], "crash");
        assert_eq!(props["crashed_app_version"], "2.0.1");
        let text = serde_json::to_string(props).unwrap();
        for leak in [
            "Jane",
            "jane",
            "Secret",
            "/Users",
            "KERN_",
            "abort()",
            "Segmentation",
            "dylib",
        ] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        let frames = e["stacktrace"]["frames"].as_array().unwrap();
        // Our frame kept; the bad offset and the other binaries' frames are
        // anonymous placeholders (the two inner ones folded into one).
        assert_eq!(frames.len(), 3);
        assert!(frames[0].get("instruction_addr").is_none());
        assert_eq!(frames[1]["function"], "powermove_native::render");
        assert_eq!(frames[1]["instruction_addr"], "0x100001000");
        assert!(frames[2].get("instruction_addr").is_none());
    }

    /// Sol P1 (w99 review): bounded, and keyed by incident (sent once).
    #[test]
    fn crash_import_is_bounded_and_keyed_by_incident() {
        let dir = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        for n in 0..(super::MAX_SCAN + 10) {
            std::fs::write(dir.path().join(format!("r{n}.ips")), REPORT).unwrap();
        }
        let big = "x".repeat(super::MAX_FILE_BYTES as usize + 1);
        std::fs::write(dir.path().join("big.ips"), big).unwrap();
        let found = super::candidates(dir.path(), now);
        assert_eq!(found.len(), super::MAX_SCAN);
        assert!(found.iter().all(|p| !p.ends_with("big.ips")));
        let a = super::parse(&super::read(&found[0]).unwrap(), "com.zellzoi.powermove").unwrap();
        let b = super::parse(&super::read(&found[1]).unwrap(), "com.zellzoi.powermove").unwrap();
        assert_eq!(a.incident, b.incident);
        assert_eq!(
            super::event_uuid(&a.incident),
            super::event_uuid(&b.incident)
        );
        // A version that isn't plain numbers is not sent.
        let odd = REPORT.replace(r#""app_version":"2.0.1""#, r#""app_version":"2.0 Jane""#);
        let props = super::parse(&odd, "com.zellzoi.powermove")
            .unwrap()
            .properties;
        assert!(props.get("crashed_app_version").is_none());
    }
}
