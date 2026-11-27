/** Synthetic evidence only; never installed as a success-shaped host backend. */
import type { NativeFacts, Publication, Row, Snapshot } from "./contract";
export function fixture(): Publication {
  return {
    generation: 1,
    sequence: 1,
    snapshot: {
      projectId: "synthetic-project",
      threadId: "thread",
      runId: null,
      phase: "idle",
      sendable: false,
      composerDisabled: false,
      stopping: false,
      backgroundRuns: 0,
      saveError: null,
      stopError: null,
      elapsedMs: null,
      showJump: false,
      draft: { text: "", selection: [0, 0], armed: null },
      attachments: [],
      settings: {
        provider: "chatgpt",
        model: "GPT-6.1",
        effort: "High",
        access: "editor",
        scope: "workspace",
      },
      catalog: {
        models: [["GPT-6.1", "GPT-6.1"]],
        providers: [["chatgpt", "ChatGPT"]],
        efforts: ["High"],
      },
      slash: { visible: false, index: 0, choices: [] },
      rows: { count: 0, range: [0, 0], items: [] },
    },
    native: {
      connection: "connected",
      reason: "ChatGPT couldn’t be reached.",
      presentation: "popover",
      atMinimumDockWidth: false,
      visible: true,
      extensionContext: false,
      loadingHistory: false,
      dropTarget: false,
      needsAnswer: false,
      keyboardFocus: "composer",
      textSelected: false,
      hoverRow: null,
      increaseContrast: false,
      reduceTransparency: false,
      reduceMotion: false,
      slashCopy: {},
      bindings: { access: "", model: "" },
    },
  };
}
export function messages(): Row[] {
  return [
    {
      id: "history:message-0",
      commandId: "",
      chosen: {},
      value: { role: "user", text: "Make the headline" },
      open: false,
    },
    {
      id: "live:text-1",
      commandId: "text-1",
      chosen: {},
      value: { kind: "text", text: "Done. Headline now scales from 92 % …" },
      open: false,
    },
  ];
}
export function state(s: Partial<Snapshot> = {}, n: Partial<NativeFacts> = {}) {
  const p = fixture();
  Object.assign(p.snapshot, s);
  Object.assign(p.native, n);
  return p;
}
export function withRows(items: Row[], n: Partial<NativeFacts> = {}) {
  return state(
    { rows: { count: items.length, range: [0, items.length], items } },
    n,
  );
}
export const states: Record<string, () => Publication> = {
  empty: fixture,
  "empty-other-access": () => {
    const p = fixture();
    p.snapshot.settings.access = "supervised";
    return p;
  },
  "empty-extension": () => state({}, { extensionContext: true }),
  checking: () => state({}, { connection: "checking" }),
  "sign-in": () => state({}, { connection: "waiting" }),
  unavailable: () => state({}, { connection: "unavailable" }),
  loading: () => withRows(messages(), { loadingHistory: true }),
  working: () => state({ runId: "run", elapsedMs: 62_000 }, {}),
  stopping: () => state({ runId: "run", stopping: true, elapsedMs: 62_000 }),
  "awaiting-answer": () =>
    withRows(
      [
        {
          id: "live:q",
          commandId: "q",
          chosen: {},
          open: false,
          value: {
            id: "q",
            kind: "question",
            status: "open",
            transport: "reply",
            questions: [
              {
                id: "q",
                question:
                  "Apply the same entrance to Kicker? It would start 0.2 s after…",
                options: [{ label: "Yes" }, { label: "No" }],
              },
            ],
          },
        },
      ],
      { needsAnswer: true },
    ),
  hover: () =>
    withRows(
      [
        {
          id: "live:tool",
          commandId: "tool",
          chosen: {},
          open: true,
          value: {
            id: "tool",
            kind: "tool",
            toolName: "apply_commands",
            label: "Make the headline",
            detail: "apply_commands",
            status: "running",
            output: "Done.",
          },
        },
      ],
      { hoverRow: "live:tool" },
    ),
  focus: () => withRows(messages(), { keyboardFocus: "conversation" }),
  selected: () => withRows(messages(), { textSelected: true }),
  disabled: () => state({ composerDisabled: true }),
  error: () => {
    const p = withRows([
      {
        id: "error",
        commandId: "",
        chosen: {},
        open: false,
        value: {
          role: "assistant",
          error: true,
          text: "ChatGPT couldn’t be reached.",
        },
      },
    ]);
    p.snapshot.saveError = "synthetic";
    p.snapshot.stopError = "synthetic";
    return p;
  },
  "drag-drop": () => state({}, { dropTarget: true }),
  "multi-select-na": () => withRows(messages()),
  "60k": () => {
    const p = withRows(messages());
    p.snapshot.rows = {
      count: 60_000,
      range: [30_000, 30_002],
      items: messages(),
    };
    return p;
  },
  narrow: () =>
    withRows(messages(), { presentation: "docked", atMinimumDockWidth: true }),
  docked: () => withRows(messages(), { presentation: "docked" }),
  "increase-contrast": () => state({}, { increaseContrast: true }),
  "reduce-transparency": () => state({}, { reduceTransparency: true }),
  "reduce-motion": () => state({}, { reduceMotion: true }),
};
