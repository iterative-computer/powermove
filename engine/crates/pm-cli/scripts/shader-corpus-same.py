#!/usr/bin/env python3
"""Whether two shader gate corpora (`corpus.json` from
engine/crates/pm-render/tools/shaders/collect.ts) hold the same cases: the
Phase 3a gate recollects the corpus at its tip and keeps the shaders lane's
WebGL references only if no case changed (docs/engine/phase3a-gate.md).

    python3 engine/crates/pm-cli/scripts/shader-corpus-same.py <corpus A> <corpus B>

Compares every case's fields except where it was collected from (`source`,
which names paths and line numbers) and the corpus-level creation stamp.
Exit 0 when identical."""
import json
import sys


def cases(path):
    d = json.load(open(f"{path}/corpus.json"))
    return d, {c["id"]: {k: v for k, v in c.items() if k != "source"} for c in d["cases"]}


(da, a), (db, b) = cases(sys.argv[1]), cases(sys.argv[2])
variants = lambda cs: sum(len(c.get("variants", [])) or 1 for c in cs.values())
only_a, only_b = sorted(set(a) - set(b)), sorted(set(b) - set(a))
changed = sorted(k for k in set(a) & set(b) if a[k] != b[k])
same_store = da.get("storeReleases") == db.get("storeReleases")
print(f"A: {len(a)} cases, {variants(a)} variants; B: {len(b)} cases, {variants(b)} variants")
print(f"only in A: {only_a}\nonly in B: {only_b}\nchanged: {changed}\nsame Store releases: {same_store}")
sys.exit(0 if not (only_a or only_b or changed) and same_store else 1)
