#!/usr/bin/env bash
# Validate this owned lane without changing the out-of-lane native lockfile.
set -euo pipefail
repo=$(git rev-parse --show-toplevel)
mkdir -p "$repo/work"
validation_root=$(mktemp -d "$repo/work/pm-promedia.XXXXXXXX")
mkdir -p "$validation_root/apps/native/crates" "$validation_root/engine/crates/pm-color/src"
cp -R "$repo/apps/native/crates/promedia" "$validation_root/apps/native/crates/"
cp "$repo/engine/crates/pm-color/src/lut.rs" "$repo/engine/crates/pm-color/src/scopes.rs" "$validation_root/engine/crates/pm-color/src/"
mkdir -p "$validation_root/engine/crates/pm-media/src"
cp "$repo/engine/crates/pm-media/src/cache_frame.rs" "$repo/engine/crates/pm-media/src/proxy.rs" "$validation_root/engine/crates/pm-media/src/"
# Reuse the exact engine time sources without importing the platform workspace.
mkdir -p "$validation_root/engine/crates/pm-time"
cp -R "$repo/engine/crates/pm-time/src" "$validation_root/engine/crates/pm-time/"
cat > "$validation_root/engine/crates/pm-time/Cargo.toml" <<'TIME'
[package]
name = "pm-time"
version = "0.1.0"
edition = "2024"
TIME
cp "$repo/apps/native/Cargo.lock" "$validation_root/apps/native/Cargo.lock"
cat > "$validation_root/apps/native/Cargo.toml" <<'MANIFEST'
[workspace]
resolver = "3"
members = ["crates/promedia"]
[workspace.package]
version = "0.1.0"
edition = "2024"
license = "AGPL-3.0-or-later"
MANIFEST
cd "$validation_root/apps/native"
# Keep registry dependencies at the seed lock's versions; add the new workspace
# package and prune unreachable packages only in this disposable workspace.
cargo update --quiet --workspace
cargo fmt -p pm-promedia -- --check
cargo test --locked -p pm-promedia
cargo clippy --locked -p pm-promedia --all-targets -- -D warnings
printf 'Validation snapshot: %s\n' "$validation_root"
