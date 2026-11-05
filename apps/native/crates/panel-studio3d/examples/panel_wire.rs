//! Synthetic six-op replay; this is not macOS presentation evidence.
use pm_design::{Accessibility, Scheme, Tokens};
use pm_surface::{
    Surface,
    kit::{Layout, Panel, focus_order},
};
use serde_json::json;
use std::{
    io::{self, BufRead},
    sync::Arc,
};
fn main() {
    let mut s = Surface::default();
    for line in io::stdin().lock().lines() {
        match s.apply_json(&line.expect("stdin")) {
            Ok(wire) => {
                let appearances=[Scheme::Light,Scheme::Dark].map(|scheme| {
                let panel=Panel::prepare(Arc::new(s.clone()),&Tokens::new(scheme,Accessibility::default()),&Layout::new(),None,false);
                json!({"diagnostics":panel.diagnostics,"ax":panel.accessibility.nodes.iter().map(|(_,n)|json!({"role":format!("{:?}",n.role()),"label":n.label(),"disabled":n.is_disabled()})).collect::<Vec<_>>()})
            });
                println!(
                    "{}",
                    json!({"accepted":true,"wire":wire,"nodes":s.nodes(),"focusOrder":focus_order(&s),"appearances":appearances})
                );
            }
            Err(e) => println!("{}", json!({"accepted":false,"error":e})),
        }
    }
}
