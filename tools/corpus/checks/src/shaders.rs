//! Real compiler diagnostics; no fallback-capability inference from an error string.
use pm_glsl::{
    Failure, FailureStage,
    compiler::{Pool, Request, Response},
};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

pub fn compile(helper: &Path, source: &str, offset: u32) -> Result<(), Failure> {
    let pool = Pool::new(helper.into(), 1);
    let response = pool.run(
        Request::Translate {
            fragment: source.into(),
            line_offset: offset,
        },
        Duration::from_secs(20),
    )?;
    let Response::Program(program) = response else {
        return Err(Failure::new(
            FailureStage::CompilerProcess,
            "unexpected translate response",
        ));
    };
    for module in [program.vertex.module, program.fragment.module] {
        match pool.run(Request::DiagnosticMsl(module), Duration::from_secs(20))? {
            Response::Msl(source, _) if !source.is_empty() => {}
            _ => {
                return Err(Failure::new(
                    FailureStage::CompilerProcess,
                    "unexpected MSL response",
                ));
            }
        }
    }
    Ok(())
}

pub fn report(helper: &Path, definitions: &Value) -> Value {
    Value::Array(definitions["definitions"].as_array().unwrap().iter().filter_map(|d| {
        let source = d["glsl"]["source"].as_str()?;
        let offset = d["glsl"]["bodyLineOffset"].as_u64().unwrap_or(0) as u32;
        Some(match compile(helper, source, offset) {
            Ok(()) => json!({"id":d["id"],"status":"pass"}),
            Err(f) => json!({"id":d["id"],"status":"fail","stage":f.stage.name(),"message":f.message,"diagnostics":f.diagnostics}),
        })
    }).collect())
}
