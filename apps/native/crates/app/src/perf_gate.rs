//! Opt-in, disposable-profile measurement seam. No listener or polling in normal runs.
use crate::session::Controller;
use gpui::{App, Entity};
use pm_protocol::{clock::TransportCommand, common::PlaybackRate};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn reply(value: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "PM_PERF {}", value);
    let _ = out.flush();
}

fn output_json(o: pm_engine_service::Output) -> Value {
    json!({"width":o.width,"height":o.height,"display":o.display,"headroom":o.headroom})
}

fn temporary(path: &std::path::Path) -> bool {
    [std::env::temp_dir(), std::path::PathBuf::from("/tmp")]
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| path.starts_with(&root) && path != root)
}

pub fn install(entity: &Entity<Controller>, cx: &mut App) {
    if std::env::var("PM_PERF_GATE").as_deref() != Ok("1") {
        return;
    }
    let profile = entity.read(cx).profile.canonicalize().ok();
    let explicit = std::env::args_os().any(|arg| arg == "--profile")
        || std::env::var_os("PM_NATIVE_PROFILE").is_some();
    if !explicit || !profile.as_deref().is_some_and(temporary) {
        reply(json!({"error":"perf gate requires an explicit temporary profile"}));
        cx.quit();
        return;
    }
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send_blocking(line).is_err() {
                break;
            }
        }
    });
    // This marks controller availability, NOT first interactive/presented frame.
    reply(json!({"ready":true,"endpoint":"controller-ready","pid":std::process::id()}));
    let weak = entity.downgrade();
    cx.spawn(async move |cx| {
        while let Ok(line) = rx.recv().await {
            let request = serde_json::from_str::<Value>(&line);
            if weak.update(cx, |v, cx| {
                let now = pm_engine_service::trace::now();
                let result = (|| -> anyhow::Result<Value> {
                    let r = request?;
                    let op = r["op"].as_str().unwrap_or("");
                    let r_layer = r["layer"].as_str().map(str::to_owned);
                    match op {
                        "status" => {},
                        "coedit_metrics" => {},
                        "coedit_begin" => v.host.borrow_mut().set_agent_status(pm_shell::AgentStatus {
                            working:true,progress:Some("Synthetic agent perf gate".into()),..Default::default()
                        }),
                        "coedit_end" => v.host.borrow_mut().set_agent_status(pm_shell::AgentStatus::default()),
                        "coedit_load" => {
                            let project=v.session.as_ref().ok_or_else(||anyhow::anyhow!("open a temporary 1080p project first"))?.project.clone();
                            let layers:Vec<_>=v.layers().iter().filter(|l|!l.effectively_locked).take(6).map(|l|l.id.clone()).collect();
                            anyhow::ensure!(!layers.is_empty(),"no editable synthetic layers");
                            let kernel=v.kernel.clone();let host=v.mcp.clone();
                            std::thread::spawn(move || {
                                let started=std::time::Instant::now();
                                for index in 0..40 {
                                    let layer=layers[index%layers.len()].clone();
                                    if let Some(publication)=kernel.published(&project) {
                                        let time=index as f64/20.;
                                        let mut property=pm_kernel::commands::properties::SetProperty::new(layer.clone(),"opacity",30.+((index/layers.len())%2)as f64,time);
                                        property.mode=pm_kernel::commands::properties::KeyMode::Keyframe;
                                        let request=pm_kernel::Request::new(pm_kernel::Caller::realm(90_001,1,pm_kernel::Origin::Agent),pm_kernel::Call::Transact {
                                            label:Some("Synthetic coedit perf".into()),expected_revision:Some(publication.revision()),
                                            body:vec![pm_kernel::Command::Property(pm_kernel::PropertyCommand::Set(property))].into(),
                                        }).with_document(publication.snapshot.doc_id).outside_revision(publication.revision());
                                        let _=kernel.call(&project,request).wait();
                                        host.publish_presence(pm_mcp::presence::Presence {
                                            project:project.clone(),run:"synthetic-perf".into(),step:index as u64,kind:pm_mcp::presence::Kind::Key,
                                            layer:Some(layer),property:Some("opacity".into()),times:vec![time],
                                        });
                                    }
                                    let next=started+std::time::Duration::from_millis((index as u64+1)*50);
                                    std::thread::sleep(next.saturating_duration_since(std::time::Instant::now()));
                                }
                            });
                        },

                        "focus" => {
                            v.host.borrow().window.makeKeyAndOrderFront(None);
                            let app=objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
                            #[allow(deprecated)]
                            app.activateIgnoringOtherApps(true);
                        },
                        "open" => {
                            let path = std::path::PathBuf::from(r["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?);
                            let path = path.canonicalize()?;
                            anyhow::ensure!(temporary(&path), "temporary corpus copy required");
                            v.open_file(path, cx);
                        },
                        "seek" => v.seek_at(r["seconds"].as_f64().ok_or_else(|| anyhow::anyhow!("missing seconds"))?, now),
                        "edit" => {
                            let layer = v.layers().iter().find(|l| !l.effectively_locked && l.kind != "audio").ok_or_else(|| anyhow::anyhow!("no editable layer"))?;
                            v.call_at(pm_kernel::Call::Transact {
                                label: None, expected_revision: None,
                                body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::SetStaticOpacity {
                                layer_id: layer.id.clone(),
                                value: r["value"].as_f64().ok_or_else(|| anyhow::anyhow!("missing value"))?,
                            }]),
                            }, Some(now), cx);
                        },
                        "play" | "pause" => {
                            let viewer = v.viewer.as_ref().ok_or_else(|| anyhow::anyhow!("no viewer"))?;
                            if op == "play" {
                                viewer.command(TransportCommand::SetLoop {
                                    range: Some(pm_protocol::common::TickRange {
                                        start: pm_protocol::common::Ticks::ZERO,
                                        end: pm_protocol::common::Ticks::new((v.duration() * pm_protocol::common::TICKS_PER_SECOND as f64) as i64)?,
                                    }),
                                })?;
                            }
                            viewer.command(if op == "play" { TransportCommand::Play { rate: PlaybackRate::NORMAL } } else { TransportCommand::Pause })?;
                        },
                        "close" => v.request_close(crate::lifecycle::Intent::Window, cx),
                        "output" => {
                            let mode = r["mode"].as_str().and_then(crate::viewer2::OutputMode::parse)
                                .ok_or_else(|| anyhow::anyhow!("mode must be sdr, pq or hlg"))?;
                            v.set_output_mode(mode, cx);
                        },
                        "overlays" => {
                            let safe = r["safe"].as_bool().unwrap_or(v.safe_areas);
                            let guides = r["guides"].as_bool().unwrap_or(v.guides);
                            v.set_overlays(safe, guides, cx);
                        },
                        "popout" => {
                            if r["open"].as_bool().unwrap_or(v.popout.is_none()) != v.popout.is_some() {
                                v.toggle_popout(cx);
                            }
                        },
                        "move" => {
                            let screen = r["screen"].as_u64().ok_or_else(|| anyhow::anyhow!("missing screen"))? as usize;
                            if r["window"].as_str() == Some("popout") {
                                let p = v.popout.as_ref().ok_or_else(|| anyhow::anyhow!("no popout"))?;
                                crate::viewer2::move_to_screen(&p.window, screen)?;
                            } else {
                                crate::viewer2::move_to_screen(&v.host.borrow().window, screen)?;
                            }
                        },
                        "screens" => {},
                        // Paint one frame now, even while the window is occluded
                        // (locked screen): GPUI's displayLayer: renders synchronously.
                        // Deferred so the frame callback never runs inside this update.
                        "redraw" => {
                            let view = v.host.borrow().gpui.clone();
                            // AppKit chrome (toolbar, tab bar) too: occluded windows skip display.
                            let w = v.host.borrow().window.clone();
                            if let Some(frame) = w.contentView().and_then(|c| unsafe { c.superview() }) {
                                frame.layoutSubtreeIfNeeded();
                            }
                            w.display();
                            cx.spawn(async move |_, _| unsafe {
                                let layer: *mut objc2::runtime::AnyObject = objc2::msg_send![&*view, layer];
                                if !layer.is_null() {
                                    let _: () = objc2::msg_send![layer, setNeedsDisplay];
                                    let _: () = objc2::msg_send![layer, displayIfNeeded];
                                }
                            }).detach();
                        },
                        // `path`: Save As to it with no panel (the panel runs
                        // out of process, so a background check can't drive it).
                        "save" => {
                            let path = match r["path"].as_str() {
                                Some(path) => {
                                    let path = std::path::PathBuf::from(path);
                                    let folder = path.parent().and_then(|f| f.canonicalize().ok());
                                    anyhow::ensure!(folder.as_deref().is_some_and(temporary), "temporary destination required");
                                    Some(path)
                                }
                                None => None,
                            };
                            v.save(path, cx)
                        },
                        // Diagnostics: the window's AppKit view tree (class, frame, hidden).
                        "views" => {
                            let w = v.host.borrow().window.clone();
                            let mut out = vec![];
                            fn walk(view: &objc2_app_kit::NSView, depth: usize, out: &mut Vec<String>) {
                                let f = view.frame();
                                out.push(format!("{}{} {:.0},{:.0} {:.0}x{:.0}{}", "  ".repeat(depth), view.class().name().to_string_lossy(), f.origin.x, f.origin.y, f.size.width, f.size.height, if view.isHidden() { " hidden" } else { "" }));
                                if depth < 7 {
                                    for s in view.subviews().to_vec() { walk(&s, depth + 1, out); }
                                }
                            }
                            let app = objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
                            for win in app.windows().to_vec() {
                                out.push(format!("WINDOW {} visible={} tabs={}", win.windowNumber(), win.isVisible(), win.tabbedWindows().map_or(0, |t| t.count())));
                                if let Some(frame) = win.contentView().and_then(|c| unsafe { c.superview() }) {
                                    walk(&frame, 0, &mut out);
                                }
                            }
                            reply(json!({"views": out, "window": w.windowNumber(), "visible": w.isVisible(), "key": w.isKeyWindow(), "selected": w.tabGroup().and_then(|g| g.selectedWindow()).map(|s| s.windowNumber())}));
                        },
                        "quit" => cx.quit(),
                        // Shell command by name (`palette`, `layout:save`, `shellTest:drag`, …).
                        "shell" => {
                            let name = r["command"].as_str().ok_or_else(|| anyhow::anyhow!("missing command"))?;
                            let args = r["args"].as_array().cloned().unwrap_or_default();
                            v.host.borrow().send_command(name, args);
                        },
                        _ => anyhow::bail!("unknown perf operation"),
                    }
                    let coedit=(op=="coedit_metrics").then(|| {
                        let mut usage:libc::rusage=unsafe{std::mem::zeroed()};
                        unsafe{libc::getrusage(libc::RUSAGE_SELF,&mut usage);}
                        let seconds=|t:libc::timeval|t.tv_sec as f64+t.tv_usec as f64/1_000_000.;
                        let timeline=v.builtins.timeline.as_ref().map(|t|t.read(cx).agent_ink_metrics());
                        json!({"shell_frames":v.host.borrow().agent_effect_frames(),"timeline":timeline,
                            "process_cpu_s":seconds(usage.ru_utime)+seconds(usage.ru_stime)})
                    });
                    let status = v.viewer.as_ref().map(|v| v.status());
                    let frame = status.as_ref().and_then(|s| s.frame.as_ref());
                    let mut out = json!({
                        "accepted_host_s":now, "window_key":v.host.borrow().window.isKeyWindow(), "window_visible":v.host.borrow().window.isVisible(), "opening":v.opening,
                        "opened":v.session.is_some(), "message":v.message,
                        "revision":v.published.as_ref().map(|p| p.revision()),
                        "viewer_id":v.viewer.as_ref().map(|v| v.identity()),
                        "frame":frame.map(|f| json!({"input_host_s":f.input_host_seconds,"revision":f.stamp.revision,"doc_id":f.stamp.doc_id,"ticks":f.time.get(),"stamp":format!("{:?}",f.stamp),
                            "cache_hit":f.cache_hit,"gpu_ok":f.gpu_ok,
                            "display_host_s":f.display_host_seconds,"completion_host_s":f.completion_host_seconds})),
                        "error":status.as_ref().and_then(|s| s.error.as_ref()),
                        "playing":status.as_ref().and_then(|s| s.clock).map(|c| c.state != pm_protocol::clock::TransportState::Stopped),
                        "skipped_targets":status.as_ref().map(|s| s.skipped_targets),
                        "no_drawable":status.as_ref().map(|s| s.no_drawable),
                        "held_frames":status.as_ref().map(|s| s.held_frames),
                        "metal_allocated_bytes":v.engine.metal_device().current_allocated_size(),
                        "duration":v.duration(),
                    });
                    // Viewer-2 / lifecycle fields (a second object keeps json! shallow).
                    // Shell 3 checks: toolbar title/subtitle, native tab group, AccentColor.
                    let shell = {
                        let h = v.host.borrow();
                        let (title, subtitle) = h.document_title();
                        let (tabs, tab_bar) = h.tab_info();
                        let hex = |c: Option<u32>| c.map(|c| format!("#{c:06X}"));
                        json!({"title":title,"subtitle":subtitle,"tabs":tabs,"tab_bar":tab_bar,
                            "accent_light":hex(pm_shell::system_accent(pm_design::Scheme::Light)),
                            "accent_dark":hex(pm_shell::system_accent(pm_design::Scheme::Dark))})
                    };
                    let more = json!({
                        "coedit":coedit,
                        "layer_count":v.layers().len(),
                        "has_layer":r_layer.as_deref().map(|id| v.layers().iter().any(|l| l.id == id)),
                        "saving":v.published.as_ref().map(|p| p.saving),
                        "save_pending":v.save_pending,
                        "dirty":v.published.as_ref().map(|p| p.dirty),
                        "output_mode":v.output_mode.name(),
                        "safe_areas":v.safe_areas,
                        "guides":v.guides,
                        "main_output":v.main_output.get().map(output_json),
                        "screens":(op == "screens").then(crate::viewer2::screens),
                        "popout":v.popout.as_ref().map(|p| {
                            let status = p.viewer.borrow().as_ref().map(|v| v.status());
                            json!({
                                "output":p.output.get().map(output_json),
                                "frames":status.as_ref().and_then(|s| s.frame.as_ref()).map(|f| json!({"ticks":f.time.get(),"gpu_ok":f.gpu_ok,"display_host_s":f.display_host_seconds,"revision":f.stamp.revision,"generation":f.stamp.generation})),
                                "error":status.as_ref().and_then(|s| s.error.clone()),
                                "playing":status.as_ref().and_then(|s| s.clock).map(|c| c.state != pm_protocol::clock::TransportState::Stopped),
                                "stale_rejections":status.as_ref().map(|s| s.stale_rejections),
                                "no_drawable":status.as_ref().map(|s| s.no_drawable),
                            })
                        }),
                        "stale_rejections":status.as_ref().map(|s| s.stale_rejections),
                        "represented":status.as_ref().map(|s| s.represented),
                        "generation":frame.map(|f| f.stamp.generation),
                        "shell": shell,
                    });
                    if let (Value::Object(base), Value::Object(more)) = (&mut out, more) {
                        base.extend(more);
                    }
                    Ok(out)
                })();
                match result {
                    Ok(value) => reply(value),
                    Err(error) => reply(json!({"error":error.to_string()})),
                }
            }).is_err() { break; }
        }
    }).detach();
}
