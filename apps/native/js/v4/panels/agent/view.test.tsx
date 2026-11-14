import { test, expect } from "bun:test";
import type { ComponentChild } from "preact";
import { createRoot, type Commit } from "../../src";
import {
  AgentPanel,
  AgentView,
  composerKey,
  elapsed,
  type Menu,
  type MenuOption,
} from "./view";
import { fixture, state, states, withRows } from "./fixtures";
import type { Action, AgentModel, Publication, Row } from "./contract";

function mount() {
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  return { root, commits };
}
function replay(commits: Commit[]) {
  const result = Bun.spawnSync([process.env.PM_UI_KIT_WIRE!], {
    stdin: new TextEncoder().encode(
      commits.map((c) => JSON.stringify(c)).join("\n") + "\n",
    ),
  });
  expect(result.exitCode).toBe(0);
  return result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
}
function assertWire(commits: Commit[]) {
  const replies = replay(commits);
  for (const row of replies) {
    expect(row.accepted).toBe(true);
    expect(row.diagnostics).toEqual([]);
    expect(row.wireDiagnostics).toEqual([]);
    expect(row.accessibilityNodes).toBe(Object.keys(row.nodes).length);
  }
  return replies.at(-1);
}
for (const [name, make] of Object.entries(states)) {
  test(`spec §4 ${name}: op-tree snapshot and Rust wire/accessibility validator`, async () => {
    const { root, commits } = mount();
    await root.render(<AgentView publication={make()} send={() => {}} />);
    const result = assertWire(commits);
    expect({
      ops: commits.flatMap((c) => c.ops).map((op) => op.op),
      nodes: result.nodes,
      focusOrder: result.focusOrder,
    }).toMatchSnapshot();
    // Gaps are intentional diagnostics in metadata, not unknown native props.
    expect(
      root
        .inspect()
        .nodes.some(
          (n) => n.props.metadata && typeof n.props.metadata === "object",
        ),
    ).toBe(true);
    root.dispose();
  });
}

const node = (
  root: ReturnType<typeof mount>["root"],
  pick: (n: any) => boolean,
) => root.inspect().nodes.find(pick)!;
const byId = (root: ReturnType<typeof mount>["root"], id: string) =>
  node(root, (n) => n.props.id === id);
const byLabel = (root: ReturnType<typeof mount>["root"], label: string) =>
  node(root, (n) => n.props.label === label);
const has = (root: ReturnType<typeof mount>["root"], label: string) =>
  root.inspect().nodes.some((n) => n.props.label === label);

test("focus order follows the thread bar, conversation and composer as the original panel's", async () => {
  const { root, commits } = mount();
  const p = states.selected();
  p.snapshot.sendable = true;
  p.snapshot.catalog.efforts = ["", "high"];
  await root.render(
    <AgentView publication={p} menu={async () => null} send={() => {}} />,
  );
  const result = assertWire(commits);
  const ids = result.focusOrder
    .map((id: number) => result.nodes[id].props.id)
    .filter(Boolean);
  expect(ids).toEqual([
    "agent-threads",
    "agent-new",
    "agent-presentation",
    "agent-conversation",
    "agent-composer-text",
    "agent-attach",
    "agent-send",
    "agent-model",
    "agent-effort",
    "agent-access",
  ]);
  expect(byId(root, "agent-conversation").props.label).toBe(
    "Agent conversation",
  );
  root.dispose();
});
test("new thread, send and stop use the model's commands; Send hides while working on an empty draft", async () => {
  const { root, commits } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.sendable = true;
  const render = () =>
    root.render(<AgentView publication={p} send={(a) => actions.push(a)} />);
  await render();
  await root.drive(byId(root, "agent-new").id, "click");
  await root.drive(byId(root, "agent-send").id, "click");
  p.snapshot.runId = "run";
  p.snapshot.sendable = false;
  await render();
  expect(root.inspect().nodes.some((n) => n.props.id === "agent-send")).toBe(
    false,
  );
  await root.drive(byId(root, "agent-stop").id, "click");
  p.snapshot.stopping = true;
  await render();
  await root.drive(byId(root, "agent-stop").id, "click");
  expect(actions).toEqual([
    { type: "newThread" },
    { type: "submit" },
    { type: "stop" },
  ]);
  assertWire(commits);
  root.dispose();
});
test("Escape in the composer reaches the model, which closes the popover (the close bug)", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  await root.render(
    <AgentView publication={fixture()} send={(a) => actions.push(a)} />,
  );
  await root.drive(byId(root, "agent-composer-text").id, "keydown", {
    key: "Escape",
  });
  expect(actions).toEqual([{ type: "key", key: { key: "Escape" } }]);
  root.dispose();
});
test("an empty thread offers the original's suggestions; one fills the composer", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  await root.render(
    <AgentView publication={fixture()} send={(a) => actions.push(a)} />,
  );
  expect(has(root, "Make your next move")).toBe(true);
  for (const label of [
    "Animate a title",
    "Shape the scene",
    "Arrange my workspace",
  ])
    expect(has(root, label)).toBe(true);
  await root.drive(byLabel(root, "Animate a title").id, "click");
  const prompt =
    "Animate the selected title with a confident entrance using editable keyframes";
  expect(actions).toEqual([
    {
      type: "draft",
      text: prompt,
      selection: [prompt.length, prompt.length],
      base: "",
    },
  ]);
  root.dispose();
});
/** The window's field menu, answering with `answer` and keeping requests. */
function fieldMenu(answer: (options: MenuOption[]) => unknown) {
  const asked: { placeholder: string; options: MenuOption[] }[] = [];
  const menu: Menu = async (request) => {
    asked.push(request);
    return answer(request.options);
  };
  return { menu, asked };
}
test("the thread picker lists threads with their state, switches, and offers New and Delete", async () => {
  const { root, commits } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.threadTitle = "Ask which title to add";
  p.snapshot.threads = [
    {
      id: "thread",
      title: "Ask which title to add",
      busy: false,
      updatedAt: 0,
    },
    { id: "older", title: "Make the headline", busy: true, updatedAt: 0 },
  ];
  let pick = "thread:older";
  const { menu, asked } = fieldMenu(() => pick);
  await root.render(
    <AgentView publication={p} menu={menu} send={(a) => actions.push(a)} />,
  );
  const picker = byId(root, "agent-threads");
  expect(
    picker.text +
      root
        .inspect()
        .nodes.filter((n) => n.text)
        .map((n) => n.text)
        .join("|"),
  ).toContain("Ask which title to add");
  await root.drive(picker.id, "click");
  await Promise.resolve();
  expect(asked[0]!.options.map((o) => o.label)).toEqual([
    "New Thread",
    "Ask which title to add",
    "Make the headline · Working…",
    "Delete Thread…",
  ]);
  pick = "delete:thread";
  await root.drive(picker.id, "click");
  await Promise.resolve();
  expect(actions).toEqual([
    { type: "switchThread", id: "older" },
    { type: "deleteThread", id: "thread" },
  ]);
  assertWire(commits);
  root.dispose();
});
test("provider and model share one menu; effort and access have their own; picks reach the model", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.catalog = {
    models: [
      ["gpt-6", "GPT-6"],
      ["gpt-6-mini", "GPT-6 mini"],
    ],
    providers: [
      ["chatgpt", "ChatGPT"],
      ["claude", "Claude"],
    ],
    efforts: ["low", "high"],
  };
  p.snapshot.settings.model = "gpt-6";
  p.snapshot.settings.effort = "high";
  const picks = ["provider:claude", "model:gpt-6-mini", "low", "auto"];
  const asked: MenuOption[][] = [];
  const menu: Menu = async (r) => {
    asked.push(r.options);
    return picks.shift();
  };
  await root.render(
    <AgentView publication={p} menu={menu} send={(a) => actions.push(a)} />,
  );
  expect(byId(root, "agent-model").props.label).toBe("Model, GPT-6");
  expect(byId(root, "agent-effort").props.label).toBe("Reasoning effort, High");
  for (const id of [
    "agent-model",
    "agent-model",
    "agent-effort",
    "agent-access",
  ]) {
    await root.drive(byId(root, id).id, "click");
    await Promise.resolve();
  }
  // One place for each: the model menu has no effort rows.
  expect([...new Set(asked[0]!.map((o) => o.category))]).toEqual([
    "Provider",
    "Model",
  ]);
  expect(asked[2]!.map((o) => [o.label, o.checked])).toEqual([
    ["Low", false],
    ["High", true],
  ]);
  expect(actions).toEqual([
    { type: "setProvider", provider: "claude" },
    { type: "setModel", model: "gpt-6-mini", effort: "high" },
    { type: "setModel", model: "gpt-6", effort: "low" },
    { type: "setAccess", access: "auto" },
  ]);
  root.dispose();
});
test("the effort picker names the model's effort, lists only its levels and hides for a model with none", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.catalog = {
    models: [["gpt-6.1-sol", "GPT-6.1-Sol"]],
    providers: [["chatgpt", "ChatGPT"]],
    // The model publishes only the efforts the selected model takes.
    efforts: ["low", "medium", "xhigh"],
  };
  p.snapshot.settings.model = "gpt-6.1-sol";
  // With none saved, the model publishes the model's default by name.
  p.snapshot.settings.effort = "medium";
  const asked: MenuOption[][] = [];
  const menu: Menu = async (r) => {
    asked.push(r.options);
    return null;
  };
  await root.render(<AgentView publication={p} menu={menu} send={() => {}} />);
  expect(byId(root, "agent-effort").props.label).toBe(
    "Reasoning effort, Medium",
  );
  await root.drive(byId(root, "agent-effort").id, "click");
  await Promise.resolve();
  expect(asked[0]!.map((o) => [o.label, o.checked])).toEqual([
    ["Low", false],
    ["Medium", true],
    ["Extra high", false],
  ]);
  expect(asked[0]!.some((o) => /Default/.test(o.label))).toBe(false);
  // A model with no effort levels: no picker at all, not a disabled one.
  p.snapshot.catalog.efforts = [];
  p.snapshot.settings.effort = "";
  await root.render(<AgentView publication={p} menu={menu} send={() => {}} />);
  expect(has(root, "Reasoning effort, ")).toBe(false);
  expect(root.inspect().nodes.some((n) => n.props.id === "agent-effort")).toBe(
    false,
  );
  root.dispose();
});
test("the model picker lists only real models and names the one that runs", async () => {
  const { root } = mount();
  const p = fixture();
  // The model resolves a thread with no choice to the provider's default
  // model; the catalog has no "Default" row.
  p.snapshot.catalog = {
    models: [
      ["gpt-6.1-sol", "GPT-6.1-Sol"],
      ["gpt-6-astra", "GPT-6-Astra"],
    ],
    providers: [["chatgpt", "ChatGPT"]],
    efforts: ["", "high"],
  };
  p.snapshot.settings.model = "gpt-6.1-sol";
  p.snapshot.settings.effort = "";
  const asked: MenuOption[][] = [];
  const menu: Menu = async (r) => {
    asked.push(r.options);
    return null;
  };
  await root.render(<AgentView publication={p} menu={menu} send={() => {}} />);
  expect(byId(root, "agent-model").props.label).toBe("Model, GPT-6.1-Sol");
  await root.drive(byId(root, "agent-model").id, "click");
  await Promise.resolve();
  const models = asked[0]!.filter((o) => o.category === "Model");
  expect(models.map((o) => [o.label, o.checked])).toEqual([
    ["GPT-6.1-Sol", true],
    ["GPT-6-Astra", false],
  ]);
  // Before a provider lists its models there is no model to name: the
  // picker names the provider, and no blank row is added.
  p.snapshot.catalog.models = [];
  p.snapshot.settings.model = "";
  await root.render(<AgentView publication={p} menu={menu} send={() => {}} />);
  expect(byId(root, "agent-model").props.label).toBe("Model, ChatGPT");
  await root.drive(byId(root, "agent-model").id, "click");
  await Promise.resolve();
  expect(asked[1]!.filter((o) => o.category === "Model")).toEqual([]);
  expect(asked.flat().some((o) => /^Default/.test(o.label))).toBe(false);
  root.dispose();
});
const turn = (): Row[] => [
  {
    id: "history:u",
    commandId: "history:u",
    chosen: {},
    open: false,
    value: { role: "user", text: "Check the project, then ask me a question." },
  },
  {
    id: "history:t/step-0:a",
    commandId: "history:t/step-0:a",
    chosen: {},
    open: false,
    trace: { id: "t", durationMs: 20_000, steering: false },
    value: {
      id: "a",
      kind: "text",
      text: "I’m checking the live composition and its layers.",
    },
  },
  {
    id: "history:t/step-1:call",
    commandId: "history:t/step-1:call",
    chosen: {},
    open: false,
    trace: { id: "t", durationMs: 20_000, steering: false },
    value: {
      id: "call",
      kind: "tool",
      toolName: "get_project_state",
      label: "Get project state",
      detail: "get_project_state",
      status: "done",
      startedAt: 1000,
      endedAt: 1400,
    },
  },
  {
    id: "history:t/step-2:b",
    commandId: "history:t/step-2:b",
    chosen: {},
    open: false,
    trace: { id: "t", durationMs: 20_000, steering: false },
    value: {
      id: "b",
      kind: "text",
      text: "The project is **blank**: 10 seconds, `1920 × 1080`.",
    },
  },
];
test("a finished turn folds its work behind “Worked for”, and its tool group discloses each call", async () => {
  const { root, commits } = mount();
  await root.render(
    <AgentView publication={withRows(turn())} send={() => {}} />,
  );
  expect(
    has(root, "You said: Check the project, then ask me a question."),
  ).toBe(true);
  // The answer stays in view; the work before it folds.
  expect(
    has(root, "Agent: The project is blank: 10 seconds, 1920 × 1080."),
  ).toBe(true);
  expect(has(root, "1 tool call, <1s")).toBe(false);
  await root.drive(byLabel(root, "Worked for 20s").id, "click");
  expect(has(root, "1 tool call, <1s")).toBe(true);
  expect(has(root, "Get project state")).toBe(false);
  await root.drive(byLabel(root, "1 tool call, <1s").id, "click");
  // The chip names what it worked on, never only the tool's own id.
  expect(has(root, "Get project state")).toBe(true);
  // Inline Markdown becomes styled runs of one native text.
  const runs = root
    .inspect()
    .nodes.find((n) => (n.props.metadata as any)?.runs)!;
  expect((runs.props.metadata as any).runs).toEqual([
    { t: "The project is " },
    { t: "blank", b: true },
    { t: ": 10 seconds, " },
    { t: "1920 × 1080", c: true },
    { t: "." },
  ]);
  assertWire(commits);
  root.dispose();
});
test("a running turn shows its timer under the request and keeps its live work open", async () => {
  const { root, commits } = mount();
  const rows: Row[] = [
    turn()[0]!,
    {
      id: "live:call",
      commandId: "call",
      chosen: {},
      open: false,
      value: {
        id: "call",
        kind: "tool",
        toolName: "get_project_state",
        label: "Get project state",
        status: "running",
      },
    },
  ];
  const p = withRows(rows);
  p.snapshot.runId = "run";
  p.snapshot.elapsedMs = 5_400;
  await root.render(<AgentView publication={p} send={() => {}} />);
  expect(has(root, "Working for 5s")).toBe(true);
  expect(has(root, "1 tool call")).toBe(true);
  expect(has(root, "Get project state")).toBe(true);
  assertWire(commits);
  root.dispose();
});
test("a failed turn reads as words, keeps its raw details behind Details, and Try Again resends it", async () => {
  const { root, commits } = mount();
  const actions: Action[] = [];
  const raw =
    '{"type":"error","error":{"type":"invalid_request_error","message":"The model gpt-x does not exist."}}';
  const rows: Row[] = [
    turn()[0]!,
    {
      id: "history:e",
      commandId: "history:e",
      chosen: {},
      open: false,
      value: { role: "assistant", text: raw, error: true },
    },
  ];
  await root.render(
    <AgentView publication={withRows(rows)} send={(a) => actions.push(a)} />,
  );
  expect(
    has(root, "Something needs attention. The model gpt-x does not exist."),
  ).toBe(true);
  // The raw JSON is never the message.
  expect(root.inspect().nodes.some((n) => n.text.includes('{"type"'))).toBe(
    false,
  );
  await root.drive(byLabel(root, "Details").id, "click");
  expect(
    root.inspect().nodes.some((n) => n.text.replaceAll("\u200b", "") === raw),
  ).toBe(true);
  await root.drive(byId(root, "error-history:e-retry").id, "click");
  expect(actions).toEqual([{ type: "retry", id: "history:e" }]);
  assertWire(commits);
  root.dispose();
});
test("question cards choose, answer and skip through the model; Answer waits for a choice", async () => {
  const { root } = mount();
  const sent: Action[] = [];
  const question: Row = {
    id: "live:q",
    commandId: "q",
    chosen: {},
    open: true,
    value: {
      id: "q",
      kind: "question",
      status: "open",
      transport: "reply",
      questions: [
        {
          id: "a",
          question: "Which title?",
          options: [{ label: "Make Your Move" }],
        },
      ],
    },
  };
  const send = (action: Action) => sent.push(action);
  await root.render(
    <AgentView publication={withRows([question])} send={send} />,
  );
  expect(byId(root, "live:q-answer").props.disabled).toBe(true);
  await root.drive(byId(root, "live:q-a-Make Your Move").id, "click");
  await root.drive(byId(root, "live:q-skip").id, "click");
  await root.render(
    <AgentView
      publication={withRows([
        { ...question, chosen: { a: ["Make Your Move"] } },
      ])}
      send={send}
    />,
  );
  expect(byId(root, "live:q-a-Make Your Move").props["aria-pressed"]).toBe(
    true,
  );
  await root.drive(byId(root, "live:q-answer").id, "click");
  expect(sent).toEqual([
    {
      type: "chooseAnswer",
      step: "q",
      question: "a",
      option: "Make Your Move",
    },
    { type: "answer", step: "q", skip: true },
    { type: "answer", step: "q", skip: false },
  ]);
  root.dispose();
});
test("settled questions show the answer, Skipped or Not answered, with no actions", async () => {
  const { root, commits } = mount();
  const q = (status: string, answers: Record<string, string>): Row => ({
    id: `q-${status}-${Object.keys(answers).length}`,
    commandId: "q",
    chosen: {},
    open: false,
    value: {
      kind: "question",
      status,
      answers,
      questions: [{ id: "q", question: "Apply the same entrance to Kicker?" }],
    },
  });
  await root.render(
    <AgentView
      publication={withRows([
        q("answered", { q: "••••••" }),
        q("answered", {}),
        q("closed", {}),
      ])}
      send={() => {}}
    />,
  );
  expect(has(root, "Answered: ••••••")).toBe(true);
  expect(has(root, "Skipped")).toBe(true);
  expect(has(root, "Not answered")).toBe(true);
  expect(root.inspect().nodes.some((n) => n.props.label === "Answer")).toBe(
    false,
  );
  assertWire(commits);
  root.dispose();
});
test("attachments open and remove by id; the armed one is marked", async () => {
  const { root, commits } = mount();
  const p = fixture();
  const actions: Action[] = [];
  p.snapshot.attachments = [
    { id: "file", name: "Authored.png", type: "image", payloadAvailable: true },
  ];
  p.snapshot.draft.armed = "file";
  await root.render(
    <AgentView publication={p} send={(a) => actions.push(a)} />,
  );
  for (const label of ["Authored.png, attachment", "Remove Authored.png"]) {
    const n = root
      .inspect()
      .nodes.find((n) =>
        n.type === "button" || n.props.role === "button"
          ? n.props.label === label
          : false,
      )!;
    await root.drive(n.id, "click");
  }
  expect(actions).toEqual([
    { type: "openAttachment", id: "file" },
    { type: "removeAttachment", id: "file" },
  ]);
  assertWire(commits);
  root.dispose();
});
test("signed out, the panel offers to connect; a stopped service reads as an error with Try Again", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  await root.render(
    <AgentView
      publication={state(
        {},
        { connection: "unavailable", signedOut: true, reason: "" },
      )}
      send={(a) => actions.push(a)}
    />,
  );
  expect(has(root, "Connect your agent")).toBe(true);
  await root.drive(byId(root, "agent-connect-action").id, "click");
  await root.render(
    <AgentView
      publication={state(
        {},
        {
          connection: "unavailable",
          reason: "Powermove's built-in ChatGPT runtime is missing.",
        },
      )}
      send={(a) => actions.push(a)}
    />,
  );
  await root.drive(byId(root, "agent-connect-error-retry").id, "click");
  expect(actions).toEqual([{ type: "connect" }, { type: "retryConnection" }]);
  root.dispose();
});
test("a live run keeps its conversation and Stop after its account signs out", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  await root.render(
    <AgentView
      publication={state({ runId: "run" }, { connection: "unavailable" })}
      send={(a) => actions.push(a)}
    />,
  );
  expect(has(root, "Connect your agent")).toBe(false);
  await root.drive(byId(root, "agent-stop").id, "click");
  expect(actions).toEqual([{ type: "stop" }]);
  root.dispose();
});
test("typing / opens the original's shortcuts in the window's menu; a pick reaches the model", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  const asked: MenuOption[][] = [];
  const menu: Menu = async (r) => {
    asked.push(r.options);
    return "new";
  };
  const p = state({
    draft: { text: "", selection: [0, 0], armed: null },
  });
  await root.render(
    <AgentView publication={p} menu={menu} send={(a) => actions.push(a)} />,
  );
  // Typing says the composer is focused, and gives the menu its anchor.
  await root.drive(byId(root, "agent-composer-text").id, "input", {
    value: "/",
  });
  p.snapshot.draft = { text: "/", selection: [1, 1], armed: null };
  p.snapshot.slash = {
    visible: true,
    index: 0,
    choices: [
      { id: "new", value: null },
      { id: "settings", value: null },
    ],
  };
  await root.render(
    <AgentView
      publication={{ ...p }}
      menu={menu}
      send={(a) => actions.push(a)}
    />,
  );
  await Promise.resolve();
  expect(asked[0]!.map((o) => o.label)).toEqual([
    "/new — Start a new conversation",
    "/settings — Open agent connections",
  ]);
  expect(actions.slice(-1)).toEqual([{ type: "chooseSlash", id: "new" }]);
  expect(actions).toContainEqual({ type: "focus", focused: true });
  root.dispose();
});
test("a run the person stopped reads plainly, with Try Again", async () => {
  const { root, commits } = mount();
  const actions: Action[] = [];
  const rows: Row[] = [
    turn()[0]!,
    {
      id: "history:c",
      commandId: "history:c",
      chosen: {},
      open: false,
      value: {
        role: "assistant",
        text: "The Codex run was cancelled.",
        error: true,
      },
    },
  ];
  await root.render(
    <AgentView publication={withRows(rows)} send={(a) => actions.push(a)} />,
  );
  expect(
    has(root, "Something needs attention. The Codex run was cancelled."),
  ).toBe(false);
  expect(
    root.inspect().nodes.some((n) => n.text === "The Codex run was cancelled."),
  ).toBe(true);
  await root.drive(byId(root, "error-history:c-retry").id, "click");
  expect(actions).toEqual([{ type: "retry", id: "history:c" }]);
  assertWire(commits);
  root.dispose();
});
test("media and visual replies a streamed reply names get their cards", async () => {
  const { root, commits } = mount();
  const actions: Action[] = [];
  const rows: Row[] = [
    turn()[0]!,
    {
      id: "live:t",
      commandId: "t",
      chosen: {},
      open: false,
      value: {
        id: "t",
        kind: "text",
        text: "Saw: take.wav",
        media: [
          {
            kind: "audio",
            path: "/tmp/take.wav",
            name: "take.wav",
            remote: false,
          },
        ],
        visualReplies: [{ id: "v1", title: "Timing chart" }],
      },
    },
  ];
  await root.render(
    <AgentView publication={withRows(rows)} send={(a) => actions.push(a)} />,
  );
  expect(root.inspect().nodes.some((n) => n.text === "Sound")).toBe(true);
  await root.drive(byId(root, "live:t-media-0").id, "click");
  await root.drive(byId(root, "live:t-v1").id, "click");
  expect(actions).toEqual([
    { type: "openMedia", path: "/tmp/take.wav", reveal: false },
    { type: "openVisual", id: "v1" },
  ]);
  assertWire(commits);
  root.dispose();
});
test("working timer copy", () => {
  expect(elapsed(62_999)).toBe("1:02");
  expect(elapsed(-1)).toBe("0:00");
});
test("mounted panel listens to model publications, forwards errors and unsubscribes on disposal", async () => {
  const { root } = mount();
  let p = fixture();
  let update = () => {};
  let stops = 0;
  const errors: unknown[] = [];
  const model: AgentModel = {
    inspect: () => p,
    subscribe: (callback) => {
      update = callback;
      return () => {
        stops++;
      };
    },
    dispatch: async () => {
      throw new Error("rejected");
    },
  };
  await root.render(
    <AgentPanel model={model} onError={(e) => errors.push(e)} />,
  );
  await root.drive(byId(root, "agent-new").id, "click");
  await Promise.resolve();
  expect(errors.map(String)).toEqual(["Error: rejected"]);
  p = states.working();
  update();
  await root.flush();
  expect(has(root, "Stop current run")).toBe(true);
  await root.dispose();
  expect(stops).toBe(1);
});
test("composer key forwarding keeps IME and repeat flags for the model", async () => {
  const seen: Action[] = [];
  for (const key of [
    { key: "c", metaKey: true },
    { key: "Enter", composing: true },
    { key: "Escape" },
  ])
    seen.push(
      composerKey({
        ...key,
        type: "keydown",
        target: 1,
        currentTarget: 1,
        defaultPrevented: false,
        preventDefault() {},
        stopPropagation() {},
        stopImmediatePropagation() {},
      }),
    );
  expect(seen[1]).toMatchObject({
    type: "key",
    key: { key: "Enter", composing: true },
  });
  expect(seen[2]).toMatchObject({ type: "key", key: { key: "Escape" } });
});
test("the Agent panel bundles and mounts without Node, Chromium, document or window", async () => {
  const { runInNewContext } = await import("node:vm");
  const { fileURLToPath } = await import("node:url");
  const build = await Bun.build({
    entrypoints: [
      fileURLToPath(new URL("./nodefree-fixture.tsx", import.meta.url)),
    ],
    target: "browser",
    format: "iife",
    minify: true,
  });
  expect(build.success).toBe(true);
  const context: {
    agentProbe?: Promise<unknown>;
    structuredClone: typeof structuredClone;
  } = { structuredClone };
  runInNewContext(await build.outputs[0].text(), context);
  expect(await context.agentProbe).toEqual({
    panel: true,
    process: "undefined",
    window: "undefined",
    document: "undefined",
    buffer: "undefined",
  });
});
test("view styles contain no literal visual values; mutation probes catch regressions", async () => {
  const ts = await import("typescript");
  const check = (source: string) => {
    const ast = ts.createSourceFile(
      "view.tsx",
      source,
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
    const violations: string[] = [];
    const visual =
      /^(color|background|.*Color|fontSize|fontFamily|fontWeight|lineHeight|symbolSize|height|width|minWidth|maxWidth|minHeight|maxHeight|padding.*|margin.*|gap|.*Radius|.*Width|duration|response|bounce|startScale)$/;
    const visit = (node: import("typescript").Node) => {
      if (
        ts.isPropertyAssignment(node) &&
        visual.test(node.name.getText(ast))
      ) {
        const value = node.initializer;
        if (ts.isStringLiteral(value) || ts.isNumericLiteral(value))
          violations.push(node.getText(ast));
      }
      ts.forEachChild(node, visit);
    };
    visit(ast);
    return violations;
  };
  expect(
    check(await Bun.file(new URL("./view.tsx", import.meta.url)).text()),
  ).toEqual([]);
  expect(
    check('const style={color:"#123456",fontSize:12,height:32,duration:0.3}'),
  ).toHaveLength(4);
});
