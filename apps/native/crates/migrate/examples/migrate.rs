//! CLI deliberately prints only aggregate statistics. Logical exports and
//! migration manifests are private files in the destination, never stdout.
use anyhow::{Context, Result, bail};
use pm_native_migrate::{
    indexeddb, local_storage,
    snapshot::{BlobCopy, Snapshot},
};
use std::{env, path::Path, time::Instant};
fn main() {
    if let Err(e) = run() {
        eprintln!("migration reader failed: {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let a: Vec<String> = env::args().skip(1).collect();
    let start = Instant::now();
    match a.first().map(String::as_str) {
        Some("snapshot-media") if a.len() == 4 || a.len() == 5 => {
            let policy = match a.get(4).map(String::as_str) {
                Some("sample") => BlobCopy::Sample {
                    max_bytes: 32 * 1024 * 1024,
                    max_files: 8,
                },
                None => BlobCopy::All,
                _ => bail!("unknown snapshot mode (expected sample or no mode)"),
            };
            let s = Snapshot::capture(
                Path::new(&a[1]),
                Some(Path::new(&a[2])),
                Path::new(&a[3]),
                policy,
            )?;
            let (files, bytes, copied, copied_bytes) = s.blob_counts();
            println!(
                "blob_files={files} blob_bytes={bytes} copied_files={copied} copied_bytes={copied_bytes}"
            );
        }
        Some("snapshot-local") if a.len() == 3 => {
            Snapshot::capture(Path::new(&a[1]), None, Path::new(&a[2]), BlobCopy::All)?;
        }
        Some("media-stats") if a.len() == 2 => {
            let s = Snapshot::open(Path::new(&a[1]))?;
            let db = indexeddb::read(&s)?;
            let mut media = 0;
            let mut posters = 0;
            let mut bytes = 0u64;
            let mut present = 0;
            let mut present_bytes = 0u64;
            for r in &db.records {
                if r.poster {
                    posters += 1;
                } else {
                    media += 1;
                }
                bytes = bytes
                    .checked_add(r.payload().size)
                    .context("payload byte total overflow")?;
                if let Some(path) = r.payload_path(&s)? {
                    let mut f = std::fs::File::open(path)?;
                    let mut hash = blake3::Hasher::new();
                    let actual = std::io::copy(&mut f, &mut hash)?;
                    anyhow::ensure!(
                        actual == r.payload().size,
                        "sample Blob length differs from descriptor"
                    );
                    let _verified_hash = hash.finalize();
                    present += 1;
                    present_bytes = present_bytes
                        .checked_add(actual)
                        .context("sample byte total overflow")?;
                }
            }
            println!(
                "records={} media={media} posters={posters} bytes={bytes} sampled_payloads={present} sampled_bytes={present_bytes} recovered_tail_bytes={}",
                db.records.len(),
                db.recovered_tail_bytes
            );
        }
        Some("migrate-media") if a.len() == 4 => {
            let s = Snapshot::open(Path::new(&a[1]))?;
            let db = indexeddb::read(&s)?;
            let input: pm_native_migrate::migration::ClosureInput =
                serde_json::from_reader(std::fs::File::open(&a[2])?)?;
            let pass = pm_native_migrate::migration::migrate(
                &s,
                &db.records,
                &input,
                Path::new(&a[3]),
                |_| Ok(()),
            )?;
            println!(
                "records={} references={} blocked_projects={} mapping_conflicts={} cutover_ready={}",
                pass.records.len(),
                pass.closure.len(),
                pass.blocked_owners.len(),
                pass.mapping_conflicts.len(),
                pass.cutover_ready
            );
        }
        Some("local-export") if a.len() == 3 => {
            anyhow::ensure!(
                !Path::new(&a[2]).exists(),
                "export destination already exists"
            );
            let s = Snapshot::open(Path::new(&a[1]))?;
            let count = pm_native_migrate::migration::export_local_storage(&s, Path::new(&a[2]))?;
            println!("exported_records={count}");
        }
        Some("local-stats") if a.len() == 2 => {
            let s = Snapshot::open(Path::new(&a[1]))?;
            let entries = local_storage::read(&s)?;
            println!(
                "records={} value_utf8_bytes={}",
                entries.len(),
                entries.iter().map(|e| e.value.len()).sum::<usize>()
            );
        }
        _ => bail!(
            "usage: migrate snapshot-media LEVELDB BLOBS NEW_SNAPSHOT [sample] | snapshot-local LEVELDB NEW_SNAPSHOT | media-stats SNAPSHOT | local-stats SNAPSHOT | local-export SNAPSHOT NEW_JSON | migrate-media SNAPSHOT CLOSURE_JSON MEDIA_STORE"
        ),
    }
    println!("elapsed_seconds={:.6}", start.elapsed().as_secs_f64());
    Ok(())
}
