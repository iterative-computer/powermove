//! Raw-byte images (P-IMAGES): a prompt's and a tool result's images cross
//! as raw bytes (the core base64-encodes them into the model request, Rust
//! never does); bad and oversized images are refused; a steer with images
//! hands the turn off and comes back to be sent as a prompt.

mod support;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{Error, Host, Image, Input, Outcome, Runtime, Steered, Tool, ToolLease};
use support::{
    Driver, Model, Reply, Seen, VISION_CATALOG, config, on_worker, sse_text, sse_tool_calls,
};

const ENDPOINT: &str = "http://127.0.0.1:9/fx";
const SCHEMA: &str = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
const SNAP: [Tool<'static>; 1] = [Tool {
    name: "snapshot",
    description: "Renders a frame.",
    input_schema_json: SCHEMA,
    idempotent: true,
    writes: false,
}];

/// A 1x1 RGBA PNG.
const PNG: [u8; 67] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

fn png() -> &'static [u8] {
    &PNG
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

struct Queue(Mutex<Sender<ToolLease>>);
impl Host for Queue {
    fn tool_call(&self, call: ToolLease) {
        let _ = self.0.lock().unwrap().send(call);
    }
}

fn queue() -> (Arc<Queue>, Receiver<ToolLease>) {
    let (send, receive) = channel();
    (Arc::new(Queue(Mutex::new(send))), receive)
}

fn recording(
    bodies: Arc<Mutex<Vec<String>>>,
    reply: impl Fn(usize) -> Reply + Send + Sync + 'static,
) -> Model {
    Arc::new(move |call| {
        bodies.lock().unwrap().push(call.body.clone());
        reply(call.index)
    })
}

fn lease(driver: &mut Driver, leases: &Receiver<ToolLease>) -> ToolLease {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        driver.pump(10);
        if let Ok(lease) = leases.try_recv() {
            return lease;
        }
        assert!(
            Instant::now() < deadline,
            "no tool call; saw {:#?}",
            driver.seen
        );
    }
}

fn end(driver: &mut Driver, turn: u64) -> Seen {
    driver.until(30, |e| {
        matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn) || matches!(e, Seen::Exited { .. })
    })
}

#[test]
fn prompt_and_tool_images_cross_as_raw_bytes() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, leases) = queue();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let model = recording(bodies.clone(), |index| match index {
            0 => Reply::Sse(sse_tool_calls("snapshot", &[r#"{"q":"frame"}"#])),
            _ => Reply::Sse(sse_text("seen")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &SNAP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.catalog = Some(VISION_CATALOG.to_string());
        driver.open();
        let image = [Image {
            mime: "image/png",
            data: png(),
        }];
        let turn = driver
            .runtime
            .prompt_input(
                &Input {
                    text: "what is this?",
                    images: &image,
                },
                "m-1",
            )
            .expect("prompt");
        let call = lease(&mut driver, &leases);
        // The tool's frame goes back raw too.
        call.answer_with("rendered", &image, false).expect("answer");
        assert!(matches!(end(&mut driver, turn), Seen::TurnEnd { .. }));
        let bodies = bodies.lock().unwrap();
        let encoded = base64(png());
        assert!(
            bodies[0].contains(&encoded),
            "the prompt image reached the model"
        );
        assert!(
            bodies[1].matches(&encoded).count() >= 2,
            "the tool's image reached the model"
        );
        drop(bodies);
        driver.runtime.close();
    });
}

#[test]
fn bad_and_oversized_images_are_refused_and_change_nothing() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, leases) = queue();
        let model: Model = Arc::new(|call| match call.index {
            0 => Reply::Sse(sse_tool_calls("snapshot", &[r#"{"q":"frame"}"#])),
            _ => Reply::Sse(sse_text("ok")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &SNAP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.catalog = Some(VISION_CATALOG.to_string());
        driver.open();
        let prompt = |runtime: &mut Runtime, images: &[Image<'_>]| {
            runtime.prompt_input(
                &Input {
                    text: "look",
                    images,
                },
                "m-1",
            )
        };
        // Bytes that are not the type they name, or no image at all.
        assert_eq!(
            prompt(
                &mut driver.runtime,
                &[Image {
                    mime: "image/jpeg",
                    data: png()
                }]
            ),
            Err(Error::Invalid)
        );
        assert_eq!(
            prompt(
                &mut driver.runtime,
                &[Image {
                    mime: "image/png",
                    data: b"not a png"
                }]
            ),
            Err(Error::Invalid)
        );
        assert_eq!(
            prompt(
                &mut driver.runtime,
                &[Image {
                    mime: "image/bmp",
                    data: png()
                }]
            ),
            Err(Error::Invalid)
        );
        // Past one image's bound, and past the per-prompt count.
        let mut big = vec![0u8; 5 * 1024 * 1024];
        big[..67].copy_from_slice(png());
        assert_eq!(
            prompt(
                &mut driver.runtime,
                &[Image {
                    mime: "image/png",
                    data: &big
                }]
            ),
            Err(Error::TooLarge)
        );
        let nine = [Image {
            mime: "image/png",
            data: png(),
        }; 9];
        assert_eq!(prompt(&mut driver.runtime, &nine), Err(Error::TooLarge));
        // None of that started a turn; a good prompt does.
        let turn = prompt(
            &mut driver.runtime,
            &[Image {
                mime: "image/png",
                data: png(),
            }],
        )
        .expect("prompt");
        let call = lease(&mut driver, &leases);
        // A tool image that is not its type ends the lease with an error.
        assert_eq!(
            call.answer_with(
                "frame",
                &[Image {
                    mime: "image/gif",
                    data: png()
                }],
                false
            ),
            Err(Error::Invalid)
        );
        assert!(matches!(end(&mut driver, turn), Seen::TurnEnd { .. }));
        assert!(
            driver
                .seen
                .iter()
                .any(|e| matches!(e, Seen::ToolEnd { failed: true, .. }))
        );
        driver.runtime.close();
    });
}

#[test]
fn a_steer_with_images_hands_the_turn_off_and_comes_back() {
    on_worker(|| {
        if !pm_aero_core::available() {
            return;
        }
        let (host, leases) = queue();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let model = recording(bodies.clone(), |index| match index {
            0 => Reply::Sse(sse_tool_calls("snapshot", &[r#"{"q":"frame"}"#])),
            _ => Reply::Sse(sse_text("ok")),
        });
        let runtime = Runtime::create(&config(ENDPOINT, &SNAP), host).expect("create");
        let mut driver = Driver::new(runtime, model, ENDPOINT);
        driver.catalog = Some(VISION_CATALOG.to_string());
        driver.open();
        let turn = driver.runtime.prompt("render it", "m-1").expect("prompt");
        let call = lease(&mut driver, &leases);
        let image = [Image {
            mime: "image/png",
            data: png(),
        }];
        let request = match driver
            .runtime
            .steer_input(
                turn,
                &Input {
                    text: "like this one",
                    images: &image,
                },
                "s-1",
            )
            .expect("steer")
        {
            Steered::Accepted(request) => request,
            Steered::Stale => panic!("the turn is running"),
        };
        call.answer("rendered", false).expect("answer");
        let ended = end(&mut driver, turn);
        assert!(matches!(ended, Seen::TurnEnd { .. }), "{ended:?}");
        assert!(driver.seen.iter().any(|e| matches!(e,
            Seen::RequestDone { request: r, outcome: Ok(Outcome::Applied), .. } if *r == request)));
        let returned = driver.seen.iter().position(|e| matches!(e,
            Seen::SteerReturned { turn: t, request: r, message_id } if *t == turn && *r == request && message_id == "s-1"));
        let ended_at = driver
            .seen
            .iter()
            .position(|e| matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn));
        assert!(
            returned.is_some() && returned < ended_at,
            "{:#?}",
            driver.seen
        );
        assert!(
            !driver
                .seen
                .iter()
                .any(|e| matches!(e, Seen::SteerJoined { .. }))
        );
        assert!(
            !bodies
                .lock()
                .unwrap()
                .iter()
                .any(|body| body.contains("like this one"))
        );
        // The host sends it as a prompt from its own copy.
        let next = driver
            .runtime
            .prompt_input(
                &Input {
                    text: "like this one",
                    images: &image,
                },
                "s-1",
            )
            .expect("the returned steer as a prompt");
        assert!(matches!(end(&mut driver, next), Seen::TurnEnd { .. }));
        driver.runtime.close();
    });
}
