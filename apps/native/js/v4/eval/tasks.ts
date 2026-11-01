/** Public, synthetic behavior-only scaffold; no owner panel names, sources, or IDs. */
export type Group = "builtin" | "wild" | "held-out" | "owner";
export interface Task {
  id: string;
  group: Group;
  status: "ready" | "pending-owner";
  brief: string;
  checks: string[];
  behavior?: {
    title: string;
    shows: string;
    inputs: string[];
    interactions: string[];
    reads: string[];
    writes: string[];
    edgeCases: string[];
    apiGaps: string[];
  };
}
const groups: [Group, string, string[]][] = [
  [
    "builtin",
    "inspector",
    [
      "Render a labelled numeric property row and preserve the binding handle.",
      "Render keyed timeline rows and reverse their order without replacing IDs.",
      "Render a scrollable media list with image resource IDs.",
      "Render a curve path and an engine frame handle without raster readback.",
      "Render a layer disclosure and expand its children.",
      "Render controlled caption text and update it from a native observation.",
      "Render settings sections with typed boolean and select values.",
      "Render an export form with numeric, text and progress data.",
      "Render command search results with disabled choices and semantic labels.",
      "Render toolbar items and Glass with semantic surface metadata.",
    ],
  ],
  [
    "wild",
    "surface",
    [
      "Render a grid of metric cards from structured data.",
      "Render a graph using paths and anchored metadata.",
      "Render canvas and frame handles in a scroll container.",
      "Render markdown and code with exact authored whitespace.",
      "Render a virtual list with a bounded visible range.",
      "Render an anchored menu and close it by changing state.",
      "Render a nested focus scope with labelled inputs.",
      "Render a ramp, point and fill control preserving structured values.",
      "Render a webview requiring explicit host opt-in.",
      "Render selector metadata for id/class/role/attr/state.",
    ],
  ],
  [
    "held-out",
    "contract",
    [
      "Move an existing keyed child between sibling positions.",
      "Remove a subtree containing listeners and create a fresh sibling.",
      "Change listener capture/default declarations while keeping the element ID.",
      "Remove props with null while preserving false and empty strings.",
      "Render fragments and conditional empty children without phantom nodes.",
      "Preserve nested hook state through keyed sibling reordering.",
      "Replace a component through Refresh while preserving state.",
      "Preserve context label association through nested function components.",
      "Keep unusual finite JSON values exact through the op protocol.",
      "Reject an invalid whole commit and keep the last accepted tree.",
    ],
  ],
];
const publicTasks: Task[] = groups.flatMap(([group, label, briefs]) =>
  briefs.map((brief, index) => ({
    id: `${group}-${String(index + 1).padStart(2, "0")}`,
    group,
    status: "ready",
    brief,
    checks: ["whole-commit-validity", "semantic-tree", `${label}-${index + 1}`],
  })),
);

/** Owner-approved behavior abstractions. All examples and labels are invented.
 * Reads/writes describe required host services. apiGaps retain the current
 * production-integration gates; executable headless checks never certify a full port.
 */
const ownerSpecs = [
  {
    title: "Curve segment editor",
    shows:
      "A curve graph, four coordinate controls, nested target checkboxes, reference curves, and an edit summary.",
    inputs: [
      "Finite handle coordinates; selected key segments or whole-layer scope; locked and excluded targets.",
    ],
    interactions: [
      "Move a handle by pointer or keyboard; read a target curve; include or omit targets; submit manually or after a change; retain preferences.",
    ],
    reads: [
      "Selected layers and keys, channel values, ancestor protection, document revision, and saved preferences.",
    ],
    writes: [
      "One undoable curve edit for admitted segments; render invalidation; preferences.",
    ],
    edgeCases: [
      "Clamp horizontal coordinates; retain vertical overshoot; ignore nonfinite edits; skip protected targets; preserve exclusions until selection changes; do not edit on cancelled drag.",
    ],
    apiGaps: [
      "Production KernelData project/selection/time publications and atomic edit admission/history/invalidation require application binding.",
      "Production extension-scoped durable settings require state_transaction binding and restart verification.",
      "Native pointer and clipboard delivery require application input adapters; pointer coordinates are publicly typed.",
    ],
  },
  {
    title: "Video cutout with brush guidance",
    shows:
      "A source preview, positive and negative brush controls, progress, three review positions, and a final commit action.",
    inputs: [
      "Selected video snapshot, source revision, normalized brush points and radius, analysis progress and completion.",
    ],
    interactions: [
      "Prepare a source; mark foreground and background; analyze; cancel; review representative frames; commit an editable masked duplicate.",
    ],
    reads: [
      "Source media and timing, project identity and frame rate, protection and existing mask state; decoded frames and local analysis resources.",
    ],
    writes: [
      "A single undoable insertion containing editable mask animation; select the new layer; release decoding and analysis resources.",
    ],
    edgeCases: [
      "Refuse protected or already masked sources; require positive guidance; ignore completion after cancellation; invalidate results on source changes; never publish partial analysis.",
    ],
    apiGaps: [
      "Production KernelData project/selection/time publications and atomic edit admission/history/invalidation require application binding.",
      "No public media decoding, analysis-resource lifecycle or editable mask insertion service.",
      "Native brush coordinate conversion requires macOS input-adapter verification.",
    ],
  },
  {
    title: "Audio take recovery workflow",
    shows:
      "A capture control, elapsed time, input meter, playback option, error status, and recovery actions.",
    inputs: [
      "Permission outcome, audio resource and meter observations, composition identity, playhead and remaining duration, import and edit acknowledgements.",
    ],
    interactions: [
      "Request capture only after a click; stop at the limit; save and insert; retry a failed insertion; recover or discard a retained take.",
    ],
    reads: [
      "Audio device capability, active composition and transport state, current time and duration.",
    ],
    writes: [
      "Permission request, optional transport start/stop, one asset import and bounded audio-layer edit, resource release and optional recovery export.",
    ],
    edgeCases: [
      "Denied permission has no project effect; retain a failed take; cache a successful import across retries; refuse insertion into another composition; do not pause unrelated playback.",
    ],
    apiGaps: [
      "Production KernelData project/selection/time publications and atomic edit admission/history/invalidation require application binding.",
      "No public audio capture, asset import, transport control or recovery-export service.",
    ],
  },
  {
    title: "Paged media search and import",
    shows:
      "Connection state, query and format filters, thumbnail cards, page navigation, preview and per-item transfer status.",
    inputs: [
      "Session-only connection material; query, media kind, orientation, page, remote result metadata and resource handles.",
    ],
    interactions: [
      "Connect; query or browse defaults; change filters; page results; preview a video; transfer an original; choose local files when remote access fails.",
    ],
    reads: [
      "Remote catalog responses, downloadable media resources, preview capabilities and file-picker results.",
    ],
    writes: [
      "Session connection state, asset import request, external navigation request; release preview resources and cancel retired requests.",
    ],
    edgeCases: [
      "Do not persist connection material; ignore old replies; disable unavailable transfers and previous-page navigation at the start; distinguish network failure from empty results; allow failed transfer retry.",
    ],
    apiGaps: [
      "No public catalog/network capability, file picker, asset import or external navigation service.",
    ],
  },
  {
    title: "Verified transparent asset picker",
    shows:
      "A format filter, verified thumbnail grid, load-more control, selected-item metadata, attribution and transfer state.",
    inputs: [
      "Query and format; paged candidates; preview and original alpha validation; attribution replies and import acknowledgement.",
    ],
    interactions: [
      "Search; verify candidates concurrently while retaining server order; append unique results; choose a card; inspect attribution; recheck its original before importing.",
    ],
    reads: [
      "Remote image metadata, preview pixels, original asset pixels, source and license information.",
    ],
    writes: [
      "Asset import and source navigation requests; cancellation and preview-handle disposal.",
    ],
    edgeCases: [
      "Reject fully opaque originals; skip broken previews; deduplicate pages; late attribution must not replace the current selection; reject duplicate successful imports.",
    ],
    apiGaps: [
      "No public image alpha inspection, catalog, attribution, asset import or external navigation service.",
    ],
  },
  {
    title: "Read-only frame comparison",
    shows:
      "Comparison progress, numeric difference summaries, live and difference images, and side-by-side regions.",
    inputs: [
      "Synthetic reference pixels, requested sample times and dimensions, crop geometry, rendered-frame handles and document revisions.",
    ],
    interactions: [
      "Validate geometry; render samples; compare RGB independently of alpha; publish metrics and image handles; rerun after failure.",
    ],
    reads: [
      "Project identity, dimensions and revision, evaluated bounds and transforms, native frame pixels and reference image resources.",
    ],
    writes: [
      "Temporary preview resources and comparison status only; never project edits.",
    ],
    edgeCases: [
      "Reject unexpected geometry or frame dimensions; mark changed-revision results invalid; release resources on failure or retirement; restore the run action.",
    ],
    apiGaps: [
      "No public evaluated-geometry query, frame rendering/readback or preview-resource lifecycle service.",
    ],
  },
  {
    title: "Position-preserving pivot grid",
    shows:
      "Nine pivot choices, editable-selection count, current-position state and a preference for newly created artwork.",
    inputs: [
      "Bounds, evaluated pivot and position, local linear transform, time, selection and creation notifications.",
    ],
    interactions: [
      "Pick a normalized location; compensate position by the transformed pivot delta; submit grouped edits; optionally center newly created eligible artwork.",
    ],
    reads: [
      "Layer types, ancestor locks, expressions and keys, evaluated bounds/transforms, creation history and saved preference.",
    ],
    writes: [
      "Pivot and compensating position edits at the correct time; durable preference.",
    ],
    edgeCases: [
      "Skip unsupported, protected or expression-driven artwork; no-op already aligned pivots; automatic edits exclude animated artwork and project-load or undo notifications.",
    ],
    apiGaps: [
      "Production KernelData project/selection/time publications and atomic edit admission/history/invalidation require application binding.",
      "Production extension-scoped durable settings require state_transaction binding and restart verification.",
      "Production evaluated bounds/transforms require a QueryMirror-backed KernelData adapter.",
    ],
  },
  {
    title: "Selective effect batch action",
    shows:
      "A keyed layer list with per-row color, selected and protected state, effect badges and one context-sensitive batch action.",
    inputs: [
      "Visual layer snapshots, selection, modifier keys, attached effect instances and acknowledgements.",
    ],
    interactions: [
      "Replace or extend selection; add an effect only to missing targets; remove it when every admitted target already has it; retry removal by effect type after an instance miss.",
    ],
    reads: [
      "Current layer list, layer protection, selection and effect membership.",
    ],
    writes: [
      "Selection requests and acknowledged effect add/remove commands; status messages.",
    ],
    edgeCases: [
      "Exclude unsupported and protected layers; suppress duplicate submissions while busy; mixed selection must not duplicate effects; recover from negative acknowledgements.",
    ],
    apiGaps: [
      "Production KernelData project/selection/time publications and atomic edit admission/history/invalidation require application binding.",
    ],
  },
  {
    title: "Keyboard arcade with saved score",
    shows:
      "A bounded play area, moving obstacles, current and personal-best score, pause state and restart action.",
    inputs: [
      "Keyboard or pointer actions, deterministic frame ticks, collision and visibility observations, saved score and persistence outcome.",
    ],
    interactions: [
      "Start or impulse; pause/resume; restart; advance only while visible and active; save a higher score after a collision.",
    ],
    reads: [
      "Saved nonnegative finite score and panel visibility/focus lifecycle.",
    ],
    writes: [
      "Best-score preference only; frame scheduler registration and cancellation; no document edits.",
    ],
    edgeCases: [
      "Discard invalid saved values; cap long frame deltas; ignore key repeats; pause on focus loss; storage failure retains session score; retirement cancels ticks.",
    ],
    apiGaps: [
      "Production extension-scoped durable settings require state_transaction binding and restart verification.",
      "No public panel visibility/focus lifecycle or native frame scheduler.",
    ],
  },
  {
    title: "Playhead annotation form",
    shows:
      "A labelled text input, submit action and status reflecting an annotation at the current time.",
    inputs: [
      "Current playhead time, exact-time annotations, edited text, and command acknowledgement.",
    ],
    interactions: [
      "Republish text on time or document changes; submit by click or Enter; update an existing annotation at the exact time or create one.",
    ],
    reads: [
      "Playhead subscription, current document annotations and revision.",
    ],
    writes: [
      "An acknowledged undoable annotation upsert; detach subscriptions on retirement.",
    ],
    edgeCases: [
      "Reject whitespace-only input; preserve input on rejection; retain existing annotation identity; empty positions show an empty field; retired subscriptions cannot update the view.",
    ],
    apiGaps: [
      "Production KernelData project/selection/time publications and atomic edit admission/history/invalidation require application binding.",
    ],
  },
];

const ownerChecks: string[][] = [
  [
    "Four controls and graph agree after a keyboard edit.",
    "Protected and omitted targets never appear in the edit request.",
    "No targets produces no request; selection change resets omissions.",
  ],
  [
    "Analysis is disabled without positive guidance.",
    "Cancelled completion cannot enable commit or insert a layer.",
    "A changed source rejects a finished result; a fresh result sends one complete insertion.",
  ],
  [
    "Mounting never requests audio access.",
    "A rejected edit retains recovery and retry reuses the imported resource.",
    "A different composition gets no insertion; successful retry clears the take; retirement releases capture.",
  ],
  [
    "A later search wins over an older completion.",
    "First-page navigation and missing-download cards are disabled.",
    "Failed transfer can retry once; retirement ignores replies and releases previews.",
  ],
  [
    "Only transparent candidates remain in original order without duplicate cards.",
    "Stale attribution is ignored.",
    "Opaque original creates no import; accepted original imports once.",
  ],
  [
    "Alpha-only differences count as identical RGB; one changed channel yields exact count and maximum.",
    "Geometry mismatch requests no frame.",
    "Revision changes invalidate results; no path writes document data.",
  ],
  [
    "Compensation preserves transformed screen position with scale and rotation.",
    "Protected or expression-driven targets create no edits.",
    "Automatic mode edits only a new unanimated target after an actual creation.",
  ],
  [
    "Rows preserve identity and color when reordered.",
    "Mixed selection adds only the missing unlocked target.",
    "Removal fallback uses one instance miss then one type request; failure clears busy state.",
  ],
  [
    "Declared keys prevent native defaults and toggle pause.",
    "Paused or hidden frames do not advance; a long tick is bounded.",
    "Collision stores a monotonic score; storage failure preserves the session best.",
  ],
  [
    "Empty input sends no command.",
    "Enter updates the annotation identity at the current time; click at another time creates one.",
    "Negative acknowledgement preserves text and reports failure; retirement detaches time updates.",
  ],
];

export const tasks: Task[] = [
  ...publicTasks,
  ...ownerSpecs.map(
    (behavior, index): Task => ({
      id: `owner-${String(index + 1).padStart(2, "0")}`,
      group: "owner",
      status: "ready",
      brief: behavior.title + ": " + behavior.shows,
      behavior,
      checks: ownerChecks[index],
    }),
  ),
];
