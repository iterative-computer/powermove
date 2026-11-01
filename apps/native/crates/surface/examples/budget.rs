//! CPU-only diagnostic: 1,000 native nodes (500 buttons + 500 text leaves).
use pm_surface::Surface;
use serde_json::json;
use std::time::Instant;
fn main() {
    let mut s = Surface::default();
    let mut ops = vec![];
    for index in 0..500u64 {
        let id = index * 2 + 1;
        ops.extend([
            json!({"op":"create","id":id,"type":"button"}),
            json!({"op":"create","id":id+1,"type":"text"}),
            json!({"op":"setText","id":id+1,"text":"0"}),
            json!({"op":"insertBefore","parent":id,"id":id+1,"before":null}),
            json!({"op":"insertBefore","parent":0,"id":id,"before":null}),
        ]);
    }
    s.apply_json(&json!({"baseRevision":0,"ops":ops}).to_string())
        .unwrap();
    let mut samples = vec![];
    for rev in 1..=130 {
        let mut ops = vec![];
        for index in 0..500u64 {
            let index = if rev % 2 == 1 { 499 - index } else { index };
            let id = index * 2 + 1;
            ops.extend([
                json!({"op":"insertBefore","parent":0,"id":id,"before":null}),
                json!({"op":"setText","id":id+1,"text":rev.to_string()}),
                json!({"op":"setProps","id":id,"patch":{"data-value":rev}}),
                json!({"op":"setListeners","id":id,"flags":{"click":{"bubble":true}}}),
            ]);
        }
        let wire = json!({"baseRevision":s.revision(),"ops":ops}).to_string();
        let start = Instant::now();
        s.apply_json(&wire).unwrap();
        if rev > 30 {
            samples.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{}",
        json!({"nativeNodes":1000,"samples":samples.len(),"p50Ms":samples[49],"p95Ms":samples[94],"scope":"JSON decode, whole-commit validation, staging, publication; no renderer or IPC"})
    );
}
