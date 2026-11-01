//! Test-only JSON boundary probe. No window, runtime, or user profile.
use pm_remote_dom::{protocol::*, *};
use serde::Deserialize;
use std::io::{self, Read};
#[derive(Deserialize)]
struct Input {
    batch: Batch,
    feedback: Vec<PageMessage>,
}
struct Queue<T>(Vec<T>);
impl<T> TrySink<T> for Queue<T> {
    fn try_send(&mut self, value: T) -> Result<(), Error> {
        self.0.push(value);
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let input: Input = serde_json::from_str(&input)?;
    let mut router = Router::new(input.batch.session.clone(), Limits::default())?;
    let mut page = Queue(vec![]);
    let mut realm = Queue(vec![]);
    router.route_batch(input.batch, &mut page)?;
    for feedback in input.feedback {
        router.route_page(feedback, &mut realm)?;
    }
    println!(
        "{}",
        serde_json::json!({ "batch": page.0[0], "feedback": realm.0, "sent": router.inspect().sent })
    );
    Ok(())
}
