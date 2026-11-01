use crate::{Automation, Error, Result, json_value, model};
use pm_kernel::DocumentStore;
use serde_json::{Value, json};
fn count(a: &Value, k: &str, default: usize) -> usize {
    a[k].as_u64().map(|n| n as usize).unwrap_or(default)
}
fn round(n: f64) -> f64 {
    (n * 1000.).round() / 1000.
}
pub fn properties(layer: &Value) -> Vec<(String, Value)> {
    fn collect(v: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
        if let Some(o) = v.as_object() {
            for (k, v) in o {
                if v.get("kf").is_some() {
                    out.push((format!("{prefix}{k}"), v.clone()));
                }
            }
        }
    }
    let mut out = vec![];
    collect(&layer["p"], "", &mut out);
    collect(&layer["d"], "c.", &mut out);
    for k in ["blend", "mblur", "matteMode"] {
        if layer[k].get("kf").is_some() {
            out.push((format!("l.{k}"), layer[k].clone()));
        }
    }
    if let Some(a) = layer["fx"].as_array() {
        for fx in a {
            if fx["on"].get("kf").is_some() {
                out.push((
                    format!("{}.$enabled", fx["id"].as_str().unwrap_or("")),
                    fx["on"].clone(),
                ));
            }
            collect(
                &fx["p"],
                &format!("{}.", fx["id"].as_str().unwrap_or("")),
                &mut out,
            );
        }
    }
    out
}
fn at_path<'a>(root: &'a Value, path: &Value) -> Option<&'a Value> {
    let mut v = root;
    for p in path.as_array()? {
        v = if let Some(k) = p.as_str() {
            v.get(k)?
        } else {
            v.get(p.as_u64()? as usize)?
        };
    }
    Some(v)
}
pub fn project_state(host: &Automation, id: &str, a: &Value) -> Result<Value> {
    let p = host.read_document(id)?;
    let selection = host.selection(id)?;
    let time = host.playhead(id);
    let mut mirror = DocumentStore::new_untitled(model(&p));
    let comp = mirror
        .open_comp()
        .unwrap_or_else(|| format!("@root:{}", p["id"].as_str().unwrap_or("")));
    let layers = p["layers"]
        .as_array()
        .ok_or_else(|| Error::invalid("Missing layers"))?;
    let index_only = a["indexOnly"] == true;
    let offset = count(a, "layerOffset", 0);
    let limit = count(a, "layerLimit", if index_only { 100 } else { 12 })
        .clamp(1, if index_only { 200 } else { 50 });
    let search = a["layerSearch"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let matching = layers
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            (a.get("layerId").is_none() || a["layerId"] == l["id"])
                && (search.is_empty()
                    || l["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&search)
                    || l["id"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&search))
        })
        .collect::<Vec<_>>();
    let prop_offset = count(a, "propertyOffset", 0);
    let prop_limit = count(a, "propertyLimit", 100).max(1);
    let key_offset = count(a, "keyframeOffset", 0);
    let key_limit = count(a, "keyframeLimit", 8);
    let mut page = vec![];
    for (index, l) in matching.iter().skip(offset).take(limit) {
        let mut item = json!({"index":index,"id":l["id"],"name":l["name"],"type":l["type"],"from":l["from"],"duration":l["dur"],"visible":l["on"],"locked":l["lock"],"parent":l["parent"],"group":l["group"].as_str(),"blend":l["blend"],"motionBlur":l["mblur"],"color":l["color"]});
        if !index_only {
            let entries = json_value(
                &mirror
                    .queries()
                    .query(&model(
                        &json!({"op":"allProps","comp":comp,"open":comp,"layer":index,"t":time}),
                    ))
                    .map_err(Error::kernel)?,
            );
            let entries = entries
                .as_array()
                .ok_or_else(|| Error::invalid("Invalid property query result"))?;
            let mut props = vec![];
            for entry in entries.iter().skip(prop_offset).take(prop_limit) {
                let key = entry["key"].as_str().unwrap_or("");
                let prop = entry["path"]
                    .get("value")
                    .or_else(|| at_path(l, &entry["path"]))
                    .cloned()
                    .unwrap_or(Value::Null);
                let keys = prop["kf"].as_array().cloned().unwrap_or_default();
                let value=json_value(&mirror.queries().query(&model(&json!({"op":"evP","comp":comp,"open":comp,"layer":index,"t":time,"key":key,"prop":prop}))).map_err(Error::kernel)?);
                let frames=keys.iter().skip(key_offset).take(key_limit).map(|k|json!({"time":round(k["t"].as_f64().unwrap_or(0.)),"compositionTime":round(l["from"].as_f64().unwrap_or(0.)+k["t"].as_f64().unwrap_or(0.)),"value":k["v"],"hold":k["hold"]==true})).collect::<Vec<_>>();
                props.push(json!({"path":key,"value":value,"keyframeCount":keys.len(),"nextKeyframeOffset":if key_limit>0&&key_offset.saturating_add(key_limit)<keys.len(){Some(key_offset+key_limit)}else{None},"keyframes":frames,"expression":prop["expr"].as_str().filter(|s|!s.is_empty()),"handEdited":l["locked_intent"].get(key).is_some_and(|v|!v.is_null()&&v!=&json!(false))||l["locked_intent"].get(key.split('.').next().unwrap_or("")).is_some_and(|v|!v.is_null()&&v!=&json!(false))}));
            }
            item["content"] = if l["type"] == "captions" {
                caption_digest(l, a)
            } else {
                l.get("d").cloned().unwrap_or(json!({}))
            };
            item["masks"]=json!(l["masks"].as_array().unwrap_or(&vec![]).iter().map(|m|json!({"id":m["id"],"shape":m["shape"],"mode":m["mode"],"hasPath":!m["path"].is_null(),"vertices":m["path"]["vertices"].as_array().map_or(0,Vec::len)})).collect::<Vec<_>>());
            item["propertyCount"] = json!(entries.len());
            item["nextPropertyOffset"] =
                json!(if prop_offset.saturating_add(prop_limit) < entries.len() {
                    Some(prop_offset + prop_limit)
                } else {
                    None
                });
            item["properties"] = json!(props);
            item["effects"] = json!(
                l["fx"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|fx| json!({"id":fx["id"],"type":fx["type"],"enabled":fx["on"]}))
                    .collect::<Vec<_>>()
            );
            item["textLayout"] = Value::Null; // fonts/layout host is a pending engine integration.
        }
        page.push(item);
    }
    let assets = p["assets"]
        .as_object()
        .map(|o| o.values().collect::<Vec<_>>())
        .unwrap_or_default();
    let media = if index_only {
        vec![]
    } else {
        assets.iter().filter(|a|a["kind"]=="video"||a["kind"]=="audio").map(|a|{let mut v=json!({"id":a["id"],"name":a["name"],"kind":a["kind"],"duration":a["dur"],"available":false,"hasAudio":false});for k in ["w","h"]{if let Some(n)=a.get(k){v[if k=="w"{"width"}else{"height"}]=n.clone();}}v}).collect()
    };
    Ok(
        json!({"composition":{"id":p["id"],"name":p["name"],"width":p["w"],"height":p["h"],"fps":p["fps"],"duration":p["dur"],"workArea":p["work"],"background":p.get("backgroundFill").unwrap_or(&p["bg"]),"playhead":round(time),"revision":host.revision(id)?},"layerCount":layers.len(),"mediaAssets":media,"videoEditingTool":"edit_video","threeDEditingTool":"edit_3d","modelAssets":if index_only{vec![]}else{assets.iter().filter(|a|a["kind"]=="model"||a["kind"]=="image").map(|a|{let mut o=json!({"id":a["id"],"name":a["name"],"kind":a["kind"]});if let Some(f)=a.get("format"){o["format"]=f.clone();}o}).collect::<Vec<_>>()},"selection":selection,"layers":page,"parameters":if index_only{json!({})}else{p.get("params").cloned().unwrap_or(json!({}))},"markers":if index_only{json!([])}else{p.get("markers").cloned().unwrap_or(json!([]))},"availableOperations":crate::commands::OPERATIONS,"availableCapabilities":null,"pagination":{"layerOffset":offset,"layerLimit":limit,"matchingLayerCount":matching.len(),"nextLayerOffset":if offset.saturating_add(page.len())<matching.len(){Some(offset+page.len())}else{None},"indexOnly":index_only,"keyframeOffset":key_offset,"propertyOffset":prop_offset,"propertyLimit":prop_limit,"note":"This page is not the whole composition. Discover all targets with indexOnly=true and follow nextLayerOffset until null, or search with layerSearch. Then read layerId and nextPropertyOffset/keyframeOffset for complete target properties. Omitted layers and channels still exist. inputs/powermove-project.json is only the run-start snapshot."}}),
    )
}
fn caption_digest(l: &Value, a: &Value) -> Value {
    let cues = l["d"]["cues"].as_array().cloned().unwrap_or_default();
    let offset = count(a, "cueOffset", 0);
    let limit = count(a, "cueLimit", 200);
    let from = l["from"].as_f64().unwrap_or(0.);
    json!({"style":l["d"].get("style").cloned().unwrap_or(json!({})),"language":l["d"]["language"],"cueCount":cues.len(),"cueOffset":offset,"cueTimes":"composition seconds","cues":cues.iter().skip(offset).take(limit).map(|c|{let mut v=json!({"id":c["id"],"start":round(c["start"].as_f64().unwrap_or(0.)+from),"end":round(c["end"].as_f64().unwrap_or(0.)+from),"text":c["text"]});if let Some(words)=c["words"].as_array().filter(|a|!a.is_empty()){v["words"]=json!(words.len());}v}).collect::<Vec<_>>()})
}
pub fn timeline(host: &Automation, id: &str, a: &Value) -> Result<Value> {
    let p = host.read_document(id)?;
    let layers = p["layers"].as_array().unwrap();
    let offset = count(a, "offset", 0);
    let limit = count(a, "limit", 100);
    let fps = p["fps"].as_f64().unwrap_or(30.);
    let time = host.playhead(id);
    let mut mirror = DocumentStore::new_untitled(model(&p));
    let mut strips = vec![];
    for (i, l) in layers.iter().enumerate().skip(offset).take(limit) {
        let from = l["from"].as_f64().unwrap_or(0.);
        let dur = l["dur"].as_f64().unwrap_or(0.);
        let visible = if l["on"].is_object() {
            json_value(
                &mirror
                    .queries()
                    .evaluate(None, l["id"].as_str().unwrap(), "l.on", time)
                    .map_err(Error::kernel)?,
            )
        } else {
            l["on"].clone()
        };
        let mut v = json!({"id":l["id"],"index":i,"name":l["name"],"type":l["type"],"from":from,"end":from+dur,"duration":dur,"startFrame":(from*fps).round(),"endFrame":((from+dur)*fps).round(),"group":l["group"].as_str(),"parent":l["parent"].as_str(),"locked":l["lock"]==true,"protected":l["locked_intent"].as_object().map(|o|o.iter().filter(|(_,v)|!v.is_null()&&v!=&&json!(false)).map(|(k,_)|k.clone()).collect::<Vec<_>>()).unwrap_or_default(),"visible":visible,"solo":l["solo"]==true,"shy":l["shy"]==true,"color":l["color"]});
        if l["type"] == "video" || l["type"] == "audio" {
            let eval = |key: &str, default: f64, mirror: &mut DocumentStore| -> Result<f64> {
                if l["d"][key].is_object() {
                    let value = json_value(
                        &mirror
                            .queries()
                            .evaluate(None, l["id"].as_str().unwrap(), &format!("c.{key}"), from)
                            .map_err(Error::kernel)?,
                    );
                    Ok(value.as_f64().unwrap_or(default))
                } else {
                    Ok(l["d"][key].as_f64().unwrap_or(default))
                }
            };
            let trim = eval("trim", 0., &mut mirror)?;
            let speed = eval("speed", 1., &mut mirror)?;
            let animated = ["trim", "speed", "sourceTime", "timeRemap"]
                .iter()
                .any(|k| {
                    l["d"][k]["kf"].as_array().is_some_and(|a| !a.is_empty())
                        || l["d"][k]["expr"].as_str().is_some_and(|s| !s.is_empty())
                        || *k == "timeRemap" && l["d"][k] == true
                });
            if let Some(asset) = l["d"].get("asset") {
                v["assetId"] = asset.clone();
            }
            v["sourceIn"] = json!(trim);
            v["speed"] = json!(speed);
            v["animatedTiming"] = json!(animated);
            v["sourceOut"] = if animated {
                Value::Null
            } else {
                json!(trim + dur * speed)
            };
            v["embeddedAudio"] = json!(l["d"]["embeddedAudio"] == true);
        }
        strips.push(v);
    }
    Ok(
        json!({"projectId":p["id"],"revision":host.revision(id)?,"fps":p["fps"],"duration":p["dur"],"playhead":time,"workArea":p["work"],"selection":host.selection(id)?["layers"],"total":layers.len(),"offset":offset,"nextOffset":if offset.saturating_add(limit)<layers.len(){Some(offset+limit)}else{None},"stackOrder":"Topmost first. Each layer has its own strip; overlaps are composited in stack order.","strips":strips}),
    )
}
