# RFC 08: Budget leases

Status: draft, version 0. Module: `pm_protocol::lease`. Gates Phase 2;
freezes when Phase 2 starts. Owner: Engine and platform (leases, section 27).

## Purpose

Main grants each engine process a memory lease from one global budget and
shrinks the job engine's lease during playback (plan section 19, "Memory and
GPU"). Each engine enforces its lease across every category it holds. This
is what keeps an export plus playback on a base 8 GB machine free of
memory-pressure warnings (section 25), and it replaces today's
`memory.ts` budgets and `previewMemoryMiB` (section 14.7).

## Types

| Type | Meaning |
|---|---|
| `Lease` | Process, epoch, total bytes (unified memory: one number across CPU and GPU), `LeaseReason`, acknowledgement deadline |
| `LeaseReason` | Initial, playback started, playback stopped, memory pressure, requested, peer exited |
| `LeaseCategories` | Bytes held in frame cache, render cache, Blender surfaces, 3D geometry, surface pool, decoders, pipelines, other |
| `LeaseUsage` | Process, epoch, total, categories, and whether this report acknowledges the epoch |
| `LeaseRequest` | An engine asking for more, with a reason |
| `LeaseLedger`, `LeaseError` | Host-side tracking of epochs and acknowledgements |

Messages: `Request::GrantLease` (host → engine, answered with `leaseUsage`
once usage fits), `Request::RequestLease` (engine → host, answered with
`lease`), `Event::LeaseUsage` (periodic reports).

## Invariants

1. **Epochs only increase** and only the latest is in force
   (`LeaseLedger::grant` rejects a non-increasing epoch, tested).
2. **A shrink is acknowledged only when usage fits.** The engine evicts and
   answers with `ack: true` and a total within the lease before
   `ack_deadline_ms`; an acknowledgement over the lease is an error, and a
   report for an older epoch is stale (`LeaseLedger::report`, tested).
3. **Totals are honest.** `total` equals the category sum and the sum stays
   in the safe integer range (`LeaseUsage::validate`, tested with overflow).
4. **Reports** go out as the acknowledgement of each change and at least
   every 5 s while the engine is not idle.
5. **Everything counts.** The job engine's duplicated decoders, glTF data,
   and pipelines count against its lease (section 19). Blender's memory is
   outside leases; Blender workers are bounded by measured RSS (contract
   11, host services). `pm-wasm` has its own per-window budget in the
   renderer, outside this contract.

## Versioning

Version 0 until Phase 2 starts. New categories are new fields; after the
freeze they are a version bump even though serde would ignore them, because
the total must keep equaling the sum on both sides.

## What Phase 0 must confirm

- **S8 (GPU contention):** that shrinking the job engine's lease at playback
  start, together with GPU time slicing (contract 13), keeps 4K playback at
  zero drops while the job engine exports and Blender renders.
- **Memory gate tooling** (Parity and QA): that per-category accounting can
  be measured against process RSS and Metal's allocated size on the
  reference machine, so a lease breach is detectable. The base 8 GB gate is
  deferred (decision 11) and reported as unverified.
- **S6:** decoder memory per session for each hardware codec, which sets the
  decoder share of the interactive lease.

## Open questions

1. How the host splits the global budget initially (a fixed fraction of
   physical memory, or measured headroom). Engine and platform decides with
   the memory gate.
2. Whether the acknowledgement deadline should scale with the shrink size.
   Eviction of GPU caches is fast; a large frame cache on disk-backed
   memory may not be.
3. Whether GPU memory needs its own lease on discrete-GPU Windows machines
   after cutover (unified memory makes one number enough on Apple Silicon).
