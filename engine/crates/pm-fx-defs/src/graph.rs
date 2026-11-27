//! Bounded iterative scheduling for source -> masks -> effects requests.
//! The embedding evaluator expands dependencies at each request's exact time;
//! this scheduler never calls JS or invents a transparent result for a cycle.
use crate::{catalog::CatalogError, inputs::Request};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_FRAME_INPUTS: usize = 256;
pub const MAX_GRAPH_NODES: usize = 65_536;
#[derive(Clone, Debug)]
pub struct Dependency {
    pub request: Request,
    pub input_lease: bool,
}
#[derive(Clone, Debug)]
pub struct InputPlan {
    /// Producer-before-consumer order. Shared requests execute once.
    pub order: Vec<Request>,
    /// Leased external or temporal inputs, in that same execution order.
    pub requests: Vec<Request>,
}

/// Roots follow the caller's authored order. Expansion includes group children,
/// mattes and backdrop prefixes as well as references; their policy stays with
/// the evaluator that knows activation/visibility, not this generic scheduler.
pub fn input_plan(
    roots: impl IntoIterator<Item = Request>,
    mut expand: impl FnMut(&Request) -> Result<Vec<Dependency>, CatalogError>,
) -> Result<InputPlan, CatalogError> {
    struct Frame {
        request: Request,
        deps: Vec<Dependency>,
        at: usize,
    }
    let mut state = BTreeMap::<Request, u8>::new();
    let mut requests = BTreeSet::new();
    let mut order = Vec::new();
    for root in roots {
        if state.get(&root) == Some(&2) {
            continue;
        }
        if state.len() >= MAX_GRAPH_NODES {
            return Err(CatalogError("dependency graph exceeds node limit".into()));
        }
        let deps = expand(&root)?;
        state.insert(root.clone(), 1);
        let mut stack = vec![Frame {
            request: root,
            deps,
            at: 0,
        }];
        while let Some(current) = stack.last_mut() {
            if current.at == current.deps.len() {
                let completed = stack.pop().expect("nonempty stack").request;
                state.insert(completed.clone(), 2);
                order.push(completed);
                continue;
            }
            let dependency = current.deps[current.at].clone();
            current.at += 1;
            if dependency.input_lease {
                requests.insert(dependency.request.clone());
                if requests.len() > MAX_FRAME_INPUTS {
                    return Err(CatalogError(
                        "temporal inputs exceed frame request limit".into(),
                    ));
                }
            }
            match state.get(&dependency.request) {
                Some(2) => continue,
                Some(1) => {
                    let start = stack
                        .iter()
                        .position(|f| f.request == dependency.request)
                        .expect("active request on stack");
                    let cycle = stack[start..]
                        .iter()
                        .map(|f| format!("{:?}", f.request))
                        .chain(std::iter::once(format!("{:?}", dependency.request)))
                        .collect::<Vec<_>>()
                        .join(" -> ");
                    return Err(CatalogError(format!("layer input cycle: {cycle}")));
                }
                _ => {}
            }
            if state.len() >= MAX_GRAPH_NODES {
                return Err(CatalogError("dependency graph exceeds node limit".into()));
            }
            let deps = expand(&dependency.request)?;
            state.insert(dependency.request.clone(), 1);
            stack.push(Frame {
                request: dependency.request,
                deps,
                at: 0,
            });
        }
    }
    let requests = order
        .iter()
        .filter(|r| requests.contains(*r))
        .cloned()
        .collect();
    Ok(InputPlan { order, requests })
}
