//! Opt-in linked adapter; the application retains it on its main run loop.
use super::{ArchiveKey, Binding, Event, InstalledProof};
use crate::update_distribution::Release;
use anyhow::{Result, ensure};
use std::{
    ffi::{CString, c_char, c_int, c_void},
    marker::PhantomData,
    path::{Path, PathBuf},
    ptr::NonNull,
    rc::Rc,
};
unsafe extern "C" {
    fn pm_sparkle_public_key(context: *mut c_void) -> *const c_char;
    fn pm_sparkle_create(path: *const c_char) -> *mut c_void;
    fn pm_sparkle_check(context: *mut c_void, release: *const c_char) -> c_int;
    fn pm_sparkle_action(context: *mut c_void, action: c_int) -> c_int;
    fn pm_sparkle_poll(context: *mut c_void) -> c_int;
    fn pm_sparkle_release(context: *mut c_void);
    fn pm_sparkle_installed(
        path: *const c_char,
        version: *const c_char,
        display_version: *const c_char,
        identifier: *const c_char,
        architecture: *const c_char,
        requirement: *const c_char,
    ) -> c_int;
}
pub struct MacSparkle {
    context: NonNull<c_void>,
    armed: bool,
    _main_thread: PhantomData<Rc<()>>,
}
impl MacSparkle {
    /// Fails off the main thread or without disabled automatic updates and a
    /// bundled HTTPS feed/public key. Does not start an updater or open windows.
    pub fn new(bundle: &Path) -> Result<Self> {
        let path = CString::new(bundle.as_os_str().as_encoded_bytes())?;
        let context =
            NonNull::new(unsafe { pm_sparkle_create(path.as_ptr()) }).ok_or_else(|| {
                anyhow::anyhow!("Sparkle main-thread/configuration requirement failed")
            })?;
        Ok(Self {
            context,
            armed: false,
            _main_thread: PhantomData,
        })
    }
    pub fn archive_key(&self) -> Result<ArchiveKey> {
        let key = unsafe { pm_sparkle_public_key(self.context.as_ptr()) };
        ensure!(!key.is_null(), "Sparkle bundle key unavailable");
        ArchiveKey::from_base64(unsafe { std::ffi::CStr::from_ptr(key) }.to_str()?)
    }
    pub fn poll(&mut self) -> Result<Option<Event>> {
        let event = unsafe { pm_sparkle_poll(self.context.as_ptr()) };
        Ok(match event {
            0 => None,
            1 => Some(Event::Found),
            2 => Some(Event::Downloading),
            3 => Some(Event::Extracting),
            4 => Some(Event::Ready),
            5 => Some(Event::Installed),
            6 => Some(Event::Aborted),
            7 => Some(Event::Cancelled),
            _ => anyhow::bail!("Sparkle used off main thread"),
        })
    }
    fn action(&mut self, action: i32) -> Result<()> {
        ensure!(
            unsafe { pm_sparkle_action(self.context.as_ptr(), action) } == 1,
            "Sparkle continuation unavailable"
        );
        Ok(())
    }
}
impl Binding for MacSparkle {
    fn check(&mut self, release: &Release) -> Result<()> {
        let json = CString::new(serde_json::to_vec(release)?)?;
        self.armed = true; // even a partial start retains the driver
        ensure!(
            unsafe { pm_sparkle_check(self.context.as_ptr(), json.as_ptr()) } == 1,
            "Sparkle could not start"
        );
        Ok(())
    }
    fn download(&mut self) -> Result<()> {
        self.action(1)
    }
    fn install(&mut self) -> Result<()> {
        self.action(2)
    }
    fn cancel(&mut self) -> Result<()> {
        self.action(3)
    }
}
impl Drop for MacSparkle {
    fn drop(&mut self) {
        // An armed updater and callbacks must outlive outstanding installer work.
        // The app retains the driver to termination; never free beneath Sparkle.
        if !self.armed {
            unsafe { pm_sparkle_release(self.context.as_ptr()) };
        }
    }
}
pub struct SignedInstalledBundle {
    pub bundle: PathBuf,
    pub designated_requirement: String,
}
impl InstalledProof for SignedInstalledBundle {
    fn matches(&mut self, release: &Release) -> Result<bool> {
        let path = CString::new(self.bundle.as_os_str().as_encoded_bytes())?;
        let version = CString::new(release.sparkle_version().as_bytes())?;
        let display_version = CString::new(release.version.as_bytes())?;
        let id = CString::new(release.bundle_id.as_bytes())?;
        let arch = CString::new(match release.architecture {
            crate::update_distribution::Architecture::Arm64 => "arm64",
            crate::update_distribution::Architecture::X86_64 => "x86_64",
        })?;
        let dr = CString::new(self.designated_requirement.as_bytes())?;
        ensure!(
            !self.designated_requirement.is_empty(),
            "missing designated requirement"
        );
        Ok(unsafe {
            pm_sparkle_installed(
                path.as_ptr(),
                version.as_ptr(),
                display_version.as_ptr(),
                id.as_ptr(),
                arch.as_ptr(),
                dr.as_ptr(),
            )
        } == 1)
    }
}

/// Commands contain only authenticated release facts, never archives/secrets.
/// All hashing, journals and quit barriers stay on the worker; pump on the app's
/// main run loop without waiting for that worker or a JavaScript continuation.
enum Action {
    Check(u64, Release),
    Download(u64),
    Install(u64),
    Cancel(u64),
}
impl Action {
    fn cycle(&self) -> u64 {
        match self {
            Self::Check(cycle, _)
            | Self::Download(cycle)
            | Self::Install(cycle)
            | Self::Cancel(cycle) => *cycle,
        }
    }
}
pub struct WorkerBinding {
    cycle: u64,
    commands: std::sync::mpsc::Sender<Action>,
    events: std::sync::mpsc::Receiver<(u64, Event)>,
}
impl WorkerBinding {
    pub fn try_event(&self) -> Result<Option<Event>> {
        loop {
            match self.events.try_recv() {
                Ok((cycle, e)) if cycle == self.cycle => return Ok(Some(e)),
                Ok(_) => continue, // a late terminal callback from a prior cycle
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(None),
                Err(_) => anyhow::bail!("Sparkle main loop disconnected"),
            }
        }
    }
    fn send(&self, action: Action) -> Result<()> {
        self.commands
            .send(action)
            .map_err(|_| anyhow::anyhow!("Sparkle main loop disconnected"))
    }
}
impl Binding for WorkerBinding {
    fn check(&mut self, release: &Release) -> Result<()> {
        let cycle = self
            .cycle
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Sparkle cycle exhausted"))?;
        self.send(Action::Check(cycle, release.clone()))?;
        self.cycle = cycle;
        Ok(())
    }
    fn download(&mut self) -> Result<()> {
        self.send(Action::Download(self.cycle))
    }
    fn install(&mut self) -> Result<()> {
        self.send(Action::Install(self.cycle))
    }
    fn cancel(&mut self) -> Result<()> {
        self.send(Action::Cancel(self.cycle))
    }
}
pub struct MainLoop {
    driver: MacSparkle,
    cycle: u64,
    commands: std::sync::mpsc::Receiver<Action>,
    events: std::sync::mpsc::Sender<(u64, Event)>,
}
impl MainLoop {
    pub fn connect(driver: MacSparkle) -> (Self, WorkerBinding) {
        let (commands_tx, commands) = std::sync::mpsc::channel();
        let (events, events_rx) = std::sync::mpsc::channel();
        (
            Self {
                driver,
                cycle: 0,
                commands,
                events,
            },
            WorkerBinding {
                cycle: 0,
                commands: commands_tx,
                events: events_rx,
            },
        )
    }
    pub fn pump(&mut self) -> Result<()> {
        while let Ok(action) = self.commands.try_recv() {
            let cycle = action.cycle();
            let result = match action {
                Action::Check(cycle, r) if cycle > self.cycle => {
                    self.driver.check(&r).map(|()| self.cycle = cycle)
                }
                Action::Download(cycle) if cycle == self.cycle => self.driver.download(),
                Action::Install(cycle) if cycle == self.cycle => self.driver.install(),
                Action::Cancel(cycle) if cycle == self.cycle => self.driver.cancel(),
                _ => Err(anyhow::anyhow!("stale Sparkle main-loop command")),
            };
            if let Err(e) = result {
                let _ = self.events.send((cycle, Event::Aborted));
                return Err(e);
            }
        }
        while let Some(event) = self.driver.poll()? {
            self.events
                .send((self.cycle, event))
                .map_err(|_| anyhow::anyhow!("Sparkle worker disconnected"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update_distribution::{Architecture, Channel};
    #[test]
    fn linked_enclosure_match_is_exact_and_refuses_delta_package_or_information() {
        unsafe extern "C" {
            fn pm_sparkle_matches(policy: *const c_char, enclosure: *const c_char) -> c_int;
        }
        let release = serde_json::json!({"version":"2.0.0","sparkleVersion":"42","archiveUrl":"https://updates.invalid/app.zip","size":42,"sparkleSignature":"synthetic-signature"});
        let actual = serde_json::json!({"version":"42","url":"https://updates.invalid/app.zip","size":42,"signature":"synthetic-signature","delta":false,"informationOnly":false,"installationType":"application"});
        let matches = |a: &serde_json::Value| {
            let r = CString::new(release.to_string()).unwrap();
            let a = CString::new(a.to_string()).unwrap();
            unsafe { pm_sparkle_matches(r.as_ptr(), a.as_ptr()) == 1 }
        };
        assert!(matches(&actual));
        for (key, value) in [
            ("version", serde_json::json!("3.0.0")),
            (
                "url",
                serde_json::json!("https://updates.invalid/other.zip"),
            ),
            ("size", serde_json::json!(43)),
            ("signature", serde_json::json!("different")),
            ("delta", serde_json::json!(true)),
            ("informationOnly", serde_json::json!(true)),
            ("installationType", serde_json::json!("package")),
        ] {
            let mut changed = actual.clone();
            changed[key] = value;
            assert!(!matches(&changed), "{key}");
        }
    }
    #[test]
    fn native_cancellation_from_previous_cycle_cannot_release_new_extraction() {
        unsafe extern "C" {
            fn pm_sparkle_cycle_transition(state: u32, signal: c_int) -> u32;
            fn pm_sparkle_cycle_can_dismiss(state: u32) -> c_int;
        }
        unsafe {
            let extracting = pm_sparkle_cycle_transition(0, 1);
            assert_eq!(pm_sparkle_cycle_can_dismiss(extracting), 0);
            let cancelled = pm_sparkle_cycle_transition(extracting, 2);
            assert_eq!(pm_sparkle_cycle_can_dismiss(cancelled), 1);
            let new_cycle = pm_sparkle_cycle_transition(cancelled, 0);
            assert_eq!(pm_sparkle_cycle_can_dismiss(new_cycle), 1);
            let new_extraction = pm_sparkle_cycle_transition(new_cycle, 1);
            assert_eq!(pm_sparkle_cycle_can_dismiss(new_extraction), 0);
            let ready_cancelled = pm_sparkle_cycle_transition(new_extraction, 2);
            assert_eq!(pm_sparkle_cycle_can_dismiss(ready_cancelled), 1);
        }
    }
    #[test]
    fn worker_ignores_late_terminal_callbacks_from_previous_manual_cycle() {
        let (commands, received) = std::sync::mpsc::channel();
        let (events, delivered) = std::sync::mpsc::channel();
        let mut worker = WorkerBinding {
            cycle: 0,
            commands,
            events: delivered,
        };
        let release: Release = serde_json::from_value(serde_json::json!({
            "schema":1,"channel":"beta","version":"2.0.0","sparkleVersion":"42",
            "architecture":"arm64","bundleId":"com.powermove.signature-proof",
            "minimumMacos":14,"archiveUrl":"https://updates.invalid/app.zip","size":42,
            "sha256":"0".repeat(64),"sparkleSignature":"synthetic","expiresUnix":42
        }))
        .unwrap();
        worker.check(&release).unwrap();
        assert_eq!(received.recv().unwrap().cycle(), 1);
        events.send((0, Event::Cancelled)).unwrap();
        events.send((1, Event::Ready)).unwrap();
        assert_eq!(worker.try_event().unwrap(), Some(Event::Ready));
        worker.check(&release).unwrap();
        assert_eq!(received.recv().unwrap().cycle(), 2);
        events.send((1, Event::Cancelled)).unwrap();
        events.send((1, Event::Aborted)).unwrap();
        events.send((2, Event::Extracting)).unwrap();
        assert_eq!(worker.try_event().unwrap(), Some(Event::Extracting));
        assert_eq!(worker.try_event().unwrap(), None);
    }
    fn run(command: &str, args: &[&str]) {
        assert!(
            std::process::Command::new(command)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    #[test]
    fn static_bundle_proof_checks_signature_dr_version_architecture_and_tamper_without_launch() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let app = root.join("Synthetic.app");
        let contents = app.join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        let plist = contents.join("Info.plist");
        std::fs::write(&plist, r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.powermove.signature-proof</string><key>CFBundleExecutable</key><string>proof</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleVersion</key><string>42</string><key>CFBundleShortVersionString</key><string>2.0.0</string></dict></plist>"#).unwrap();
        let source = root.join("proof.c");
        std::fs::write(&source, "int main(void) { return 0; }\n").unwrap();
        let binary = contents.join("MacOS/proof");
        run(
            "xcrun",
            &[
                "clang",
                "-arch",
                "arm64",
                "-arch",
                "x86_64",
                "-mmacosx-version-min=14.0",
                source.to_str().unwrap(),
                "-o",
                binary.to_str().unwrap(),
            ],
        );
        run(
            "codesign",
            &[
                "--force",
                "--sign",
                "-",
                "--timestamp=none",
                app.to_str().unwrap(),
            ],
        );
        let mut proof = SignedInstalledBundle {
            bundle: app,
            designated_requirement: "identifier \"com.powermove.signature-proof\"".into(),
        };
        let mut release = Release {
            schema: 1,
            channel: Channel::Beta,
            version: "2.0.0".into(),
            sparkle_version: Some("42".into()),
            architecture: Architecture::Arm64,
            bundle_id: "com.powermove.signature-proof".into(),
            minimum_macos: 14,
            archive_url: "https://updates.invalid/app.zip".into(),
            size: 1,
            sha256: "0".repeat(64),
            sparkle_signature: String::new(),
            expires_unix: 1,
        };
        assert!(proof.matches(&release).unwrap());
        release.sparkle_version = Some("43".into());
        assert!(!proof.matches(&release).unwrap());
        release.sparkle_version = Some("42".into());
        release.architecture = Architecture::X86_64;
        assert!(proof.matches(&release).unwrap());
        release.version = "3.0.0".into();
        assert!(!proof.matches(&release).unwrap());
        release.version = "2.0.0".into();
        proof.designated_requirement = "identifier \"com.powermove.signature-proof.wrong\"".into();
        assert!(!proof.matches(&release).unwrap());
        proof.designated_requirement = "identifier \"com.powermove.signature-proof\"".into();
        release.bundle_id = "com.powermove.signature-proof.wrong".into();
        assert!(!proof.matches(&release).unwrap());
        release.bundle_id = "com.powermove.signature-proof".into();
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(binary)
            .unwrap()
            .write_all(b"tamper")
            .unwrap();
        assert!(!proof.matches(&release).unwrap());
    }
}
