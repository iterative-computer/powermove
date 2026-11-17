//! Synthetic transport admission: no account, HTTP request or provider CLI.
use pm_agents::{
    Access,
    compatible::{self, ChatHost, Saved},
    transport::Action,
};
use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};
use std::time::Duration;
#[derive(Debug)]
struct HeldHost(Mutex<mpsc::Receiver<()>>);
impl ChatHost for HeldHost {
    fn saved(&self) -> Option<Saved> {
        let _ = self.0.lock().unwrap().recv_timeout(Duration::from_secs(5));
        None
    }
    fn key(&self, _: &str) -> Option<String> {
        None
    }
    fn stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Vec<u8>,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<u16, String> {
        panic!("no network")
    }
}
#[test]
fn compatible_accepts_steer_while_working() {
    let (release, held) = mpsc::channel();
    let handle = compatible::start(
        Arc::new(HeldHost(Mutex::new(held))),
        compatible::Run {
            id: "run".into(),
            prompt: "Original brief".into(),
            model: None,
            effort: None,
            access: Access::Editor,
            instructions: String::new(),
            images: vec![],
            tools: ("unused".into(), vec![]),
            tool_env: Default::default(),
            allowed: vec![],
        },
    );
    let ack = handle
        .send_checked(Action::Steer {
            run: "run".into(),
            prompt: "Make it blue".into(),
            images: vec!["/synthetic/reference.png".into()],
        })
        .unwrap_or_else(|_| panic!("worker mailbox unavailable"));
    let accepted = ack.recv_timeout(Duration::from_secs(2)).unwrap();
    release.send(()).unwrap();
    assert!(accepted, "compatible transport rejected live direction");
}
