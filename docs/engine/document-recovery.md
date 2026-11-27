# Synchronous compatibility during utility recovery

N7 preserves deprecated extension APIs at versions 1–3 through a brief native
utility restart. The owner rule prohibits routing evaluation or rendering to
the legacy engine; it permits a renderer copy of document state.

The renderer keeps its existing independent, revision-stamped barrier shadow.
It does not allocate another recovery snapshot. Raw project and layer identities,
pending writes, the evaluator facade, prepared batch arrays, and public handles
survive a generation change. Structural reads remain available from the committed
document state. A mirror divergence is different from a utility crash: it still
requires checked repair from the authoritative raw project.

Main retains one owner-private canonical checkpoint on disk and an ordered,
bounded patch suffix. It records successful writes before acknowledging them.
A write whose utility acknowledgment is lost is retried against the restored
last acknowledged revision, so it is applied once. Calls queue in owner order
while the utility restarts. Native evaluation waits for restoration, with a
120-second bound, without requiring an asynchronous renderer task to run.

The patch suffix is compacted after 128 batches or 8 MiB. The native reader
streams an immutable revision to a private file in bounded chunks. Main accepts
the file before replacing its previous checkpoint and deleting the old suffix.
Definitions, fonts, registry answers, and seeds are retained as replaceable disk
records. Only live interning addresses and small file metadata remain in main's
heap. Public handle numbers are translated to the restarted evaluator's handles;
released holes cannot redirect a caller to a different property.

Synchronous native snapshots carry an owner-fenced document reference even if
the renderer has not received its old viewer port's close event. Main restores
and adopts the shared tree before rendering. No whole-document JSON is sent with
those snapshots. Evaluation and rendering continue in the native utility.

Owner navigation, renderer exit, document replacement, and normal shutdown remove
private recovery files. Utility restart preserves them; app startup uses a fresh
cache lifecycle. Recovery tests compare a deterministic extension's synchronous
read/write/evaluation/render loop across SIGKILL with an uninterrupted run,
including exact final document bytes and frame hashes.

The preload copies the snapshot request field by field. It must forward the
document's `nativeKernel` flag: without it, main answered every synchronous
render of a native document with `needsDocument`, even without a crash.
`e2e/engine-document-recovery.spec.ts` runs the same deterministic v1–3 loop
twice: once uninterrupted, and once with the interactive utility SIGKILLed
mid-loop. The first write, evaluation, and render after the kill run before
the renderer has seen the port close (2026-10-22: the same generation,
disconnected, 119 ms). Both runs end with 24 ordered edits, zero exceptions
or fallbacks, equal revisions and frame hashes, and byte-identical final
documents (four passes).

Documents now arrive as JSON text in any member order (see "Open-time
storage writes and preparation" in host-processes.md). Main's recovery copy
of such a document keeps the hash of its bytes and reloads it as text; the
first load answers the tree's canonical hash, which activation and later
restores use. Checkpoints the utility persists are canonical again.
`document-recovery.test.ts` restores a text-ingested document across a kill.

A resync base is a text checkpoint too. The barrier's resync JSON is the
canonical project form in JSON.stringify's member order, not canonical
bytes, so a recovery copy that claimed its byte hash as the canonical hash
could never be restored (the utility refused the load). It keeps the byte
hash and reloads as text; `document-recovery.test.ts` restores a resync
base whose members are out of canonical order across two kills, against a
fixture that checks canonical loads by the canonical hash of the parsed tree.
