# Agent lane edits outside the original ownership list

This inventory includes every file touched by non-merge commits on the agent
lane, including edits subsequently reverted or deleted, plus Round 4 work.
Integration merges retain their owning lanes’ changes and are not attributed to
this lane. Paths in the original ownership list are omitted. Test and build
hooks are included even where they are necessary to validate owned code.
No changes to `docs/rust-engine.md` were made by this lane.

| File | Justification |
| --- | --- |
| `.gitignore` | Keep assembled CLI native binaries and build artifacts out of Git. |
| `apps/desktop/e2e/document-host-contract.spec.ts` | Required desktop/hidden document, import, workspace, pins or playback-observation gate; uses production tools. |
| `apps/desktop/e2e/engine-agent-observation.spec.ts` | Required desktop/hidden document, import, workspace, pins or playback-observation gate; uses production tools. |
| `apps/desktop/playwright.prebuilt.config.ts` | Run already-built app contracts so the Electron lease never covers application builds. |
| `apps/desktop/scripts/build-serve.mjs` | Build or exercise the native CLI/desktop host, background service and parity gates requested for this lane. |
| `apps/desktop/scripts/test-mcp.mjs` | Build or exercise the native CLI/desktop host, background service and parity gates requested for this lane. |
| `apps/desktop/scripts/test-native-hidden-parity.mjs` | Build or exercise the native CLI/desktop host, background service and parity gates requested for this lane. |
| `apps/desktop/scripts/test-native-hidden-services.mjs` | Build or exercise the native CLI/desktop host, background service and parity gates requested for this lane. |
| `apps/desktop/scripts/test-native-import-contract.mjs` | Build or exercise the native CLI/desktop host, background service and parity gates requested for this lane. |
| `apps/desktop/src/engine/document-host/import-media.test.ts` | Contract 15 host media-import/service integration and metadata/status parity; worker remains authoritative for documents and transactions. |
| `apps/desktop/src/engine/document-host/import-media.ts` | Contract 15 host media-import/service integration and metadata/status parity; worker remains authoritative for documents and transactions. |
| `apps/desktop/src/engine/document-host/media.test.ts` | Contract 15 host media-import/service integration and metadata/status parity; worker remains authoritative for documents and transactions. |
| `apps/desktop/src/engine/document-host/media.ts` | Contract 15 host media-import/service integration and metadata/status parity; worker remains authoritative for documents and transactions. |
| `apps/desktop/src/engine/document-host/pool.ts` | Contract 15 host media-import/service integration and metadata/status parity; worker remains authoritative for documents and transactions. |
| `apps/desktop/src/engine/document-host/protocol.ts` | Contract 15 host media-import/service integration and metadata/status parity; worker remains authoritative for documents and transactions. |
| `apps/desktop/src/engine/document-host/worker.ts` | Contract 15 host media-import/service integration and metadata/status parity; worker remains authoritative for documents and transactions. |
| `apps/desktop/src/main/engine/agent-frames.test.ts` | Expose revision-pinned snapshot batches to every agent capture entry point, with matching IPC tests. |
| `apps/desktop/src/main/engine/agent-frames.ts` | Expose revision-pinned snapshot batches to every agent capture entry point, with matching IPC tests. |
| `apps/desktop/src/main/engine/agent-jobs.test.ts` | Regression coverage for owned job batching, cancellation, exports and host packing. |
| `apps/desktop/src/main/engine/definitions.ts` | Expose the winning definition snapshot for the persisted CLI profile set; no registration selection change. |
| `apps/desktop/src/main/engine/engine-process.ts` | Extract the unchanged engine process carrier for reuse with supervised Node child processes. |
| `apps/desktop/src/main/engine/engine-runtime.ts` | Extract the unchanged engine process carrier for reuse with supervised Node child processes. |
| `apps/desktop/src/main/engine/index.ts` | Install the batch IPC and host save packer; persist the shader namespace and winning CLI definitions; flush on shutdown. |
| `apps/desktop/src/main/engine/integration/measure-main.ts` | Remove obsolete routed-result handling from the existing agent batch measurement consumer. |
| `apps/desktop/src/main/engine/profile-definitions.test.ts` | Atomically persist and test the profile’s winning definitions for CLI user extensions and overrides. |
| `apps/desktop/src/main/engine/profile-definitions.ts` | Atomically persist and test the profile’s winning definitions for CLI user extensions and overrides. |
| `apps/desktop/src/main/engine/shader-environment.test.ts` | Share and test the desktop’s persisted per-machine shader probe namespace with hidden hosts and CLI renders. |
| `apps/desktop/src/main/engine/shader-environment.ts` | Share and test the desktop’s persisted per-machine shader probe namespace with hidden hosts and CLI renders. |
| `apps/desktop/src/main/media-store/store.test.ts` | Stage read-only input as a private writable file before fsync; regression protects host imports without changing originals. |
| `apps/desktop/src/main/media-store/store.ts` | Stage read-only input as a private writable file before fsync; regression protects host imports without changing originals. |
| `apps/desktop/src/preload/index.ts` | Minimal bridge-owner hook exposing the existing agent batch IPC to renderer callers. |
| `apps/desktop/src/renderer/src/engine-connection/install.ts` | Allow document-only native transport for hidden CPU snapshots without creating a desktop viewer pool. |
| `apps/desktop/src/renderer/src/engine-connection/render-transport.ts` | Allow document-only native transport for hidden CPU snapshots without creating a desktop viewer pool. |
| `apps/desktop/src/renderer/src/host/hidden-bridge.ts` | Native hidden browser transport, CPU presentation, font bridge and authoritative project-session refresh; required for project-pinned DOM tools. |
| `apps/desktop/src/renderer/src/host/hidden-cpu.ts` | Native hidden browser transport, CPU presentation, font bridge and authoritative project-session refresh; required for project-pinned DOM tools. |
| `apps/desktop/src/renderer/src/host/native-hidden.ts` | Native hidden browser transport, CPU presentation, font bridge and authoritative project-session refresh; required for project-pinned DOM tools. |
| `apps/desktop/src/renderer/src/host/remote-sync.test.ts` | Native hidden browser transport, CPU presentation, font bridge and authoritative project-session refresh; required for project-pinned DOM tools. |
| `apps/desktop/src/renderer/src/host/remote-sync.ts` | Native hidden browser transport, CPU presentation, font bridge and authoritative project-session refresh; required for project-pinned DOM tools. |
| `apps/desktop/src/renderer/src/host/web-bridge.ts` | Native hidden browser transport, CPU presentation, font bridge and authoritative project-session refresh; required for project-pinned DOM tools. |
| `apps/desktop/src/renderer/src/legacy/assistant/agent-frames.test.ts` | Shared renderer-side native batch adapter for capture and observeVisual; tests forbid legacy capture. |
| `apps/desktop/src/renderer/src/legacy/assistant/agent-frames.ts` | Shared renderer-side native batch adapter for capture and observeVisual; tests forbid legacy capture. |
| `apps/desktop/src/renderer/src/legacy/assistant/harness.test.ts` | Regression tests for owned harness frame plans, pending imports and save completion behavior. |
| `apps/desktop/src/renderer/src/legacy/assistant/media-management.test.ts` | Accept host-supplied file bytes through today’s agent importer, with the same validation and durable import behavior. |
| `apps/desktop/src/renderer/src/legacy/assistant/media-management.ts` | Accept host-supplied file bytes through today’s agent importer, with the same validation and durable import behavior. |
| `apps/desktop/src/renderer/src/legacy/core/project-file.ts` | Share the existing linked-original and Undo asset enumeration with the host packer; keep today’s serialization. |
| `apps/desktop/src/renderer/src/legacy/gl/raster.ts` | The original asset-identity extraction was reverted in 1a566f27; the editor importer again owns metadata construction (media owner hot file). |
| `apps/desktop/src/shared/engine-ipc.ts` | Typed agent frame/save and hidden-host integration payloads shared by main, preload, renderer and Node host. |
| `apps/desktop/src/shared/engine-protocol/AudioFacts.ts` | Generated numeric audio-probe facts and protocol build identity used by native imports; generated from pm-protocol. |
| `apps/desktop/src/shared/engine-protocol/MediaFacts.ts` | Generated numeric audio-probe facts and protocol build identity used by native imports; generated from pm-protocol. |
| `apps/desktop/src/shared/engine-protocol/index.ts` | Generated numeric audio-probe facts and protocol build identity used by native imports; generated from pm-protocol. |
| `apps/desktop/src/shared/ipc.ts` | Typed agent frame/save and hidden-host integration payloads shared by main, preload, renderer and Node host. |
| `apps/desktop/src/shared/media-identity.ts` | Temporary extracted metadata helper was removed in 1a566f27 in favor of executing today’s editor importer. |
| `apps/desktop/src/shared/project-assets.ts` | Share today’s packer asset enumeration and linked embedding with host saves, including Undo-only assets. |
| `apps/desktop/src/shared/wire.ts` | Advertise native-hidden capability in the authenticated web handshake so tabs attach CPU/font services. |
| `apps/desktop/tsconfig.node.json` | Include the embedded document-host interface and exclude browser-only contract support from the Node type graph. |
| `engine/crates/pm-media/build.rs` | Expose numeric audio probe facts and cross-build the same native media dependencies for Intel prebuilds. |
| `engine/crates/pm-media/native/build.sh` | Expose numeric audio probe facts and cross-build the same native media dependencies for Intel prebuilds. |
| `engine/crates/pm-media/src/facts.rs` | Expose numeric audio probe facts and cross-build the same native media dependencies for Intel prebuilds. |
| `engine/crates/pm-protocol/src/media.rs` | Add numeric audio facts and codec samples for the host’s import metadata; mirrored generated TypeScript is listed separately. |
| `engine/crates/pm-protocol/src/samples.rs` | Add numeric audio facts and codec samples for the host’s import metadata; mirrored generated TypeScript is listed separately. |
| `engine/crates/pm-render/src/effects/exec.rs` | Configure the shared shader reference/cache from CLI and refuse unverified native user programs when no reference exists. |
| `engine/crates/pm-render/src/effects/mod.rs` | Configure the shared shader reference/cache from CLI and refuse unverified native user programs when no reference exists. |
| `engine/crates/pm-render/src/renderer.rs` | Configure the shared shader reference/cache from CLI and refuse unverified native user programs when no reference exists. |
| `engine/crates/pm-render/src/shader_layers/exec.rs` | Configure the shared shader reference/cache from CLI and refuse unverified native user programs when no reference exists. |
| `engine/third_party/ffmpeg/README.md` | Cross-build and verify x86_64 with the same component selection and LGPL policy as ARM64; record architecture in the manifest. |
| `engine/third_party/ffmpeg/build.sh` | Cross-build and verify x86_64 with the same component selection and LGPL policy as ARM64; record architecture in the manifest. |
| `engine/third_party/ffmpeg/manifest.py` | Cross-build and verify x86_64 with the same component selection and LGPL policy as ARM64; record architecture in the manifest. |
| `engine/third_party/ffmpeg/verify.sh` | Cross-build and verify x86_64 with the same component selection and LGPL policy as ARM64; record architecture in the manifest. |
| `apps/desktop/src/main/media-store/service.test.ts` | Regression for a host save’s frozen MIME when a storage key is overwritten. |
| `apps/desktop/src/main/media-store/service.ts` | Freeze the stored blob MIME beside job content bindings so the host packer matches today’s key reads, including concurrent replacement. |
| `apps/desktop/src/renderer/src/host/hidden-media-store.test.ts` | Verify upload payloads and that a native hidden import waits for host verification. |
| `apps/desktop/src/renderer/src/host/hidden-media-store.ts` | Expose the existing authenticated host media-store protocol to native hidden tabs so import completion means the host has verified the bytes. |
| `apps/desktop/src/renderer/src/media/host-media-store.ts` | Move the existing MIME fallback table unchanged into shared code for the host packer; preserve its public export. |
| `apps/desktop/src/shared/media-mime.ts` | The desktop’s unchanged MIME filename fallback, shared with host project packing. |
| `apps/desktop/src/renderer/src/media/host-media-store.test.ts` | Verify that native hidden tabs install verified host storage while ordinary remote tabs retain their existing store. |
| `apps/desktop/src/main/media-proxy.ts` | The temporary container-identity change in 44e4110f3 was fully reverted by 02150c06d: production must preserve today’s exact metadata and container size. |
| `apps/desktop/src/main/media-proxy.test.ts` | Prove the import contract’s test-only frozen WebM container preserves today’s decoded RGBA; temporary inputs and outputs are removed. |

This inventory itself is the additional `docs/engine/agents-ownership.md` edit requested by Round 4 blocker 7.
