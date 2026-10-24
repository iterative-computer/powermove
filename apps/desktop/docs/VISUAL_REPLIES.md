# Agent visual replies

`publish_visual_reply({title, html, height?})` publishes an interactive HTML answer in the calling agent run's thread. The contract is shared by native Codex, Claude and compatible-provider tool sessions. It is offered in app/project and inspection runs, and changes no composition state. It uses the existing authenticated native tool bridge and renderer request/response validation. No new IPC, external service, model account or tool-approval bypass is introduced.

Pages published by the same active response share one compact attachment group without replacing assistant prose. The renderer harness dispatches to `SpatialAssistant.publishVisualReply`; `publish-visual-reply.ts` resolves the active run to its project/thread archive rather than the visible conversation. A successful tool result means both the immutable artifact and its assistant-message association have been saved through `AgentThreads`. A failed storage write removes the provisional message and reports an error. Up to four pages per run, 384 KiB UTF-8 each, are accepted. Thread deletion removes the inline artifact bytes with its conversation. Reopening reconstructs the page from the saved source; replay resets its interactions. Interaction state itself is deliberately not saved. Follow-up context includes the page title and artifact ID, not its full HTML.

`Turn.svelte` keeps the assistant's complete Markdown and renders associated visuals in an attachment rail. Each live preview is at most 280 CSS pixels wide and 180 pixels tall; multiple previews scroll horizontally without making the response taller. The inline attachment offers Expand. Zoom (50–200%), replay and close controls belong to the deliberately opened dialog. Collapsing or pressing Escape keeps the same iframe and restores focus to its Expand button; Escape does not cancel the agent run or undo its result. The page accepts bounded content-size reports, while the inline preview remains compact. Theme changes preserve the live document and its interaction state, and adding another attachment preserves earlier frame identities.

Host chrome uses Powermove's existing attachment classes (`agent-attachment`, file-type badge and attachment-name/open behavior), the shared Phosphor `Icon` component, and native `iconbtn` / ghost `btn` controls. The expanded viewer uses the application's modal surface, radius, shadow, title typography, spacing, backdrop and entrance motion. Its shadow is resolved from root theme tokens so the Agent panel's local popover override does not change a window-level modal. Loading uses the existing shimmer treatment; errors use the shared `ErrorNotice` and native Replay action. Buttons retain accessible names, tooltips, focus indicators and disabled states. Compact previews are exposed as named groups, and only deliberate expansion exposes a named dialog. Escape remains effective after native pointer-focus handling clears toolbar focus. Reduced motion uses the application's existing motion override. These are host-only changes; generated artwork and the sandbox document are not restyled.

The presentation was checked against current T3 Code source at [`6a6ebb278dc70dd859439a3c28960a7d23a1e645`](https://github.com/pingdotgg/t3code/tree/6a6ebb278dc70dd859439a3c28960a7d23a1e645). Its [MessagesTimeline attachment row](https://github.com/pingdotgg/t3code/blob/6a6ebb278dc70dd859439a3c28960a7d23a1e645/apps/web/src/components/chat/MessagesTimeline.tsx), [inline HtmlRenderFrame](https://github.com/pingdotgg/t3code/blob/6a6ebb278dc70dd859439a3c28960a7d23a1e645/apps/web/src/components/chat/HtmlRenderFrame.tsx), and [HtmlRenderDialog](https://github.com/pingdotgg/t3code/blob/6a6ebb278dc70dd859439a3c28960a7d23a1e645/apps/web/src/components/chat/HtmlRenderDialog.tsx) separate the visual from prose, offer an inline expansion action, and put zoom in fullscreen. Powermove follows that separation and control hierarchy, with a bounded attachment preview for its narrow chat. T3 itself fits the inline page to content; the 180-pixel cap and horizontal grouping implement the user's explicit compactness requirement, not a claimed T3 pixel-for-pixel layout. No T3 code or assets were copied.


## Security boundary

Only Electron renders scripts. Browser hosts display a desktop-app requirement and cannot publish a visual reply. This restriction is intentional: the browser host has no equivalent to Electron's main-process frame-navigation guard.

The static `/visual-reply.html` bootstrap receives exactly one bounded display document from its actual parent, checking the parent's origin and message shape. The iframe grants only `allow-scripts`; it never grants `allow-same-origin`, forms, top navigation, popups or downloads. Electron keeps context isolation and its OS sandbox, disables Node in the window and explicitly in subframes, and does not inject a subframe preload. The static document response and generated document carry restrictive CSPs: no network connections, external scripts, frames, workers, objects, media, fonts, base URLs or form submissions. Inline JavaScript and CSS and embedded raster images are allowed. A dedicated document is necessary because `srcdoc` would inherit the host's restrictive script policy.

The page cannot call tools, send chat messages, open links, write files or access a host bridge. Its only outbound protocol is `ready`, bounded `size`, and bounded display `error`. The host checks the currently mounted Window source, opaque `null` origin, per-mount random token, closed message schema, and numeric/string bounds. Stale/spoofed messages are discarded. Theme messages are checked against the actual parent and token and accept only four bounded CSS colors and a boolean-like appearance choice.

Main's `will-frame-navigate` handler blocks navigation originating from a committed visual-reply frame, including navigation back into the app. It identifies the committed frame/initiator URL, rather than `window.name`, which page scripts can change. Existing navigation and approval rules remain in place. Host errors expose generic messages rather than arbitrary generated console output.

No filesystem path is accepted by the publishing API. Local image bytes must already have been acquired through the agent's existing authorized tools and supplied as raster data URIs. Validation parses into inert template contents so even rejected image/iframe markup cannot load resources in the host. Nested templates are rejected. Static image references reject file/remote paths, SVG data URIs, mismatched raster signatures and encoded images above 256 KiB (at most about 192 KiB decoded). The whole page remains capped at 384 KiB. CSS/dynamically created resource requests are blocked by CSP. The publishing tool therefore cannot read a privileged local file on the agent's behalf.

This is a desktop feature, not a general MCP Apps host. It provides no bidirectional tool bridge. Resource-heavy scripts can still consume CPU; no separate execution watchdog has been added. Native CSP, origin isolation, navigation, live theme delivery and iframe lifecycle are exercised by the Electron walkthrough.

## Examples and validation

`e2e/fixtures/visual-replies/` contains complete tool inputs for a timing/easing chart with a duration slider and animation, a three-shot storyboard with a next-shot control, and an inspectable palette. These are real self-contained pages, not a replacement chat UI.

Unit/regression coverage includes input and byte limits, embedded-image validation, host-message origin/source/token/schema checks, run/thread targeting, inactive runs, output limits, storage-failure rollback, archive reopening/deletion, follow-up context, native chat controls, browser fail-closed behavior, and existing tool approval/context restrictions. happy-dom disables actual iframe loading; those tests validate host behavior and do not prove Chromium sandbox execution.

`e2e/agent-visual-replies.spec.ts` covers publication, interaction, same-frame expansion, zoom, replay, live light/dark themes, content resize, compact layout, malformed inputs, script errors and relaunch persistence. Hostile pages probe Node/preload/parent access, forged tool messages, renamed-frame navigation, popup denial and network containment using a local HTTP observer. Chromium can replace an attempted forbidden navigation with its blocked-navigation page; Replay restores the visualization without contacting the target.

Mixed prose with one/multiple attachments, long replies, narrow rails, expansion/collapse, focus return and replay are covered alongside persistence and security. The deterministic provider fixture also enters the actual native run lifecycle and publishes all three examples through the run-scoped authenticated MCP session. It does not override renderer publication or insert messages manually, and requires no paid model call. A regression covers replacing visual artifacts when switching assistant rows while preserving unchanged iframe inputs.

Run from `apps/desktop` on a sandbox-capable isolated Mac host:

```sh
POWERMOVE_VISUAL_EVIDENCE_DIR=/tmp/powermove-visual-evidence \
  npm run test:e2e -- e2e/agent-visual-replies.spec.ts --trace on
```

Use a disposable profile and normal Chromium sandboxing. The native suite verifies sandbox/context isolation preferences and disabled Node integration in the window and subframes. Review the light/dark captures against existing Powermove panels. The provider fixture validates integration, not the quality of a live model's generated HTML.

## Upstream research and attribution

Research used the official [pingdotgg/t3code repository](https://github.com/pingdotgg/t3code), cloned at [`3a4d118a518a025e44f11248fc4d2c87eb86aa00`](https://github.com/pingdotgg/t3code/tree/3a4d118a518a025e44f11248fc4d2c87eb86aa00) on 2026-10-18. The supplied X post was not viewed. A GitHub API tree request returned HTTP 403; the independent official Git clone that was already running completed successfully, so no source details were inferred from that failed request.

The [user docs](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/docs/user/html-renders.md) describe HTML visual replies separately from MCP Apps. Implementation evidence:

| Responsibility | Exact source |
| --- | --- |
| `html_preview` and `html_render` schemas and agent instructions | [mcp/toolkits/html/tools.ts](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/apps/server/src/mcp/toolkits/html/tools.ts) |
| Caller/thread-scoped publication and returned `htmlRender` reference | [mcp/toolkits/html/handlers.ts](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/apps/server/src/mcp/toolkits/html/handlers.ts) |
| Local-image embedding, image-byte checks, attachment writes, measurement and preview | [htmlRender/HtmlRender.ts](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/apps/server/src/htmlRender/HtmlRender.ts) |
| Shared references, size calculation, theme and page-host messages | [shared/htmlRender.ts](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/packages/shared/src/htmlRender.ts) |
| Attachment URLs, in-thread sizing and expansion | [chat/HtmlRenderFrame.tsx](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/apps/web/src/components/chat/HtmlRenderFrame.tsx) |
| Sandboxed frame, theme posting and validated page interactions | [files/BrowserDocumentFrame.tsx](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/apps/web/src/components/files/BrowserDocumentFrame.tsx) |
| Server HTML response CSP and sandbox | [http.ts](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/apps/server/src/http.ts) |
| Thread-associated HTML attachment cleanup | [attachmentStore.ts](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/apps/server/src/attachmentStore.ts) |

Relevant history:

- [677d1527c3d96b90b2294d9a0aed7d8c8874b29b](https://github.com/pingdotgg/t3code/commit/677d1527c3d96b90b2294d9a0aed7d8c8874b29b), 2026-10-05: agents can show HTML pages inline in threads (#15968).
- [4d976d1fc233d3aa971fa7dcbf2e7316bfa3ac72](https://github.com/pingdotgg/t3code/commit/4d976d1fc233d3aa971fa7dcbf2e7316bfa3ac72), 2026-10-07: separate MCP Apps hosting (#16236).
- [8c192c75564f7abb3b060950f810dc4d4c97385b](https://github.com/pingdotgg/t3code/commit/8c192c75564f7abb3b060950f810dc4d4c97385b), 2026-10-08: visual pages can send an answer to the thread. Powermove intentionally does not expose this capability.
- [aba3a6ddc0d2625739cb65c333f6052a4a35bc71](https://github.com/pingdotgg/t3code/commit/aba3a6ddc0d2625739cb65c333f6052a4a35bc71), 2026-10-14: fullscreen with zoom (#17561).

T3 Code's [MIT license](https://github.com/pingdotgg/t3code/blob/3a4d118a518a025e44f11248fc4d2c87eb86aa00/LICENSE) names Copyright (c) 2026 T3 Tools Inc. Copies or substantial portions of its software must retain that copyright and the MIT permission notice; retaining the full license is the straightforward distribution practice. This Powermove implementation was independently authored after inspecting the architecture; no T3 source code or UI assets were copied. Powermove retains its existing AGPL-3.0-or-later licensing. The source links above provide research provenance without adding attribution captions to the product UI.
