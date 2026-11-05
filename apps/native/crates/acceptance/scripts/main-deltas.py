#!/usr/bin/env python3
"""Read-only delta/provenance check; --write updates this lane's own snapshot."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[5]
REFERENCE = "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
ROUTES = {"A": "acceptance/agent-MCP integration", "P": "delta-data", "E": "delta-fx", "U": "later UI lane"}
BACKINGS = ["provider-presets-model-discovery", "agent-client-config-registration", "claude-auto-permission-classifier", "markdown-local-media-grants", "visual-reply-thread-persistence", "main-mcp-host-integration", "project-memory-index-and-tool", "extension-rebase-appearance", "publication-panel-generation-fences", "provider-review-consent-error-identity"]
PROOFS = {
    "A01": ["main/compatible-provider.ts", "shared/compatible-provider.ts"],
    "A02": ["main/agent-clients.ts"],
    "A03": ["main/claude/adapter.ts", "main/claude/runner.ts"],
    "A04": ["panels/agent/markdown.ts", "panels/agent/work-groups.ts", "main/agent-media.ts"],
    "A05": ["shared/visual-reply.ts"],
    "A06": ["main/agent-tools/spec.ts", "main/agent-tools/external-spec.ts"],
    "A07": ["core/memory.ts", "legacy/assistant/harness.ts"],
    "A08": ["kernel/theme-state.ts", "kernel/workspace-appearance.ts", "main/extensions/rebase.ts"],
    "A09": ["legacy/assistant/panel-tools.ts", "legacy/assistant/harness.ts"],
    "A10": ["legacy/assistant/harness.ts"],
}
text = (ROOT / "docs/native/main-delta.md").read_text()
expanded = text.split("## Expanded change inventory", 1)[1].split("## Complete commit ledger", 1)[0]
rows = []
for line in expanded.splitlines():
    if not re.match(r"^\| [PEUA]\d\d", line):
        continue
    cells = line.strip().strip("|").split(" | ")
    identifier = cells[0].split()[0]
    row = dict(id=identifier, priority=cells[0], commits=cells[1], expected=cells[2],
               coverage_before_reference_change=cells[3], owner=ROUTES[identifier[0]], reference=REFERENCE)
    if identifier.startswith("A"):
        row["blocked_by"] = "main:" + BACKINGS[int(identifier[1:]) - 1]
        row["source_hashes"] = {}
        for relative in PROOFS[identifier]:
            path = "apps/desktop/src/" + (relative if relative.startswith(("main/", "shared/")) else "renderer/src/" + relative)
            source = subprocess.check_output(["git", "show", REFERENCE + ":" + path], cwd=ROOT)
            row["source_hashes"][path] = hashlib.sha256(source).hexdigest()
    rows.append(row)
if len(rows) != 57 or len({row["id"] for row in rows}) != 57:
    raise SystemExit("delta inventory changed; reconcile ownership")
output = json.dumps(rows, indent=2) + "\n"
path = ROOT / "apps/native/crates/acceptance/data/main-deltas.json"
if "--write" in sys.argv:
    path.write_text(output)
elif path.read_text() != output:
    raise SystemExit("delta routes or pinned source provenance drift")
print("57 delta rows and pinned A01–A10 source hashes verified; shared docs read-only")
