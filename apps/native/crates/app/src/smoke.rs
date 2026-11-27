use anyhow::{Context, Result, ensure};
use pm_engine_service::{Headless, Scene};
use pm_kernel::{persistence::DiskFormat, session::*, *};
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_render::text::host::RenderFonts;
use pm_text::host::{FontBroker, FontService};
use std::{path::PathBuf, sync::mpsc, time::Duration};

/// Real corpus reads, kernel edits, PMV writes and engine frames. All writes temporary.
pub fn run(manifest: PathBuf) -> Result<()> {
    let broker = FontBroker::new();
    // The command-line host pumps on main; a wake marker admits broker asks.
    broker.set_wake(std::sync::Arc::new(|| {}));
    let (tx, rx) = mpsc::channel();
    let fonts = RenderFonts::new(broker.clone());
    let worker = std::thread::spawn(move || {
        let _ = tx.send(corpus(manifest, fonts));
    });
    let mut source = FontService::new().map_err(anyhow::Error::msg)?;
    let result = loop {
        pm_text::host::serve(&mut source, broker.take_asks());
        match rx.try_recv() {
            Ok(result) => break result,
            Err(mpsc::TryRecvError::Disconnected) => anyhow::bail!("Smoke worker stopped"),
            Err(_) => std::thread::sleep(Duration::from_millis(2)),
        }
    };
    broker.close();
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("Smoke worker panicked"))?;
    result
}
fn shell(k: &KernelHandle, c: ShellCall) -> Result<ShellReply> {
    k.shell(c)
        .wait()
        .context("Kernel stopped")?
        .map_err(Into::into)
}
fn corpus(manifest: PathBuf, fonts: RenderFonts) -> Result<()> {
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let rows = manifest["projects"]
        .as_array()
        .context("manifest needs projects array")?;
    let temp = tempfile::tempdir()?;
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(temp.path().join("journals")),
        ..Default::default()
    });
    let kernel = actor.handle();
    shell(
        &kernel,
        ShellCall::AttachLibrary {
            profile: temp.path().join("profile"),
            format: DiskFormat::V1_2,
            electron_profile: None,
        },
    )?;
    let mut headless = Headless::with_fonts(fonts)?;
    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    for (index, row) in rows.iter().enumerate() {
        if row["extensions"].as_array().is_some_and(|a| !a.is_empty()) {
            skipped += 1;
            continue;
        }
        let result = (|| -> Result<()> {
            let source = PathBuf::from(row["path"].as_str().context("project path missing")?);
            let id = format!("smoke-{index}");
            let ShellReply::Opened { session, .. } = shell(
                &kernel,
                ShellCall::Open {
                    id: id.clone(),
                    source: SessionSource::File(source.clone()),
                    electron_profile: None,
                },
            )?
            else {
                anyhow::bail!("No session")
            };
            let p = kernel
                .published(&session.project)
                .context("No publication")?;
            let comp = p.snapshot.open_comp().context("No open composition")?;
            let root = p.snapshot.document.root().context("No document")?;
            let c = pm_kernel::model::comp_node(root, Some(&comp)).context("No composition")?;
            let duration = c.get("dur").and_then(crate::node_number).unwrap_or(1.);
            let mut media = crate::media::for_session(
                &p.snapshot,
                &p.media,
                p.path.as_deref(),
                &temp.path().join(format!("media-{index}")),
            )?
            .catalog;
            if let Some(paths) = row["media"].as_object() {
                for (id, path) in paths {
                    media.originals.insert(
                        id.clone(),
                        PathBuf::from(path.as_str().context("Bad media path")?),
                    );
                }
            }
            let scene = Scene {
                doc: p.snapshot.document.clone(),
                comp,
                media: std::sync::Arc::new(media.export_paths(false)),
                content: p.snapshot.doc_id,
            };
            let size = Size::new(320, 180);
            for seconds in [0., duration / 2.] {
                let pixels = headless.frame(
                    &scene,
                    Ticks::new((seconds * TICKS_PER_SECOND as f64) as i64)?,
                    size,
                    &Default::default(),
                )?;
                ensure!(
                    pixels.len() == (size.width * size.height) as usize,
                    "Incomplete frame"
                );
                ensure!(
                    pixels.iter().flatten().all(|v| v.is_finite()),
                    "Nonfinite frame"
                );
            }
            let mut query = QueryMirror::new(&p);
            let layer = query
                .queries()
                .layers(None)
                .into_iter()
                .find(|l| !l.effectively_locked && l.kind != "audio")
                .context("No editable visual layer")?;
            let original = query
                .queries()
                .evaluate(None, &layer.id, "opacity", 0.)?
                .as_f64()
                .context("Non-numeric opacity")?;
            let value = if original > 50. { 37. } else { 73. };
            let ack = kernel.call_blocking(
                &session.project,
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: Some("Smoke opacity".into()),
                        body: WriteBody::Commands(vec![Command::SetStaticOpacity {
                            layer_id: layer.id.clone(),
                            value,
                        }]),
                        expected_revision: Some(p.revision()),
                    },
                ),
            )?;
            ensure!(
                !matches!(ack.reply, Reply::Throw(_)),
                "Opacity edit failed: {:?}",
                ack.reply
            );
            let save = temp.path().join(format!("roundtrip-{index}.pmv"));
            ensure!(
                matches!(
                    shell(
                        &kernel,
                        ShellCall::SaveAs {
                            session: session.clone(),
                            path: save.clone(),
                            replace: false,
                        }
                    )?,
                    ShellReply::Saved {
                        recovery_copy: None,
                        ..
                    }
                ),
                "Save fell back to recovery"
            );
            shell(&kernel, ShellCall::Close(session))?;
            let ShellReply::Opened { session, .. } = shell(
                &kernel,
                ShellCall::Open {
                    id: format!("reopen-{index}"),
                    source: SessionSource::File(save),
                    electron_profile: None,
                },
            )?
            else {
                anyhow::bail!("Reopen failed")
            };
            let p = kernel
                .published(&session.project)
                .context("No reopened publication")?;
            let mut q = QueryMirror::new(&p);
            // Static edit is verified in the stored channel; animated keys may still win evaluated opacity.
            let root = p.snapshot.document.to_value().context("No reopened root")?;
            let comp = root
                .get("comps")
                .and_then(|v| v.get(p.snapshot.open_comp().as_deref().unwrap_or("")))
                .context("Missing reopened composition")?;
            let l = comp
                .get("layers")
                .and_then(|v| v.as_array())
                .and_then(|a| {
                    a.iter()
                        .find(|l| l.get("id").and_then(|v| v.as_str()) == Some(layer.id.as_str()))
                })
                .context("Layer lost")?;
            let stored = l
                .get("p")
                .and_then(|v| v.get("opacity"))
                .and_then(|v| v.get("v"))
                .and_then(|v| v.as_f64())
                .context("Opacity lost")?;
            ensure!((stored - value).abs() < 1e-9, "Opacity did not round-trip");
            ensure!(
                q.queries().layers(None).iter().any(|l| l.id == layer.id),
                "Layer identity lost"
            );
            shell(&kernel, ShellCall::Close(session))?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                passed += 1;
                println!("PASS project {}", index + 1);
            }
            Err(e) => {
                failed += 1;
                eprintln!("FAIL project {}: {e:#}", index + 1);
            }
        }
    }
    actor.shutdown();
    println!(
        "Gate P1 smoke: {passed} passed, {failed} failed, {skipped} extension-dependent skipped, {} total",
        rows.len()
    );
    ensure!(failed == 0, "Gate P1 smoke has {failed} failures");
    Ok(())
}
