"""Copy exact sources; register only owned package in a disposable workspace."""
import os
from pathlib import Path
import shutil
import subprocess
import sys

repo = Path(__file__).resolve().parents[5]
target = Path(sys.argv[1]).resolve()
if target.exists() or target == repo or repo in target.parents:
    raise SystemExit("Verification destination must be new and outside the repository")
(target / "apps").mkdir(parents=True)
shutil.copytree(repo / "engine", target / "engine", ignore=shutil.ignore_patterns("target", ".git"))
shutil.copytree(repo / "apps/native", target / "apps/native", ignore=shutil.ignore_patterns("target", ".git"))
shutil.copytree(repo / "tools/trace-recorder", target / "tools/trace-recorder")
# Existing kernel tests run immutable tagged-source oracles. This is read-only.
(target / ".git").symlink_to(repo / ".git", target_is_directory=True)
manifest = target / "apps/native/Cargo.toml"
text = manifest.read_text()
text = text.replace('members = ["spikes/*", "crates/*"]', 'members = ["crates/kernel", "crates/panel-inspector"]')
manifest.write_text(text)
work = manifest.parent
# The first check registers the local package in the COPY of the shared lock.
# All registry dependency versions are inherited from the existing lockfile.
subprocess.run(["cargo", "check", "--offline", "-p", "pm-panel-inspector"], cwd=work, check=True)
subprocess.run(["cargo", "test", "--locked", "-p", "pm-panel-inspector", "-p", "pm-kernel"], cwd=work, check=True)
subprocess.run(["cargo", "clippy", "--locked", "-p", "pm-panel-inspector", "--all-targets", "--no-deps", "--", "-D", "warnings"], cwd=work, check=True)
subprocess.run(["cargo", "fmt", "-p", "pm-panel-inspector", "--check"], cwd=work, check=True)
