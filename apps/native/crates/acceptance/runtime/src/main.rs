//! Optional real V8 backing. The parent acceptance runner owns the wall timeout.
use pm_runtime_v8::*;
mod native_compat;
mod portable;
mod source_mcp;
// fork/exec can briefly inherit a service lock before CLOEXEC runs. Keep the
// bundler child outside the exact drop/reopen assertion; never retry the lock.
#[cfg(test)]
static FIXTURE_PROCESS_GATE: Mutex<()> = Mutex::new(());
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Host {
    storage: Mutex<Value>,
    ops: Mutex<Vec<Value>>,
}
impl KernelHooks for Host {
    fn load_storage(&self, _: &str) -> Result<Value> {
        Ok(self.storage.lock().unwrap().clone())
    }
    fn commit_activation(
        &self,
        _: Option<&Generation>,
        _: &Generation,
        ops: &[Value],
        storage: &Value,
    ) -> Result<()> {
        *self.storage.lock().unwrap() = storage.clone();
        self.ops.lock().unwrap().extend_from_slice(ops);
        Ok(())
    }
    fn commit_retained(&self, _: &Generation, ops: &[Value]) -> Result<()> {
        self.ops.lock().unwrap().extend_from_slice(ops);
        Ok(())
    }
    fn revoke_generation(&self, _: &Generation) {}
    fn seal_open_gestures(&self, _: &Generation) {}
}
impl HostProxy for Host {
    fn fetch(
        &self,
        _: &Generation,
        _: FetchRequest,
        _: Cancellation,
    ) -> Result<Box<dyn HostRequest>> {
        Err(Error::Host("No network in synthetic acceptance".into()))
    }
    fn console(&self, _: &Generation, _: &str, _: &str) {}
}
impl WatchdogHooks for Host {
    fn notice(&self, _: &Generation, _: u64) {}
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        WatchdogDecision::Wait
    }
}
fn wait<T>(mut ticket: Ticket<T>) -> Result<T> {
    wait_until(&mut ticket, Instant::now() + Duration::from_secs(10))
        .unwrap_or_else(|| Err(Error::Host("acceptance execution timeout".into())))
}
fn eval(realm: &Realm, source: impl Into<String>) -> Result<Evaluation> {
    wait(realm.eval(source)?)
}
fn check(id: &str, bundle: Option<&str>) -> Result<()> {
    if id == "A510" {
        return source_mcp::fork().map_err(Error::Host);
    }
    if native_compat::IDS.contains(&id) {
        return native_compat::check(id);
    }
    if let Some(result) = portable::check(id) {
        return result.map_err(Error::Host);
    }
    let host = Arc::new(Host {
        storage: Mutex::new(json!({})),
        ..Default::default()
    });
    let admission = Admission::new(512 * 1024 * 1024);
    let pool = RuntimePool::new(
        "acceptance-synthetic",
        Config {
            limits: Limits::default(),
            watchdog: WatchdogPolicy::default(),
            kernel: host.clone(),
            host: host.clone(),
            watchdog_hooks: host.clone(),
            cache: CodeCache::new(4 * 1024 * 1024),
            engine: None,
            admission: admission.clone(),
        },
    )?;
    let realm = pool.realm(RealmKind::Compat)?;
    match id {
        "W037.v8-reload" => {
            let before = eval(
                &realm,
                "storage.deep={n:7};hot.data.deep={n:11};globalThis.provider={value:19};runtime.stage({id:'kept'});typeof process+':'+typeof document",
            )?;
            assert_eq!(before.value, "undefined:undefined");
            assert!(wait(realm.reload("storage.deep.n=99;hot.data.deep.n=99;runtime.stage({id:'refused'});throw Error('synthetic activation refusal')")?).is_err());
            let after = eval(
                &realm,
                "storage.deep.n+':'+hot.data.deep.n+':'+provider.value",
            )?;
            assert_eq!(after.value, "7:11:19");
            assert_eq!(after.generation, before.generation);
            assert!(
                !host
                    .ops
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|op| op["id"] == "refused")
            );
        }
        "W038.v8-stop" => {
            let other = pool.realm(RealmKind::V4Extension("other".into()))?;
            let ticket = realm.eval("while(true){}")?;
            assert_eq!(eval(&other, "6*7")?.value, "42");
            let end = Instant::now() + Duration::from_secs(3);
            while realm.active_execution().is_none() {
                assert!(Instant::now() < end);
                thread::yield_now();
            }
            assert!(realm.stop(ticket.execution));
            assert!(matches!(wait(ticket), Err(Error::Terminated)));
            assert_eq!(eval(&other, "40+2")?.value, "42");
            other.close();
        }
        "W037.compat-services"
        | "W040.compat-recorder"
        | "W040.compat-remote-dom"
        | "A660"
        | "A661"
        | "A662"
        | "A663"
        | "A666"
        | "A668"
        | "A669"
        | "A678"
        | "B568"
        | "B569"
        | "B570"
        | "B610"
        | "B611"
        | "B612"
        | "B613"
        | "B614"
        | "A693"
        | "A695"
        | "A696"
        | "A698"
        | "A659"
        | "A689"
        | "A691" => {
            let bundle = bundle.ok_or(Error::Invalid("compat bundle required"))?;
            eval(&realm, bundle)?;
            assert_eq!(
                eval(&realm, format!("globalThis.__acceptanceResult = null; Promise.resolve(PowermoveAcceptance.run({})).then(v => {{ __acceptanceResult = v }}, e => {{ __acceptanceResult = String(e) }}); true", json!(id)))?.value,
                "true"
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let result = eval(&realm, "JSON.stringify(__acceptanceResult)")?.value;
                if result == "true" {
                    break;
                }
                if result != "null" || Instant::now() >= deadline {
                    return Err(Error::Host(format!("compat assertion {id}: {result}")));
                }
                thread::sleep(Duration::from_millis(1));
            }
            if id == "W040.compat-remote-dom" {
                let evidence = eval(&realm, "JSON.stringify(PowermoveAcceptance.evidence())")?;
                let evidence: Value = serde_json::from_str(&evidence.value)
                    .map_err(|e| Error::Host(e.to_string()))?;
                portable::admit_remote_evidence(evidence, |event| {
                    let reply = eval(
                        &realm,
                        format!("JSON.stringify(PowermoveAcceptance.receive({event}))"),
                    )
                    .map_err(|e| e.to_string())?;
                    serde_json::from_str(&reply.value).map_err(|e| e.to_string())
                })
                .map_err(Error::Host)?;
            }
        }
        _ => return Err(Error::Invalid("unknown runtime acceptance ID")),
    }
    realm.close();
    drop(pool);
    let end = Instant::now() + Duration::from_secs(5);
    while admission.used() != 0 {
        assert!(Instant::now() < end, "V8 owners did not dispose");
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
fn main() {
    let result = thread::spawn(|| {
        let mut args = std::env::args().skip(1);
        let id = args.next().ok_or("runtime case required".to_string())?;
        let bundle = args
            .next()
            .map(std::fs::read_to_string)
            .transpose()
            .map_err(|e| e.to_string())?;
        if args.next().is_some() {
            return Err("unexpected runtime arguments".into());
        }
        check(&id, bundle.as_deref()).map_err(|e| e.to_string())
    })
    .join();
    match result {
        Ok(Ok(())) => println!("{{\"ok\":true}}"),
        Ok(Err(e)) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
        Err(_) => std::process::exit(1),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_current_source_compat_case_executes_in_native_v8() {
        let gate = FIXTURE_PROCESS_GATE.lock().unwrap();
        let built = std::process::Command::new("bun")
            .arg("build")
            .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("compat-probes.ts"))
            .args(["--target=browser", "--format=iife"])
            .output()
            .expect("Bun is required to bundle current production compat sources");
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let bundle = String::from_utf8(built.stdout).unwrap();
        drop(gate);
        for &(id, _, _, requires_bundle) in pm_acceptance::runtime::CASES {
            if requires_bundle {
                check(id, Some(&bundle)).unwrap_or_else(|error| panic!("{id}: {error}"));
            }
        }
    }
    #[test]
    fn reload_preserves_active_generation() {
        check("W037.v8-reload", None).unwrap();
    }
    #[test]
    fn stop_does_not_stop_other_isolates() {
        check("W038.v8-stop", None).unwrap();
    }
    #[test]
    fn missing_bundle_is_an_error() {
        assert!(check("W037.compat-services", None).is_err());
    }
    #[test]
    fn malformed_bundle_is_an_error() {
        assert!(check("W037.compat-services", Some("throw Error('synthetic')")).is_err());
    }
}
