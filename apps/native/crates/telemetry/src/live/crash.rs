//! Hard crashes (a segfault, an abort, a stack overflow) end the process
//! before anything can be sent. macOS writes a crash report for each
//! (`~/Library/Logs/DiagnosticReports/*.ips`); at the next launch the reports
//! of this bundle newer than the last one sent are read and each is sent once
//! as an exception with `crash: true`: the exception, and the crashed
//! thread's frames as binary + offset (symbols the report carries are kept,
//! paths and everything else in the report stay on the Mac).
use super::exception::{Frame, Image, Kind};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Frames kept per crash, nearest the crash site (a stack overflow has
/// thousands).
const MAX_FRAMES: usize = 128;

pub struct Crash {
    /// The report file's time (seconds since 1970).
    pub time: f64,
    pub properties: Map<String, Value>,
}

pub fn reports_folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Logs/DiagnosticReports"))
}

/// Reports of `bundle_id` written after `since`, oldest first.
pub fn new_reports(folder: &Path, bundle_id: &str, since: f64) -> Vec<Crash> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return vec![];
    };
    let mut found: Vec<(f64, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "ips"))
        .filter_map(|e| {
            let time = e
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs_f64();
            (time > since).then(|| (time, e.path()))
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found
        .into_iter()
        .filter_map(|(time, path)| {
            let text = std::fs::read_to_string(&path).ok()?;
            let properties = parse(&text, bundle_id)?;
            Some(Crash { time, properties })
        })
        .collect()
}

/// The exception properties of one `.ips` report, if it is `bundle_id`'s.
pub fn parse(text: &str, bundle_id: &str) -> Option<Map<String, Value>> {
    let (header, body) = text.split_once('\n')?;
    let header: Value = serde_json::from_str(header).ok()?;
    if header["bundleID"].as_str() != Some(bundle_id) {
        return None;
    }
    let body: Value = serde_json::from_str(body).ok()?;
    let images: Vec<Image> = body["usedImages"]
        .as_array()?
        .iter()
        .map(|i| Image {
            base: i["base"].as_u64().unwrap_or(0),
            size: i["size"].as_u64().unwrap_or(0),
            uuid: i["uuid"].as_str().unwrap_or("").to_uppercase(),
            name: i["name"]
                .as_str()
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_owned(),
        })
        .collect();
    let faulting = body["faultingThread"].as_u64().unwrap_or(0) as usize;
    let thread = &body["threads"][faulting];
    let frames: Vec<Frame> = thread["frames"]
        .as_array()
        .map(|frames| {
            frames
                .iter()
                .take(MAX_FRAMES)
                .filter_map(|f| {
                    let image = images.get(f["imageIndex"].as_u64()? as usize)?;
                    Some(Frame {
                        addr: image.base + f["imageOffset"].as_u64()?,
                        function: f["symbol"].as_str().map(str::to_owned),
                        // The report names the source file without its folder.
                        file: f["sourceFile"].as_str().map(str::to_owned),
                        line: f["sourceLine"].as_u64().map(|l| l as u32),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let exception = &body["exception"];
    let kind = match (exception["type"].as_str(), exception["signal"].as_str()) {
        (Some(t), Some(s)) => format!("{t} ({s})"),
        (Some(t), None) => t.to_owned(),
        _ => "Crash".to_owned(),
    };
    let mut message: Vec<&str> = [
        exception["subtype"].as_str(),
        exception["message"].as_str(),
        body["termination"]["indicator"].as_str(),
    ]
    .into_iter()
    .flatten()
    .collect();
    // Application-specific information (`abort() called`, a Rust panic's
    // last words): kept, then scrubbed with the rest.
    let asi: Vec<String> = body["asi"]
        .as_object()
        .map(|asi| {
            asi.values()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    message.extend(asi.iter().map(String::as_str));
    let mut props = super::exception::properties(
        Kind {
            exception_type: &kind,
            message: &message.join(" — "),
            mechanism: "crash",
            level: "fatal",
        },
        &frames,
        &images,
    );
    props.insert("crash".into(), true.into());
    if let Some(version) = body["bundleInfo"]["CFBundleShortVersionString"].as_str() {
        props.insert("crashed_app_version".into(), version.into());
    }
    Some(props)
}

#[cfg(test)]
mod tests {
    const REPORT: &str = concat!(
        r#"{"app_name":"powermove-native","bundleID":"com.zellzoi.powermove","name":"powermove-native"}"#,
        "\n",
        r#"{"procPath":"/Users/jane/Applications/Powermove.app/Contents/MacOS/powermove-native",
          "exception":{"type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS at 0x0"},
          "termination":{"indicator":"Segmentation fault: 11"},
          "asi":{"libsystem_c.dylib":["abort() called"]},
          "faultingThread":1,
          "bundleInfo":{"CFBundleShortVersionString":"2.0.1"},
          "threads":[{"frames":[]},{"triggered":true,"frames":[
            {"imageOffset":16,"symbol":"__pthread_kill","imageIndex":1},
            {"imageOffset":4096,"imageIndex":0}]}],
          "usedImages":[
            {"base":4294967296,"size":1048576,"uuid":"6030ab4f-66dd-3529-86e7-67196e0787b0",
             "path":"/Users/jane/Applications/Powermove.app/Contents/MacOS/powermove-native","name":"powermove-native"},
            {"base":8589934592,"size":4096,"uuid":"11111111-2222-3333-4444-555555555555",
             "path":"/usr/lib/system/libsystem_kernel.dylib","name":"libsystem_kernel.dylib"}]}"#
    );

    #[test]
    fn a_report_of_this_bundle_becomes_one_scrubbed_crash() {
        assert!(super::parse(REPORT, "com.powermove.native.dev.x").is_none());
        let props = super::parse(REPORT, "com.zellzoi.powermove").unwrap();
        assert_eq!(props["crash"], true);
        let e = &props["$exception_list"][0];
        assert_eq!(e["type"], "EXC_BAD_ACCESS (SIGSEGV)");
        assert!(e["value"].as_str().unwrap().contains("abort() called"));
        let frames = e["stacktrace"]["frames"].as_array().unwrap();
        // Outermost first: the app frame, then the crash site.
        assert_eq!(frames[0]["instruction_addr"], "0x100001000");
        assert_eq!(frames[0]["image_addr"], "0x100000000");
        assert_eq!(frames[1]["function"], "__pthread_kill");
        let images = props["$debug_images"].as_array().unwrap();
        assert_eq!(
            images[0]["debug_id"],
            "6030AB4F-66DD-3529-86E7-67196E0787B0"
        );
        let text = serde_json::to_string(&props).unwrap();
        assert!(
            !text.contains("/Users/") && !text.contains("jane"),
            "{text}"
        );
    }

    #[test]
    fn only_reports_newer_than_the_last_sent_are_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ips"), REPORT).unwrap();
        std::fs::write(dir.path().join("b.txt"), REPORT).unwrap();
        let all = super::new_reports(dir.path(), "com.zellzoi.powermove", 0.);
        assert_eq!(all.len(), 1);
        assert!(super::new_reports(dir.path(), "com.zellzoi.powermove", all[0].time).is_empty());
    }
}
