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

test("the empty thread starts at the top of the conversation above the pinned composer", async () => {
  const { root } = mount();
  await root.render(<AgentView publication={fixture()} send={() => {}} />);
  const scroll = byId(root, "agent-conversation");
  const welcome = byId(root, "agent-welcome");
  const composer = byId(root, "agent-composer");
  expect(composer.props.style.flexShrink).toBe(0);
  expect(scroll.props.style.overflow).toBe("scroll");
  expect(scroll.props.metadata.scrollAnchor).toBe("start");
  const content = byId(root, "agent-conversation-content");
  expect(scroll.children).toContain(content.id);
  expect(content.children).toContain(welcome.id);
  expect(byId(root, "agent-panel").children).toContain(composer.id);
  expect(welcome.props.style.paddingTop).toEqual({ token: "metrics.agent_gap" });
  expect(welcome.props.style.flexGrow).toBeUndefined();
  expect(welcome.props.style.justifyContent).toBeUndefined();
  for (const label of ["Animate a title", "Shape the scene", "Arrange my workspace"]) {
    expect(byId(root, "agent-suggestions").children).toContain(byLabel(root, label).id);
  }
  expect(byId(root, "agent-welcome-description")).toBeDefined();
  await root.drive(scroll.id, "scroll", { value: { compact: true } });
  expect(byId(root, "agent-welcome-description")).toBeUndefined();
  const compactTitle = byId(root, "agent-welcome-title").props.style;
  expect(compactTitle.fontSize).toEqual({ token: "typography.title" });
  expect(compactTitle.lineHeight).toEqual({ token: "typography.dense_leading" });
  expect(byId(root, "agent-welcome").props.style.gap).toEqual({ token: "space.hair" });
  expect(has(root, "Arrange my workspace")).toBe(true);
  expect(byLabel(root, "Arrange my workspace").props.style.minHeight).toEqual({ token: "metrics.hit" });
  await root.drive(scroll.id, "scroll", { value: { compact: false } });
  expect(byId(root, "agent-welcome-description")).toBeDefined();
  root.dispose();
});

test("the user message keeps a fill and radius without a border while working and after finishing", async () => {
  const { root } = mount();
  const p = states.selected();
  p.snapshot.rows.items = p.snapshot.rows.items.slice(0, 1);
  p.snapshot.rows.count = 1;
  p.snapshot.rows.range = [0, 1];
  for (const runId of [null, "run", null]) {
    p.snapshot.runId = runId;
    p.snapshot.elapsedMs = runId ? 11_000 : null;
    await root.render(<AgentView publication={p} send={() => {}} />);
    const message = byLabel(root, "You said: Make the headline");
    const content = node(root, (n) => n.id === message.children[0]);
    const bubble = node(root, (n) => n.id === content.children[0]);
    expect(bubble.props.style).toMatchObject({
      background: { token: "colors.field" },
      borderRadius: { token: "metrics.panel_radius" },
    });
    expect(bubble.props.style).not.toHaveProperty("borderWidth");
    expect(bubble.props.style).not.toHaveProperty("borderColor");
  }
  root.dispose();
});

test("focus order follows the thread bar, conversation and the composer's one row, left to right", async () => {
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
    "quote-source-history:message-0",
    "quote-source-live:text-1",
    "agent-composer-text",
    "agent-add",
    "agent-model",
    "agent-effort",
    "agent-access",
    "agent-send",
  ]);
  expect(byId(root, "agent-conversation").props.label).toBe(
    "Agent conversation",
  );
  root.dispose();
});
test("the model's name gives way in the docked composer's one row; effort keeps its width", async () => {
  const { root } = mount();
  const p = states.selected();
  p.native.presentation = "docked";
  p.snapshot.catalog.efforts = ["high"];
  p.snapshot.settings.effort = "high";
  await root.render(<AgentView publication={p} send={() => {}} />);
  const nodes = root.inspect().nodes;
  const label = (picker: ReturnType<typeof byId>) =>
    nodes.find((n) => picker.children.includes(n.id) && n.type === "text");
  const model = byId(root, "agent-model");
  expect(model.props.style).toMatchObject({ flexShrink: 1, minWidth: { token: "metrics.hit" } });
  expect(label(model)?.props.style).toMatchObject({ flexShrink: 1, textOverflow: "ellipsis" });
  const effort = byId(root, "agent-effort");
  expect(effort.props.style).toMatchObject({ flexShrink: 0 });
  expect(label(effort)?.props.style).toMatchObject({ flexShrink: 0 });
  expect(label(effort)?.props.metadata).toBeUndefined();
  // Every control sits in the composer's one row, on its one surface: "+",
  // the model chooser (model and effort), and Approvals.
  const options = byId(root, "agent-options");
  for (const id of ["agent-add", "agent-chooser", "agent-access"])
    expect(options.children).toContain(byId(root, id).id);
  for (const id of ["agent-model", "agent-effort"])
    expect(byId(root, "agent-chooser").children).toContain(byId(root, id).id);
  expect(byId(root, "agent-composer").children).toContain(options.id);
  expect(byId(root, "agent-threads")).toBeDefined();
  expect(byId(root, "agent-new")).toBeDefined();
  root.dispose();
});

test("the composer is one surface: the draft sits on it with no inset field, its corner concentric with the panel or popover", async () => {
  const { root } = mount();
  for (const presentation of ["docked", "popover"] as const) {
    const p = fixture();
    p.native.presentation = presentation;
    await root.render(<AgentView publication={p} menu={async () => null} send={() => {}} />);
    const composer = byId(root, "agent-composer");
    expect(composer.props.style).toMatchObject({
      borderRadius: {
        token: presentation === "popover" ? "metrics.composer_popover_radius" : "metrics.composer_radius",
      },
      borderWidth: { token: "metrics.border" },
      borderColor: { token: "colors.hairline" },
      background: { token: presentation === "popover" ? "colors.raised" : "colors.floating" },
    });
    expect(root.inspect().nodes.some((n) => n.props.id === "agent-field")).toBe(false);
    // The draft is written on the surface: its field takes the surface's own
    // fill (never the kit's well).
    const text = byId(root, "agent-composer-text");
    expect(composer.children).toContain(text.id);
    expect(text.props.style.background).toEqual(composer.props.style.background);
    // Pickers are quiet on the surface until their menu is open.
    for (const id of ["agent-model", "agent-effort", "agent-access", "agent-add"])
      expect(byId(root, id).props.style).not.toHaveProperty("background");
  }
  root.dispose();
});

test("new thread, send and stop use the model's commands; Send hides while working on an empty draft", async () => {
  const { root, commits } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.draft.text = "A new request";
  p.snapshot.sendable = true;
  const render = () =>
    root.render(<AgentView publication={p} send={(a) => actions.push(a)} />);
  await render();
  await root.drive(byId(root, "agent-new").id, "click");
  await root.drive(byId(root, "agent-send").id, "click");
  p.snapshot.runId = "run";
  p.snapshot.draft.text = "";
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
test("provider and model share one menu; effort and approvals each have their own; picks reach the model", async () => {
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
  const picks = ["provider:claude", "model:gpt-6-mini", "low", "auto", "attach"];
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
  expect(byId(root, "agent-access").props.label).toBe("Approvals, Composition only");
  for (const id of [
    "agent-model",
    "agent-model",
    "agent-effort",
    "agent-access",
    "agent-add",
  ]) {
    await root.drive(byId(root, id).id, "click");
    await Promise.resolve();
  }
  // One place for each: the model menu has no effort rows.
  expect([...new Set(asked[0]!.map((o) => o.category))]).toEqual([
    "ChatGPT",
    "Claude",
  ]);
  expect(asked[2]!.map((o) => [o.label, o.checked])).toEqual([
    ["Low", false],
    ["High", true],
  ]);
  // Approvals: its own picker in the composer row.
  expect(asked[3]!.map((o) => [o.label, o.checked ?? false])).toEqual([
    ["Composition only", true],
    ["Supervised", false],
    ["Auto", false],
    ["Full access", false],
  ]);
  // "+": attachments only.
  expect(asked[4]!.map((o) => o.label)).toEqual([
    "Add Files…",
    "Select Part of the Screen",
  ]);
  expect(actions).toEqual([
    { type: "setProvider", provider: "claude" },
    { type: "setModel", model: "gpt-6-mini", effort: "high" },
    { type: "setModel", model: "gpt-6", effort: "low" },
    { type: "setAccess", access: "auto" },
    { type: "attach" },
  ]);
  // Full access highlights the composer's Approvals button in warning ink.
  p.snapshot.settings.access = "full";
  await root.render(
    <AgentView publication={p} menu={menu} send={(a) => actions.push(a)} />,
  );
  const accessNode = byId(root, "agent-access");
  expect(accessNode.props.label).toBe("Approvals, Full access");
  const accessText = root.inspect().nodes.find((n) => accessNode.children.includes(n.id) && n.type === "text");
  expect(accessText?.props.style.color).toEqual({ token: "colors.warning" });
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
  const models = asked[0]!.filter((o) => o.category === "ChatGPT");
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
  expect(asked[1]!.filter((o) => o.category === "ChatGPT")).toEqual([]);
  expect(asked.flat().some((o) => /^Default/.test(o.label))).toBe(false);
  root.dispose();
});
test("a provider pick keeps the model menu open and lists that provider's models in it", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.catalog = {
    models: [["gpt-6", "GPT-6"]],
    providers: [
      ["chatgpt", "ChatGPT"],
      ["claude", "Claude"],
    ],
    efforts: [],
  };
  p.snapshot.settings.model = "gpt-6";
  const requests: Parameters<Menu>[0][] = [];
  const answers = ["provider:claude", "model:sonnet"];
  const menu: Menu = async (r) => {
    requests.push(r);
    return answers.shift() ?? null;
  };
  const send = (a: Action) => actions.push(a);
  await root.render(<AgentView publication={p} menu={menu} send={send} />);
  await root.drive(byId(root, "agent-model").id, "click");
  await Promise.resolve();
  // Provider rows keep the menu up; model rows close it.
  const first = requests[0]!.options;
  expect(first.find((o) => o.value === "provider:claude")?.stays).toBe(true);
  expect(first.find((o) => o.value === "model:gpt-6")?.stays).toBeUndefined();
  expect(actions).toEqual([{ type: "setProvider", provider: "claude" }]);
  // The switch publishes Claude's models: they go to the same open menu,
  // its default ticked, and a model pick there runs.
  const q = structuredClone(p);
  q.snapshot.settings.provider = "claude";
  q.snapshot.settings.model = "opus";
  q.snapshot.catalog.models = [
    ["opus", "Opus 5.5"],
    ["sonnet", "Sonnet 5.5"],
  ];
  await root.render(<AgentView publication={q} menu={menu} send={send} />);
  for (let i = 0; i < 4; i++) await Promise.resolve();
  expect(requests.length).toBe(2);
  expect(requests[1]!.update).toBe(true);
  expect(
    requests[1]!.options
      .filter((o) => o.category === "Claude")
      .map((o) => [o.label, o.checked]),
  ).toEqual([
    ["Opus 5.5", true],
    ["Sonnet 5.5", false],
  ]);
  expect(actions.at(-1)).toEqual({
    type: "setModel",
    model: "sonnet",
    effort: q.snapshot.settings.effort,
  });
  root.dispose();
});
test("Claude discovery updates the already-open picker without reopening it", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.catalog.providers = [
    ["chatgpt", "ChatGPT"],
    ["claude", "Claude"],
    ["compatible", "Connected model"],
  ];
  p.snapshot.settings.provider = "claude";
  p.snapshot.settings.model = "opus";
  p.snapshot.catalog.models = [["opus", "Opus 5.5"]];
  p.native.findingModels = true;
  const requests: Parameters<Menu>[0][] = [];
  let choose!: (value: string) => void;
  const menu: Menu = (request) => {
    requests.push(request);
    return request.refresh
      ? Promise.resolve(null)
      : new Promise((resolve) => {
          choose = resolve;
        });
  };
  const actions: Action[] = [];
  const send = (action: Action) => actions.push(action);
  await root.render(<AgentView publication={p} menu={menu} send={send} />);
  await root.drive(byId(root, "agent-model").id, "click");
  expect(
    requests[0]!.options
      .filter((o) => o.category === "Claude")
      .map((o) => [o.label, o.informational]),
  ).toEqual([
    ["Opus 5.5", undefined],
    ["Finding models…", true],
  ]);

  // A failed discovery does not change the catalog, but removes its notice.
  const failed = structuredClone(p);
  failed.native.findingModels = false;
  await root.render(<AgentView publication={failed} menu={menu} send={send} />);
  expect(requests).toHaveLength(2);
  expect(requests[1]!.refresh).toBe(true);
  expect(requests[1]!.options.some((o) => o.informational)).toBe(false);
  await root.render(<AgentView publication={p} menu={menu} send={send} />);
  expect(requests).toHaveLength(3);
  expect(requests[2]!.options.some((o) => o.informational)).toBe(true);

  const discovered = structuredClone(p);
  discovered.snapshot.catalog.models.push(
    ["fable", "Fable 5.2"],
    ["sonnet", "Sonnet 5.5"],
    ["haiku", "Haiku 5.5"],
  );
  await root.render(
    <AgentView publication={discovered} menu={menu} send={send} />,
  );
  expect(requests).toHaveLength(4);
  expect(requests[3]!.update).toBe(true);
  expect(requests[3]!.refresh).toBe(true);
  expect(
    requests[3]!.options
      .filter((o) => o.category === "Claude")
      .map((o) => [o.label, o.checked]),
  ).toEqual([
    ["Opus 5.5", true],
    ["Fable 5.2", false],
    ["Sonnet 5.5", false],
    ["Haiku 5.5", false],
  ]);
  await root.render(
    <AgentView
      publication={structuredClone(discovered)}
      menu={menu}
      send={send}
    />,
  );
  expect(requests).toHaveLength(4);
  choose("model:sonnet");
  await Promise.resolve();
  await Promise.resolve();
  expect(actions).toEqual([
    { type: "setModel", model: "sonnet", effort: p.snapshot.settings.effort },
  ]);
  root.dispose();
});
test("composer pickers use the pressed anchor and enable search only for a long model list", async () => {
  const { root } = mount();
  const p = fixture();
  const requests: Parameters<Menu>[0][] = [];
  const menu: Menu = async (request) => {
    requests.push(request);
    return null;
  };
  const render = () => root.render(<AgentView publication={p} menu={menu} send={() => {}} />);
  await render();
  for (const id of ["agent-model", "agent-effort", "agent-access", "agent-add"]) {
    const button = byId(root, id);
    await root.drive(button.id, "click");
    // The effort part's menu is the pill's own reasoning menu (no search).
    expect(requests.at(-1)).toMatchObject(
      id === "agent-effort"
        ? { anchor: button.id, picker: "chooser", menu: "effort" }
        : id === "agent-access"
          ? { anchor: button.id, picker: "access", search: false }
          : { anchor: button.id, search: false },
    );
    await Promise.resolve();
  }
  for (const length of [12, 13]) {
    p.snapshot.catalog.models = Array.from({ length }, (_, i) => [`model-${i}`, `Model ${i}`]);
    p.snapshot.settings.model = "model-0";
    await render();
    await root.drive(byId(root, "agent-model").id, "click");
    expect(requests.at(-1)).toMatchObject({ search: length > 12, placeholder: "Search models" });
  }
  root.dispose();
});
test("model menu groups discovered choices by provider and switches directly to a model", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.settings.model = "gpt-6.2-sol";
  p.snapshot.catalog.models = [["gpt-6.2-sol", "GPT-6.2 Sol"]];
  p.snapshot.catalog.providers = [
    ["chatgpt", "ChatGPT"], ["claude", "Claude"], ["compatible", "gpt-6-astra"],
  ];
  p.native.modelGroups = {
    chatgpt: [["gpt-6.2-sol", "GPT-6.2 Sol", true]],
    claude: [["opus", "Opus 5.5", true], ["sonnet", "Sonnet 5.5", false]],
    compatible: [["gpt-6-astra", "gpt-6-astra", true]],
  };
  p.native.connectedModelName = "GPT-6 Astra";
  const actions: Action[] = [];
  const requests: Parameters<Menu>[0][] = [];
  await root.render(<AgentView publication={p} send={(a) => actions.push(a)} menu={async (request) => {
    requests.push(request);
    return "other:claude:sonnet";
  }} />);
  const model = byId(root, "agent-model");
  await root.drive(model.id, "keydown", { key: "Enter" });
  expect(requests[0]!.options.map(({ label, category, checked, default: isDefault }) =>
    [category, label, checked, isDefault ?? false],
  )).toEqual([
    ["ChatGPT", "GPT-6.2 Sol", true, true],
    ["Claude", "Opus 5.5", false, true],
    ["Claude", "Sonnet 5.5", false, false],
    ["GPT-6 Astra", "gpt-6-astra", false, true],
  ]);
  expect(actions).toEqual([{ type: "setProvider", provider: "claude", model: "sonnet" }]);
  await root.drive(model.id, "keydown", { key: " " });
  expect(requests).toHaveLength(2);
  root.dispose();
});
test("suggestions read as buttons: a raised fill with a hairline edge, the label in body text", async () => {
  const { root } = mount();
  await root.render(
    <AgentView
      publication={states.empty()}
      menu={async () => null}
      send={() => {}}
    />,
  );
  const tokens = (v: unknown) => JSON.stringify(v);
  for (const label of [
    "Animate a title",
    "Shape the scene",
    "Arrange my workspace",
  ]) {
    const b = byLabel(root, label);
    const style = b.props.style as Record<string, unknown>;
    expect(b.props.role).toBe("button");
    expect(tokens(style.background)).toContain("colors.raised");
    expect(tokens(style.borderColor)).toContain("colors.hairline");
    expect(tokens(style.minHeight)).toContain("metrics.control");
    const parts = root
      .inspect()
      .nodes.filter((n: any) => b.children?.includes(n.id) ?? false);
    const styles = tokens(parts.map((n: any) => n.props.style));
    // Body-text label, secondary chevron: not the quiet faint pair.
    expect(styles).toContain("colors.text");
    expect(styles).toContain("colors.secondary");
    expect(styles).not.toContain("colors.muted");
  }
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
  for (const label of ["Open Authored.png", "Remove Authored.png"]) {
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
test("a saved cancellation has no transcript message or retry button", async () => {
  const { root, commits } = mount();
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
  await root.render(<AgentView publication={withRows(rows)} send={() => {}} />);
  const result = assertWire(commits);
  expect(JSON.stringify(result.nodes)).not.toContain(
    "The Codex run was cancelled.",
  );
  expect(has(root, "Try Again")).toBe(false);
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

test("composer sends direction with a draft during a run", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.runId = "run";
  p.snapshot.draft.text = "Make it blue";
  p.snapshot.sendable = true;
  await root.render(<AgentView publication={p} send={(a) => actions.push(a)} />);
  expect(byId(root, "agent-composer-text").props.disabled).not.toBe(true);
  expect(byId(root, "agent-send").props.disabled).toBe(false);
  // Stop steps back beside Send: a dim disc, and Send is the lit one.
  const disc = (id: string) => root.inspect().nodes.find((n) => byId(root, id).children.includes(n.id))!;
  expect(disc("agent-stop").props.style).toMatchObject({ background: { token: "colors.composer_send_idle" } });
  expect(disc("agent-send").props.style).toMatchObject({ background: { token: "colors.text" } });
  await root.drive(byId(root, "agent-send").id, "click");
  expect(actions).toEqual([{ type: "submit" }]);
});

test("each attachment kind has its tile, and file drops route to native attachment input", async () => {
  const { root } = mount();
  const p = fixture();
  const actions: Action[] = [];
  p.snapshot.attachments = [
    "frame.png",
    "clip.mov",
    "sound.wav",
    "notes.pdf",
    "Frame 0:02 selection.png",
  ].map((name, i) => ({
    id: String(i),
    name,
    type: "",
    payloadAvailable: true,
  }));
  await root.render(
    <AgentView publication={p} send={(a) => actions.push(a)} />,
  );
  expect(
    root
      .inspect()
      .nodes.filter((n) => (n.props.metadata as any)?.kind)
      .map((n) => (n.props.metadata as any).kind),
  ).toEqual(["image", "video", "audio", "document", "pick"]);
  await root.drive(byId(root, "agent-composer").id, "dragover", {});
  await root.drive(byId(root, "agent-composer").id, "drop", {
    drag: {
      types: ["public.file-url"],
      data: { "public.file-url": ["/synthetic/frame.png"] },
      effect: "copy",
    },
  });
  expect(actions.at(-1)).toEqual({
    type: "attachInput",
    paths: ["/synthetic/frame.png"],
  });
  p.snapshot.attachments.push({
    id: "6",
    name: "six.txt",
    type: "text/plain",
    payloadAvailable: true,
  });
  await root.render(
    <AgentView publication={p} send={(a) => actions.push(a)} />,
  );
  // At six, "+" says so in place of its attachment rows.
  let asked: MenuOption[] = [];
  await root.render(
    <AgentView publication={p} send={(a) => actions.push(a)} menu={async (r) => { asked = r.options; return null; }} />,
  );
  await root.drive(byId(root, "agent-add").id, "click");
  expect(asked[0]).toMatchObject({ informational: true, label: "A message can carry six attachments" });
  expect(asked.some((o) => o.value === "attach" || o.value === "pickRegion")).toBe(false);
  root.dispose();
});

test("region chips and floating sent thumbnails open from their images", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  const region = (id: string, name: string) => ({
    id,
    name,
    type: "image/png",
    screenPick: true,
    payloadAvailable: true,
    thumbnail: id + "-thumb",
  });
  const p = withRows([
    {
      id: "history:attachment-only",
      commandId: "",
      chosen: {},
      open: false,
      value: { role: "user", attachments: [region("sent", "Sent region")] },
    },
    {
      id: "history:with-text",
      commandId: "",
      chosen: {},
      open: false,
      value: { role: "user", text: "Use this", attachments: [region("reply", "Reply")] },
    },
  ]);
  p.snapshot.attachments = [
    region("viewer", "Viewer · 0:03.43"),
    region("window", "Powermove"),
  ];
  await root.render(<AgentView publication={p} send={(a) => actions.push(a)} />);
  const nodes = root.inspect().nodes;
  const byNodeId = (id: number) => nodes.find((n) => n.id === id)!;
  const style = (n: (typeof nodes)[number]) => n.props.style as any;
  const texts = (n: (typeof nodes)[number]): string[] =>
    [n.text, ...n.children.flatMap((id) => texts(byNodeId(id)))].filter(Boolean);
  const chip = byLabel(root, "Open Viewer · 0:03.43");
  expect(style(chip).borderRadius).toEqual({ token: "metrics.inner_radius" });
  expect(texts(chip)).toEqual(["Viewer", "0:03.43"]);
  expect(texts(byLabel(root, "Open Powermove"))).toEqual(["Powermove", "Region"]);

  for (const id of ["viewer", "sent", "reply"]) {
    const image = node(root, (n) => n.props.resource === id + "-thumb");
    await root.drive(image.id, "click");
    expect(actions.at(-1)).toEqual({ type: "openAttachment", id });
  }
  const only = byNodeId(byLabel(root, "You said: Attached 1 file").children[0]!);
  expect(only.children).toHaveLength(1);
  expect(style(byNodeId(only.children[0]!)).background).toBeUndefined();
  const withText = byNodeId(byLabel(root, "You said: Use this").children[0]!);
  expect(withText.children).toHaveLength(2);
  expect(style(byNodeId(withText.children[0]!)).background).toBeUndefined();
  expect(style(byNodeId(withText.children[1]!)).background).toEqual({ token: "colors.field" });
  root.dispose();
});

test("full or disabled composers reject drops and clear their accepting highlight", async () => {
  const { root } = mount();
  const p = fixture();
  const actions: Action[] = [];
  const render = () =>
    root.render(<AgentView publication={p} send={(a) => actions.push(a)} />);
  p.native.dropTarget = true;
  p.snapshot.draft.text = "Keep the composer visible";
  await render();
  expect(byId(root, "agent-composer").props.state).toMatchObject({
    dropTarget: true,
  });
  for (const blocked of ["full", "disabled", "offline"]) {
    p.snapshot.attachments =
      blocked === "full"
        ? Array.from({ length: 6 }, (_, i) => ({
            id: String(i),
            name: "file.txt",
            type: "text/plain",
            payloadAvailable: true,
          }))
        : [];
    p.snapshot.composerDisabled = blocked === "disabled";
    p.native.connection = blocked === "offline" ? "unavailable" : "connected";
    await render();
    expect(byId(root, "agent-composer").props.state).toMatchObject({
      dropTarget: false,
    });
    await root.drive(byId(root, "agent-composer").id, "drop", {
      drag: {
        types: ["public.file-url"],
        data: { "public.file-url": ["/synthetic/file.txt"] },
        effect: "copy",
      },
    });
  }
  expect(actions.filter((a) => a.type === "attachInput")).toEqual([]);
  root.dispose();
});

test("borderless attachment tiles expose remove on hover and keep the armed wash", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.attachments = [
    {
      id: "image",
      name: "Image.png",
      type: "image/png",
      payloadAvailable: true,
      thumbnail: "synthetic-thumb",
    },
  ];
  await root.render(<AgentView publication={p} send={() => {}} />);
  expect(has(root, "Remove Image.png")).toBe(false);
  const group = byLabel(root, "Image.png, attachment");
  const tile = byLabel(root, "Open Image.png");
  expect((tile.props.style as any).width).toEqual({
    token: "metrics.agent_attachment",
  });
  expect((tile.props.style as any).borderWidth).toBeUndefined();
  await root.drive(group.id, "pointerenter");
  expect(has(root, "Remove Image.png")).toBe(true);
  await root.drive(group.id, "pointerleave");
  expect(has(root, "Remove Image.png")).toBe(false);
  await root.drive(tile.id, "focus");
  expect(has(root, "Remove Image.png")).toBe(true);
  await root.drive(tile.id, "blur");
  await new Promise((resolve) => setTimeout(resolve, 1));
  expect(has(root, "Remove Image.png")).toBe(false);
  p.snapshot.draft.armed = "image";
  await root.render(<AgentView publication={p} send={() => {}} />);
  expect(
    (byLabel(root, "Open Image.png").props.style as any).background,
  ).toEqual({ token: "colors.selected_fill" });
  expect(has(root, "Remove Image.png")).toBe(true);
  expect(
    (byLabel(root, "Image.png, attachment").props.metadata as any).armed,
  ).toBe(true);
  root.dispose();
});

test("composer pop-up triggers stay pressed and open while their menu is up", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.catalog.models = [["opus", "Opus 5.5 (1M context)"]];
  p.snapshot.settings.model = "opus";
  let close: (value: unknown) => void = () => {};
  const requests: Parameters<Menu>[0][] = [];
  const menu: Menu = (request) => {
    requests.push(request);
    return new Promise((resolve) => {
      close = resolve;
    });
  };
  await root.render(<AgentView publication={p} menu={menu} send={() => {}} />);
  const model = byId(root, "agent-model");
  expect(model.props.metadata).toEqual({ menuAnchor: true, pill: "part" });
  expect(model.props.role).toBe("button");
  expect(model.props.tabIndex).toBe(0);
  expect(model.props.title).toBeUndefined();
  expect(root.inspect().nodes.some((n) => n.text === "Opus 5.5")).toBe(true);
  await root.drive(model.id, "click");
  expect(byId(root, "agent-model").props.open).toBe(true);
  expect(byId(root, "agent-model").props["aria-expanded"]).toBe(true);
  // The host draws the pill on the open part (no fill of its own), and its
  // chevron turns to point up.
  expect((byId(root, "agent-model").props.style as any).background).toBeUndefined();
  const turns = () =>
    root
      .inspect()
      .nodes.filter((n) => n.props.metadata?.symbol === "composer.chevron")
      .map((n) => n.props.metadata.turn);
  expect(turns()).toEqual([true, false, false]);
  expect(
    requests[0]!.options.find((o) => o.value === "model:opus"),
  ).toMatchObject({ label: "Opus 5.5", detail: "1M context" });
  close(null);
  for (let i = 0; i < 4; i++) await Promise.resolve();
  await root.flush();
  expect(byId(root, "agent-model").props.open).toBe(false);
  expect(turns()).toEqual([false, false, false]);
  p.snapshot.runId = "busy";
  await root.render(<AgentView publication={p} menu={menu} send={() => {}} />);
  for (const id of ["agent-model", "agent-effort", "agent-access"])
    expect(byId(root, id).props.disabled).toBe(true);
  // During a run "+" still adds files; Approvals wait for the run to end.
  expect(byId(root, "agent-add").props.disabled).toBe(false);
  await root.drive(byId(root, "agent-add").id, "click");
  expect(requests.at(-1)!.options.some((o) => o.category === "Approvals")).toBe(false);
  close(null);
  root.dispose();
});

test("Space and Return reach the shell toggle for open composer pickers", async () => {
  const { root } = mount();
  const requests: Parameters<Menu>[0][] = [];
  let settle: ((value: unknown) => void) | undefined;
  const menu: Menu = (request) => {
    requests.push(request);
    if (settle) {
      settle(null);
      settle = undefined;
      return Promise.resolve(null);
    }
    return new Promise((resolve) => { settle = resolve; });
  };
  await root.render(<AgentView publication={fixture()} menu={menu} send={() => {}} />);
  for (const [id, key] of [["agent-model", "Enter"], ["agent-effort", " "], ["agent-access", "Enter"], ["agent-add", "Enter"]]) {
    const trigger = byId(root, id);
    await root.drive(trigger.id, "keydown", { key });
    expect(byId(root, id).props["aria-expanded"]).toBe(true);
    await root.drive(trigger.id, "keydown", { key });
    await root.flush();
    expect(byId(root, id).props["aria-expanded"]).toBe(false);
  }
  expect(requests.map(({ picker }) => picker)).toEqual([
    "model", "model", "chooser", "chooser", "access", "access", "add", "add",
  ]);
  root.dispose();
});

test("L2 fields are optional; only a current typed native plan can Apply", async () => {
  const { root } = mount();
  const p = fixture();
  const actions: Action[] = [];
  const render = () =>
    root.render(<AgentView publication={p} send={(a) => actions.push(a)} />);
  await render();
  expect(has(root, "Apply change")).toBe(false);
  p.snapshot.phase = "preview";
  p.snapshot.plan = {
    revision: 4,
    commands: [{ type: "add_layer", id: "title", layerType: "text" }],
    summary: "Add a title",
  };
  await render();
  expect(has(root, "Apply change")).toBe(false);
  p.snapshot.resultActions = { apply: true, keep: false, undo: false };
  await render();
  await root.drive(byLabel(root, "Apply change").id, "click");
  expect(actions).toEqual([{ type: "applyPlan" }]);
  p.snapshot.runId = "active";
  await render();
  expect(has(root, "Apply change")).toBe(false);
  p.snapshot.runId = null;
  p.snapshot.plan = {
    kind: "scene",
    sceneEdit: { label: "Legacy scene proposal", commands: [] },
    revision: 4,
    commands: [],
  };
  await render();
  expect(has(root, "Apply change")).toBe(false);
  expect(
    root.inspect().nodes.some((n) => n.text?.includes("Legacy scene proposal")),
  ).toBe(true);
  root.dispose();
});

test("L2 result actions belong to the current settled run and extensions use published panel pairs", async () => {
  const { root } = mount();
  const result = {
    runId: "settled",
    summary: "Created tools",
    changed: true,
    verificationError: "Preview verification was unavailable.",
    extensions: [
      {
        extensionId: "published-extension",
        panelId: "contributed-panel",
        name: "Tools",
        action: "created",
        status: "ready",
      },
      {
        extensionId: "no-panel",
        panelId: null,
        name: "Commands",
        action: "updated",
        status: "ready",
      },
      {
        extensionId: "failed",
        panelId: "bad-panel",
        name: "Failed",
        action: "updated",
        status: "error",
      },
      {
        extensionId: "removed",
        panelId: "old-panel",
        name: "Removed",
        action: "removed",
        status: "ready",
      },
    ],
  } as const;
  const row: Row = {
    id: "result-row",
    commandId: "",
    chosen: {},
    open: false,
    value: {
      role: "assistant",
      text: "Created tools",
      nativeResult: result as any,
    },
  };
  const p = withRows([row]);
  p.snapshot.phase = "result";
  p.snapshot.result = structuredClone(result) as any;
  p.snapshot.resultActions = { apply: false, keep: true, undo: true };
  const actions: Action[] = [];
  const render = () =>
    root.render(<AgentView publication={p} send={(a) => actions.push(a)} />);
  await render();
  for (const label of ["Keep Change", "Undo Change", "Open panel, Tools"])
    await root.drive(byLabel(root, label).id, "click");
  expect(actions).toEqual([
    { type: "keepResult" },
    { type: "undoResult" },
    {
      type: "openResultPanel",
      extensionId: "published-extension",
      panelId: "contributed-panel",
    },
  ]);
  for (const name of ["Commands", "Failed", "Removed"])
    expect(has(root, `Open panel, ${name}`)).toBe(false);
  expect(
    root
      .inspect()
      .nodes.some((n) => n.text?.includes(result.verificationError)),
  ).toBe(true);
  p.snapshot.resultActions = { apply: false, keep: false, undo: false };
  await render();
  expect(has(root, "Keep Change")).toBe(false);
  expect(has(root, "Undo Change")).toBe(false);
  p.snapshot.resultActions = { apply: false, keep: true, undo: true };
  p.snapshot.result!.runId = "newer-result";
  await render();
  expect(has(root, "Keep Change")).toBe(false);
  expect(has(root, "Undo Change")).toBe(false);
  p.snapshot.result = undefined;
  p.snapshot.resultActions = undefined;
  await render();
  expect(has(root, "Undo Change")).toBe(false);
  expect(has(root, "Rendered result")).toBe(true);
  root.dispose();
});

test("child thread menus nest under their parent and the path returns to each ancestor", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.threadId = "grandchild";
  p.snapshot.threads = [
    {
      id: "grandchild",
      parentId: "child",
      kind: "subagent",
      title: "Polish",
      busy: true,
      updatedAt: 3,
    },
    { id: "root", title: "Title card", busy: true, updatedAt: 1 },
    {
      id: "child",
      parentId: "root",
      kind: "subagent",
      title: "Layout",
      busy: true,
      updatedAt: 2,
    },
  ];
  const actions: Action[] = [];
  let request: Parameters<Menu>[0] | undefined;
  await root.render(
    <AgentView
      publication={p}
      send={(a) => actions.push(a)}
      menu={async (r) => {
        request = r;
        return null;
      }}
    />,
  );
  expect(has(root, "Thread path")).toBe(true);
  await root.drive(byLabel(root, "Back to Layout").id, "click");
  expect(actions).toEqual([{ type: "switchThread", id: "child" }]);
  await root.drive(byId(root, "agent-threads").id, "click");
  expect(
    request!.options
      .filter((o) => String(o.value).startsWith("thread:"))
      .map((o) => o.label),
  ).toEqual([
    "Title card · Working…",
    "  ↳ Layout · Working…",
    "    ↳ Polish · Working…",
  ]);
  root.dispose();
});

test("saved stop and empty completion placeholders stay out of the transcript", async () => {
  const { root } = mount();
  const messages: Array<Row["value"] & { text: string }> = [
    { role: "user", text: "Check the opening shot." },
    { role: "assistant", text: "I found the opening shot." },
    { role: "assistant", error: true, text: "The run was stopped." },
    { role: "assistant", error: true, text: "The task was stopped." },
    { role: "assistant", error: true, text: "Agent runtime stopped." },
    {
      role: "assistant",
      error: true,
      text: "ChatGPT generation completed without a result.",
    },
    {
      role: "assistant",
      error: true,
      text: "The provider stopped accepting requests. Sign in and try again.",
    },
    { role: "assistant", text: "Done." },
  ];
  const publication = withRows(
    messages
      .slice(1)
      .flatMap((value) => [messages[0]!, value])
      .map((value, index) => ({
        id: `history:${index}`,
        commandId: "",
        chosen: {},
        open: false,
        value,
      })),
  );
  try {
    await root.render(<AgentView publication={publication} send={() => {}} />);
    const transcript = JSON.stringify(root.inspect().nodes);
    for (const { text } of messages.slice(2, 6))
      expect(transcript).not.toContain(text);
    expect(transcript).toContain(messages[1]!.text);
    expect(transcript).toContain(messages[6]!.text);
    expect(transcript).toContain(messages[7]!.text);
    expect(
      root.inspect().nodes.filter((n) => n.props.label === "Try Again"),
    ).toHaveLength(1);
  } finally {
    root.dispose();
  }
});

test("saved cancelled tasks keep their status and omit generated output", async () => {
  const { root } = mount();
  const publication = withRows([
    {
      id: "history:task",
      commandId: "",
      chosen: {},
      open: false,
      value: {
        kind: "tool",
        id: "task",
        toolName: "delegate_task",
        label: "Check the cut",
        status: "done",
        output: "The task was stopped.",
        task: { taskId: "task", status: "cancelled", title: "Check the cut" },
      },
    },
  ]);
  try {
    await root.render(<AgentView publication={publication} send={() => {}} />);
    await root.drive(byLabel(root, "1 task").id, "click");
    expect(root.inspect().nodes.some((n) => n.text === "Stopped")).toBe(true);
    expect(
      root
        .inspect()
        .nodes.some(
          (n) => n.props.label === "Check the cut" && n.type === "button",
        ),
    ).toBe(false);
  } finally {
    root.dispose();
  }
});

test("a rendered result keeps the change card without placeholder prose", async () => {
  const { root } = mount();
  const publication = withRows([
    {
      id: "history:result",
      commandId: "",
      chosen: {},
      open: false,
      value: {
        role: "assistant",
        text: "",
        nativeResult: { runId: "r", changed: true },
      },
    },
  ]);
  try {
    await root.render(<AgentView publication={publication} send={() => {}} />);
    expect(has(root, "Rendered result")).toBe(true);
    expect(JSON.stringify(root.inspect().nodes)).not.toContain(
      "The rendered change is ready.",
    );
  } finally {
    root.dispose();
  }
});

test("a real provider timeout keeps the failure card and its details", async () => {
  const { root } = mount();
  const failure = "The provider stopped responding and timed out.";
  const publication = withRows([
    turn()[0]!,
    {
      id: "history:provider-error",
      commandId: "",
      chosen: {},
      open: false,
      value: { role: "assistant", text: failure, error: true },
    },
  ]);
  try {
    await root.render(<AgentView publication={publication} send={() => {}} />);
    expect(
      root
        .inspect()
        .nodes.some(
          (n) =>
            n.props.role === "alert" &&
            String(n.props.label).startsWith("The request timed out"),
        ),
    ).toBe(true);
    expect(has(root, "Try Again")).toBe(true);
    await root.drive(byLabel(root, "Details").id, "click");
    expect(JSON.stringify(root.inspect().nodes)).toContain(failure);
  } finally {
    root.dispose();
  }
});

test("dragging an attachment keeps the same focused composer editor", async () => {
  const root = createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
  const p = fixture();
  await root.render(<AgentView publication={p} send={() => {}} />);
  const editor = byId(root, "agent-composer-text").id;
  await root.drive(byId(root, "agent-composer").id, "dragover", {});
  expect(byId(root, "agent-composer-text").id).toBe(editor);
  await root.drive(byId(root, "agent-composer").id, "dragleave", {});
  expect(byId(root, "agent-composer-text").id).toBe(editor);
  await root.dispose();
});

for (const method of ["Return", "Send"] as const) {
test(`${method} paints the user bubble and Stop in the first frame`, async () => {
  const { root, commits } = mount();
  const p = fixture();
  p.snapshot.draft.text = "Older draft";
  p.snapshot.draft.sent = 0;
  p.snapshot.sendable = true;
  const actions: Action[] = [];
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => { finish = resolve; });
  const send = (action: Action) => {
    actions.push(action);
    return pending;
  };
  const render = () => root.render(<AgentView publication={p} send={send} />);
  await render();
  const editor = byId(root, "agent-composer-text").id;
  const firstFrame = commits.length;
  const detail = { value: "First message", composerEpoch: 0 };
  if (method === "Return") await root.drive(editor, "keydown", { key: "Enter", ...detail });
  else await root.drive(byId(root, "agent-send").id, "click", detail);
  const nodes = Object.values(replay(commits)[firstFrame].nodes) as any[];
  expect(nodes.filter((n) => n.props.label === "You said: First message")).toHaveLength(1);
  expect(nodes.some((n) => n.props.id === "agent-stop")).toBe(true);
  expect(nodes.some((n) => n.props.id === "agent-send")).toBe(false);
  expect(nodes.some((n) => n.text === "Make your next move")).toBe(false);
  expect(nodes.find((n) => n.props.id === "agent-composer-text")?.props.value).toBe("");
  p.sequence++;
  await render();
  expect(byId(root, "agent-composer-text").id).toBe(editor);
  await root.drive(editor, "keydown", { key: "Enter" });
  expect(actions).toHaveLength(1);
  expect(actions[0]).toMatchObject(method === "Return"
    ? { type: "key", composerEpoch: 0, key: { key: "Enter", value: "First message" } }
    : { type: "submit", composerEpoch: 0, text: "First message" });
  const bubble = byLabel(root, "You said: First message").id;
  p.snapshot.draft.text = "";
  p.snapshot.draft.sent = 1;
  p.snapshot.sendable = false;
  p.snapshot.runId = "run";
  p.snapshot.rows = { count: 1, range: [0, 1], items: [{
    id: "history:message-0", commandId: "history:message-0", chosen: {}, open: false,
    value: { role: "user", text: "First message" },
  }] };
  finish();
  await pending;
  await render();
  expect(byLabel(root, "You said: First message").id).toBe(bubble);
  expect(has(root, "Working for 0s")).toBe(true);
  await root.dispose();
});
}

test("a host refusal keeps the first bubble, restores the draft and offers Retry", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.draft.text = "Keep these words";
  p.snapshot.sendable = true;
  let refuse!: (reason: Error) => void;
  const pending = new Promise<void>((_, reject) => { refuse = reject; });
  const actions: Action[] = [];
  await root.render(<AgentView publication={p} send={(action) => {
    actions.push(action);
    return actions.length === 1 ? pending : undefined;
  }} />);
  await root.drive(byId(root, "agent-send").id, "click");
  refuse(new Error("Provider disconnected"));
  await pending.catch(() => {});
  await root.flush();
  expect(has(root, "You said: Keep these words (Couldn’t send)")).toBe(true);
  expect(byId(root, "agent-composer-text").props.value).toBe("Keep these words");
  await root.drive(byLabel(root, "Retry").id, "click");
  expect(actions).toHaveLength(2);
  await root.dispose();
});

test("a slow start keeps one timer row and can be stopped before host acceptance", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.draft.text = "Start slowly";
  p.snapshot.sendable = true;
  const actions: Action[] = [];
  let accept!: () => void;
  const pending = new Promise<void>((resolve) => { accept = resolve; });
  const render = () => root.render(<AgentView publication={p} send={(action) => {
    actions.push(action);
    return actions.length === 1 ? pending : undefined;
  }} />);
  await render();
  await root.drive(byId(root, "agent-send").id, "click");
  expect(has(root, "Starting…")).toBe(false);
  expect(has(root, "Working for 0s")).toBe(false);
  await root.drive(byId(root, "agent-stop").id, "click");
  expect(actions).toHaveLength(1);
  await new Promise((resolve) => setTimeout(resolve, 330));
  await root.flush();
  expect(has(root, "Starting…")).toBe(true);
  p.snapshot.draft.text = "";
  p.snapshot.sendable = false;
  p.snapshot.runId = "run";
  p.snapshot.starting = true;
  p.snapshot.rows = { count: 1, range: [0, 1], items: [{
    id: "history:message-0", commandId: "history:message-0", chosen: {}, open: false,
    value: { role: "user", text: "Start slowly" },
  }] };
  accept();
  await pending;
  await render();
  expect(actions.at(-1)).toEqual({ type: "stop" });
  const timer = byLabel(root, "Starting…").id;
  p.snapshot.starting = false;
  p.snapshot.elapsedMs = 0;
  await render();
  expect(byLabel(root, "Working for 0s").id).toBe(timer);
  await root.dispose();
});

test("a new user message jumps to the end but streaming updates retain position", async () => {
  const { root } = mount();
  const p = fixture();
  const render = () => root.render(<AgentView publication={p} send={() => {}} />);
  const getToken = () =>
    (byId(root, "agent-conversation").props.metadata as { scrollToken: string }).scrollToken;
  await render();
  const start = getToken();
  p.snapshot.runId = "run";
  p.snapshot.rows = { count: 1, range: [0, 1], items: [
    { id: "user-1", commandId: "", chosen: {}, open: false, value: { role: "user", text: "Direction" } },
  ] };
  await render();
  const sent = getToken();
  expect(sent).not.toBe(start);
  p.snapshot.rows = { count: 2, range: [0, 2], items: [
    ...p.snapshot.rows.items,
    { id: "live-1", commandId: "", chosen: {}, open: false, value: { kind: "text", text: "Working…" } },
  ] };
  await render();
  expect(getToken()).toBe(sent);
  await root.dispose();
});

test("queued directions are marked on the user bubble", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.runId = "run";
  p.snapshot.rows = { count: 1, range: [0, 1], items: [
    { id: "user-1", commandId: "", chosen: {}, open: false, value: { role: "user", text: "Keep the logo.", delivery: "queued" } },
  ] };
  await root.render(<AgentView publication={p} send={() => {}} />);
  expect(root.inspect().nodes.filter((node) => node.text === "Queued for next step")).toHaveLength(1);
  expect(has(root, "You said: Keep the logo. (Queued for next step)")).toBe(true);
  await root.dispose();
});
const harnessCatalog = (models: [string, string][], efforts: string[] = []) => ({
  models,
  providers: [
    ["aero", "Aero"],
    ["claude", "Claude Code"],
    ["chatgpt", "Codex"],
  ] as [string, string][],
  efforts,
});
const textOf = (root: ReturnType<typeof mount>["root"], at: any): string => {
  const nodes = root.inspect().nodes;
  const walk = (n: any): string =>
    (n.text ?? "") + (n.children ?? []).map((id: number) => walk(nodes.find((m) => m.id === id))).join("");
  return walk(at);
};
const segmented = (root: ReturnType<typeof mount>["root"]) =>
  root.inspect().nodes.filter((n) => n.type === "segmented-control" || n.props.role === "radiogroup");
const chooserRows = (r: Parameters<Menu>[0]) =>
  r.options.map((o) => [o.kind, o.label, o.checked ?? null, o.icon ?? null, !!o.disabled, !!o.stays]);
test("with Aero the composer is one row: \"+\", the split pill (the Aero mark and model, then the effort, each its own menu), and Send; no segmented control", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.catalog = harnessCatalog([["pm/openai/gpt-6-sol", "GPT-6 Sol"]], ["low", "high"]);
  p.snapshot.settings.provider = "aero";
  p.snapshot.settings.model = "pm/openai/gpt-6-sol";
  p.snapshot.settings.effort = "high";
  await root.render(<AgentView publication={p} menu={async () => null} send={() => {}} />);
  expect(segmented(root)).toEqual([]);
  const options = byId(root, "agent-options");
  const kids = options.children.map((id: number) => root.inspect().nodes.find((n) => n.id === id)!);
  expect(kids.map((n: any) => n.props.id ?? null)).toEqual(["agent-add", "agent-chooser", "agent-access", null, "agent-send"]);
  expect(byId(root, "agent-chooser").props.metadata).toEqual({ pill: "group" });
  const inside = (id: string) => {
    const part = byId(root, id);
    expect(part.props.metadata).toEqual({ menuAnchor: true, pill: "part" });
    return root
      .inspect()
      .nodes.filter((n) => part.children.includes(n.id))
      .map((n) => [n.props.metadata?.symbol ?? textOf(root, n), n.props.metadata?.pill ?? null, n.props.style?.fontSize ?? null]);
  };
  expect(byId(root, "agent-model").props.label).toBe("Model, GPT-6 Sol");
  expect(byId(root, "agent-effort").props.label).toBe("Reasoning effort, High");
  // At rest `[mark] GPT-6 Sol  High ⌄` in the panels' body size; the
  // model's own chevron shows only with the pill.
  const body = { token: "typography.body" };
  expect(inside("agent-model")).toEqual([
    ["provider.powermove", null, null],
    ["GPT-6 Sol", null, body],
    ["composer.chevron", "reveal", null],
  ]);
  expect(inside("agent-effort")).toEqual([
    ["High", null, body],
    ["composer.chevron", null, null],
  ]);
  // Send is the pill's highlight box: its height and corner.
  const face = root.inspect().nodes.find((n) => n.props.metadata?.symbol === "composer.send")!;
  expect(face.props.style.height).toEqual({ token: "metrics.hit" });
  expect(face.props.style.borderRadius).toEqual({ token: "metrics.inner_radius" });
  // The draft is the panels' body size too.
  expect(byId(root, "agent-composer-text").props.style.fontSize).toEqual(body);
  // A model with no effort levels is the model part alone, its chevron shown.
  p.snapshot.catalog.efforts = [];
  await root.render(<AgentView publication={p} menu={async () => null} send={() => {}} />);
  expect(root.inspect().nodes.some((n) => n.props.id === "agent-effort")).toBe(false);
  expect(inside("agent-model").at(-1)).toEqual(["composer.chevron", null, null]);
  root.dispose();
});
test("with Aero the model part opens the picker: the rail (Aero, Claude Code, Codex), Powermove's catalogue and the usage monitor at its foot; a model closes it", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.catalog = {
    ...harnessCatalog(
      [
        ["pm/openai/gpt-6-sol", "GPT-6 Sol (272K context)"],
        ["pm/anthropic/claude-opus-5.5", "Claude Opus 5.5"],
        ["pm/google/gemini-3-pro", "Gemini 3 Pro"],
      ],
      ["low", "medium", "high"],
    ),
    defaultEffort: "medium",
  };
  p.snapshot.settings.provider = "aero";
  p.snapshot.settings.model = "pm/openai/gpt-6-sol";
  p.snapshot.settings.effort = "high";
  p.native.aero = {
    usage: [
      ["Credits", "812 of 1,000 left"],
      ["Plan", "Aerolite"],
      ["Resets", "Dec 25"],
    ],
  };
  const asked: Parameters<Menu>[0][] = [];
  const picks = ["model:pm/anthropic/claude-opus-5.5", "providers:"];
  const menu: Menu = async (r) => {
    asked.push(r);
    return r.refresh ? null : picks.shift();
  };
  await root.render(<AgentView publication={p} menu={menu} send={(a) => actions.push(a)} />);
  await root.drive(byId(root, "agent-model").id, "click");
  for (const _ of [0, 1, 2, 3]) await Promise.resolve();
  await root.flush();
  const first = asked[0]!;
  expect(first.picker).toBe("chooser");
  expect(first.menu).toBeUndefined();
  expect(first.placeholder).toBe("Search models");
  expect(chooserRows(first)).toEqual([
    ["harness", "Aero", true, "provider.powermove", false, true],
    ["harness", "Claude Code", false, "provider.claude", false, true],
    ["harness", "Codex", false, "provider.chatgpt", false, true],
    ["heading", "Powermove", null, "provider.powermove", false, false],
    ["model", "GPT-6 Sol", true, "provider.chatgpt", false, false],
    ["model", "Claude Opus 5.5", false, "provider.claude", false, false],
    ["model", "Gemini 3 Pro", false, "provider.compatible", false, false],
    ["add", "Providers…", null, null, false, false],
  ]);
  expect(first.options.some((o) => o.label === "Aerolite" || o.label === "Custom")).toBe(false);
  expect(first.usage).toEqual([
    ["Credits", "812 of 1,000 left"],
    ["Plan", "Aerolite"],
    ["Resets", "Dec 25"],
  ]);
  // The model closed the picker; the pill reads it with the kernel's
  // publication, which brings the model's own levels (never the old
  // model's level beside the new model).
  expect(actions).toEqual([{ type: "setModel", model: "pm/anthropic/claude-opus-5.5", effort: "high" }]);
  expect(byId(root, "agent-model").props.open).toBe(false);
  expect(byId(root, "agent-model").props.label).toBe("Model, GPT-6 Sol");
  p.snapshot.settings.model = "pm/anthropic/claude-opus-5.5";
  p.snapshot.settings.effort = "medium";
  p.snapshot.catalog.efforts = ["low", "medium"];
  await root.render(<AgentView publication={structuredClone(p)} menu={menu} send={(a) => actions.push(a)} />);
  expect(byId(root, "agent-model").props.label).toBe("Model, Claude Opus 5.5");
  expect(byId(root, "agent-effort").props.label).toBe("Reasoning effort, Medium");
  // Providers… closes it for Settings › Providers.
  await root.drive(byId(root, "agent-model").id, "click");
  for (const _ of [0, 1, 2, 3]) await Promise.resolve();
  expect(asked.filter((r) => !r.refresh).map((r) => !!r.update)).toEqual([false, false]);
  expect(actions.at(-1)).toEqual({ type: "openSettings" });
  root.dispose();
});
test("the pill never shows a model id: a saved model the catalogue doesn't list reads by its last listed, built-in or readable name", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.settings.provider = "aero";
  p.snapshot.catalog = harnessCatalog([["pm/anthropic/claude-opus-5-9", "Claude Opus 5.9 Preview"]], []);
  p.snapshot.settings.model = "pm/anthropic/claude-opus-5-9";
  await root.render(<AgentView publication={structuredClone(p)} menu={async () => null} send={() => {}} />);
  expect(byId(root, "agent-model").props.label).toBe("Model, Claude Opus 5.9 Preview");
  // The catalogue is gone (signed out, a 401): the names stay.
  for (const [model, name] of [
    ["pm/anthropic/claude-opus-5-9", "Claude Opus 5.9 Preview"],
    ["pm/anthropic/claude-opus-5-5", "Claude Opus 5.5"],
    ["pm/openai/gpt-6.2-sol", "GPT-6.2 Sol"],
    ["pm/mistral/le-chat-2-1", "Le Chat 2.1"],
  ]) {
    p.snapshot.catalog = harnessCatalog([], []);
    p.snapshot.settings.model = model!;
    await root.render(<AgentView publication={structuredClone(p)} menu={async () => null} send={() => {}} />);
    expect(byId(root, "agent-model").props.label).toBe(`Model, ${name}`);
  }
  root.dispose();
});
test("the effort part opens the reasoning menu: the model's levels in order, ✓ on the one in use; a level closes it and the pill reads it at once, never the old one", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.catalog = {
    ...harnessCatalog([["gpt-6-sol", "GPT-6 Sol"]], ["low", "medium", "high", "xhigh", "max"]),
    defaultEffort: "medium",
  };
  p.snapshot.settings.provider = "chatgpt";
  p.snapshot.settings.model = "gpt-6-sol";
  p.snapshot.settings.effort = "high";
  const asked: Parameters<Menu>[0][] = [];
  let answer: (value: unknown) => void = () => {};
  const menu: Menu = (r) => {
    asked.push(r);
    return new Promise((resolve) => {
      answer = resolve;
    });
  };
  const render = () =>
    root.render(<AgentView publication={structuredClone(p)} menu={menu} send={(a) => actions.push(a)} />);
  await render();
  const effort = byId(root, "agent-effort");
  await root.drive(effort.id, "click");
  const request = asked[0]!;
  expect(request).toMatchObject({ anchor: effort.id, picker: "chooser", menu: "effort" });
  expect(request.options.map((o) => [o.kind, o.label, o.value, !!o.checked, !!o.stays])).toEqual([
    ["effort", "Low", "low", false, false],
    ["effort", "Medium", "medium", false, false],
    ["effort", "High", "high", true, false],
    ["effort", "Extra high", "xhigh", false, false],
    ["effort", "Max", "max", false, false],
  ]);
  expect(byId(root, "agent-effort").props.open).toBe(true);
  answer("low");
  for (const _ of [0, 1, 2, 3]) await Promise.resolve();
  await root.flush();
  expect(actions).toEqual([{ type: "setModel", model: "gpt-6-sol", effort: "low" }]);
  expect(byId(root, "agent-effort").props.open).toBe(false);
  expect(byId(root, "agent-effort").props.label).toBe("Reasoning effort, Low");
  // A publication from before the kernel took it still reads Low (the old
  // level never flickers back); the kernel's answer settles it.
  await render();
  expect(byId(root, "agent-effort").props.label).toBe("Reasoning effort, Low");
  p.snapshot.settings.effort = "low";
  await render();
  expect(byId(root, "agent-effort").props.label).toBe("Reasoning effort, Low");
  // A level the model does not offer reads as the model's default, and
  // with no default listed, as its first level: always one it can run.
  p.snapshot.settings.effort = "minimal";
  await render();
  expect(byId(root, "agent-effort").props.label).toBe("Reasoning effort, Medium");
  p.snapshot.catalog.defaultEffort = undefined;
  await render();
  expect(byId(root, "agent-effort").props.label).toBe("Reasoning effort, Low");
  root.dispose();
});
test("Aero's card shows no usage monitor until the backend has answered", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.catalog = harnessCatalog([["pm/openai/gpt-6-sol", "GPT-6 Sol"]]);
  p.snapshot.settings.provider = "aero";
  p.snapshot.settings.model = "pm/openai/gpt-6-sol";
  const asked: Parameters<Menu>[0][] = [];
  await root.render(
    <AgentView
      publication={p}
      menu={async (r) => {
        asked.push(r);
        return null;
      }}
      send={() => {}}
    />,
  );
  await root.drive(byId(root, "agent-model").id, "click");
  expect(asked[0]!.usage).toBeUndefined();
  root.dispose();
});
test("Aero out of credits says until when in the chooser, and offers the bigger plan on Aerolite", async () => {
  for (const [upgrade, rows] of [
    [
      "Aero",
      [
        ["status", "Out of credits until Dec 25."],
        ["action", "Upgrade to Aero…"],
      ],
    ],
    [null, [["status", "Out of credits until Dec 25."]]],
  ] as const) {
    const { root } = mount();
    const p = fixture();
    p.snapshot.catalog = harnessCatalog([["pm/openai/gpt-6-sol", "GPT-6 Sol"]], ["low"]);
    p.snapshot.settings.provider = "aero";
    p.snapshot.settings.model = "pm/openai/gpt-6-sol";
    p.native.aero = {
      usage: [["Credits", "0 of 1,000 left"]],
      outOfCredits: { resets: "Dec 25", upgrade },
    };
    const asked: Parameters<Menu>[0][] = [];
    await root.render(
      <AgentView
        publication={p}
        menu={async (r) => {
          asked.push(r);
          return null;
        }}
        send={() => {}}
      />,
    );
    await root.drive(byId(root, "agent-model").id, "click");
    expect(
      asked[0]!.options.filter((o) => o.kind === "status" || o.kind === "action").map((o) => [o.kind, o.label]),
    ).toEqual(rows);
    root.dispose();
  }
});
test("Aero on Free says it needs a plan, quietly, with Subscribe… to Settings › Account", async () => {
  for (const [needsPlan, rows] of [
    [
      true,
      [
        ["status", "Subscribe to Aerolite."],
        ["action", "Subscribe…"],
      ],
    ],
  ] as const) {
    const { root } = mount();
    const p = fixture();
    p.snapshot.catalog = harnessCatalog([]);
    p.snapshot.settings.provider = "aero";
    p.native.connection = "unavailable";
    p.native.reason = "Aero needs Aerolite.";
    p.native.aero = { usage: [], needsPlan };
    const asked: Parameters<Menu>[0][] = [];
    await root.render(
      <AgentView
        publication={p}
        menu={async (r) => {
          asked.push(r);
          return null;
        }}
        send={() => {}}
      />,
    );
    // Quiet: no fault notice in the panel.
    expect(root.inspect().nodes.some((node) => node.props.id === "agent-connect-error")).toBe(false);
    await root.drive(byId(root, "agent-model").id, "click");
    expect(asked[0]!.options.filter((o) => o.kind !== "harness").map((o) => [o.kind, o.label])).toEqual(rows);
    root.dispose();
  }
});
test("Aero on Free offers Subscribe beside the model pill and in the empty panel, straight to Checkout", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.catalog = harnessCatalog([]);
  p.snapshot.settings.provider = "aero";
  p.native.connection = "unavailable";
  p.native.reason = "Aero needs Aerolite.";
  p.native.aero = { usage: [], needsPlan: true };
  const sent: Action[] = [];
  await root.render(<AgentView publication={p} send={(a) => sent.push(a)} />);
  await root.drive(byId(root, "agent-subscribe").id, "click");
  await root.drive(byId(root, "agent-welcome-subscribe").id, "click");
  expect(sent.filter((a) => a.type === "subscribe").length).toBe(2);
  root.dispose();
  // On a plan, neither shows.
  const { root: paid } = mount();
  const q = fixture();
  q.snapshot.settings.provider = "aero";
  q.native.aero = { usage: [], needsPlan: false };
  await paid.render(<AgentView publication={q} send={() => {}} />);
  expect(paid.inspect().nodes.some((node) => node.props.id === "agent-subscribe")).toBe(false);
  paid.dispose();
});
test("Aero shows a quiet low-credit notice at 80% (warning) and 95% (critical), hidden when out of credits or on other harnesses", async () => {
  for (const [level, remaining, percentUsed, expected] of [
    ["warning", 180, 82, "Low on credits: 180 of 1,000 left · resets Dec 25"],
    ["critical", 40, 96, "Almost out of credits: 40 of 1,000 left · resets Dec 25"],
  ] as const) {
    const { root } = mount();
    const p = fixture();
    p.snapshot.catalog = harnessCatalog([["pm/openai/gpt-6.2-sol", "GPT-6.2 Sol"]], ["medium"]);
    p.snapshot.settings.provider = "aero";
    p.snapshot.settings.model = "pm/openai/gpt-6.2-sol";
    p.native.aero = {
      usage: [["Credits", `${remaining} of 1,000 left`]],
      lowCredits: { remaining, included: 1000, percentUsed, level, resets: "Dec 25" },
    };
    await root.render(<AgentView publication={p} send={() => {}} />);
    const notice = byId(root, "agent-low-credits");
    expect(notice.props.label).toBe(expected);
    root.dispose();
  }
  // Hidden when out of credits (outOfCredits takes over) or on another harness.
  for (const [provider, outOfCredits] of [
    ["aero", { resets: "Dec 25", upgrade: null }],
    ["claude", null],
  ] as const) {
    const { root } = mount();
    const p = fixture();
    p.snapshot.catalog = harnessCatalog([["pm/openai/gpt-6.2-sol", "GPT-6.2 Sol"]], ["medium"]);
    p.snapshot.settings.provider = provider;
    p.native.aero = {
      usage: [["Credits", "40 of 1,000 left"]],
      lowCredits: { remaining: 40, included: 1000, percentUsed: 96, level: "critical", resets: "Dec 25" },
      outOfCredits,
    };
    await root.render(<AgentView publication={p} send={() => {}} />);
    expect(root.inspect().nodes.some((node) => node.props.id === "agent-low-credits")).toBe(false);
    root.dispose();
  }
});
test("the pill shows Claude Code's logo, OpenAI's for Codex, and the Aero (Powermove) mark for every Aero model", async () => {
  for (const [harness, model, mark] of [
    ["claude", "claude-opus-5-5", "provider.claude"],
    ["aero", "pm/anthropic/claude-opus-5.5", "provider.powermove"],
    ["aero", "pm/openai/gpt-6-sol", "provider.powermove"],
    ["chatgpt", "gpt-6-sol", "provider.chatgpt"],
  ] as const) {
    const { root } = mount();
    const p = fixture();
    p.snapshot.catalog = harnessCatalog([[model, "Model"]]);
    p.snapshot.settings.provider = harness;
    p.snapshot.settings.model = model;
    await root.render(<AgentView publication={p} menu={async () => null} send={() => {}} />);
    const pill = byId(root, "agent-model");
    const symbols = root.inspect().nodes.filter((n) => pill.children.includes(n.id)).map((n) => n.props.metadata?.symbol);
    expect(symbols[0]).toBe(mark);
    root.dispose();
  }
});
test("with the Codex and Claude Code harnesses the chooser heads their models with the account they run on, without Aerolite", async () => {
  for (const [harness, accounts, heading] of [
    ["chatgpt", undefined, "ChatGPT"],
    ["chatgpt", { chatgpt: "OpenAI API key" }, "OpenAI API key"],
    ["claude", undefined, "Claude"],
  ] as const) {
    const { root } = mount();
    const p = fixture();
    p.snapshot.catalog = harnessCatalog([["m-1", "Model One"], ["m-2", "Model Two"]]);
    p.snapshot.settings.provider = harness;
    p.snapshot.settings.model = "m-2";
    if (accounts) p.native.accounts = accounts;
    const asked: Parameters<Menu>[0][] = [];
    await root.render(
      <AgentView
        publication={p}
        menu={async (r) => {
          asked.push(r);
          return null;
        }}
        send={() => {}}
      />,
    );
    await root.drive(byId(root, "agent-model").id, "click");
    expect(chooserRows(asked[0]!).filter(([kind]) => kind !== "harness" && kind !== "add")).toEqual([
      ["heading", heading, null, harness === "claude" ? "provider.claude" : "provider.chatgpt", false, false],
      ["model", "Model One", false, `provider.${harness === "claude" ? "claude" : "chatgpt"}`, false, false],
      ["model", "Model Two", true, `provider.${harness === "claude" ? "claude" : "chatgpt"}`, false, false],
    ]);
    root.dispose();
  }
});
test("a harness with no models offers one action and no card; one that couldn't start offers Try Again", async () => {
  for (const [connection, rows, action] of [
    ["connected", [["status", "Sign in to Claude in Settings › Providers"], ["action", "Connect a provider…"]], "providers:"],
    ["unavailable", [["status", "Claude Code couldn’t start."], ["action", "Try Again"]], "retry:"],
  ] as const) {
    const { root } = mount();
    const actions: Action[] = [];
    const p = fixture();
    p.snapshot.catalog = harnessCatalog([]);
    p.snapshot.settings.provider = "claude";
    p.snapshot.settings.model = "";
    p.native.connection = connection;
    const asked: Parameters<Menu>[0][] = [];
    await root.render(
      <AgentView
        publication={p}
        menu={async (r) => {
          asked.push(r);
          return action;
        }}
        send={(a) => actions.push(a)}
      />,
    );
    await root.drive(byId(root, "agent-model").id, "click");
    await Promise.resolve();
    expect(chooserRows(asked[0]!).filter(([kind]) => kind !== "harness").map(([kind, label]) => [kind, label])).toEqual(
      rows.map((r) => [...r]),
    );
    expect(asked[0]!.usage).toBeUndefined();
    expect(actions).toEqual([{ type: connection === "connected" ? "openSettings" : "retryConnection" }]);
    root.dispose();
  }
});
test("two context variants of one model never read alike, in the picker or the pill", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.catalog = harnessCatalog([
    ["opus", "Opus 5.5"],
    ["opus[1m]", "Opus 5.5 (1M context)"],
    ["sonnet", "Sonnet 5.5 (200K context)"],
  ]);
  p.snapshot.settings.provider = "claude";
  p.snapshot.settings.model = "opus[1m]";
  const asked: Parameters<Menu>[0][] = [];
  await root.render(
    <AgentView
      publication={p}
      menu={async (r) => {
        asked.push(r);
        return null;
      }}
      send={() => {}}
    />,
  );
  expect(byId(root, "agent-model").props.label).toBe("Model, Opus 5.5 (1M)");
  await root.drive(byId(root, "agent-model").id, "click");
  const names = asked[0]!.options.filter((o) => o.kind === "model").map((o) => o.label);
  expect(names).toEqual(["Opus 5.5", "Opus 5.5 (1M)", "Sonnet 5.5"]);
  expect(new Set(names).size).toBe(names.length);
  root.dispose();
});
test("Stop is the same rounded rect as Send: the pill's highlight box (no circles)", async () => {
  const { root } = mount();
  const p = fixture();
  p.snapshot.catalog = harnessCatalog([["pm/openai/gpt-6-sol", "GPT-6 Sol"]]);
  p.snapshot.settings.provider = "aero";
  p.snapshot.runId = "run-1";
  await root.render(<AgentView publication={p} menu={async () => null} send={() => {}} />);
  const stop = root.inspect().nodes.find((n) => n.props.metadata?.symbol === "composer.stop")!;
  expect(stop.props.style.width).toEqual({ token: "metrics.hit" });
  expect(stop.props.style.height).toEqual({ token: "metrics.hit" });
  expect(stop.props.style.borderRadius).toEqual({ token: "metrics.inner_radius" });
  root.dispose();
});
test("with Aero a rail harness keeps the chooser open and switches the harness", async () => {
  const { root } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.catalog = harnessCatalog([["pm/openai/gpt-6-sol", "GPT-6 Sol"]]);
  p.snapshot.settings.provider = "aero";
  p.snapshot.settings.model = "";
  let answered = false;
  const menu: Menu = async () => {
    if (answered) return null;
    answered = true;
    return "provider:claude";
  };
  await root.render(<AgentView publication={p} menu={menu} send={(a) => actions.push(a)} />);
  // No model chosen: the pill shows the one Aero runs.
  expect(byId(root, "agent-model").props.label).toBe("Model, GPT-6 Sol");
  await root.drive(byId(root, "agent-model").id, "click");
  await Promise.resolve();
  expect(actions).toEqual([{ type: "setProvider", provider: "claude" }]);
  root.dispose();
});
test("signed out: the pill reads Sign in to Powermove (Aero, with its mark) or Connect a provider, and opens the rail and one compact card; no line under the composer", async () => {
  for (const [harness, line, pillText, mark, action] of [
    ["aero", "Sign in to Powermove in Settings › Providers", "Sign in to Powermove", "provider.powermove", "Sign In to Powermove…"],
    ["chatgpt", "Sign in to ChatGPT in Settings › Providers", "Connect a provider", null, "Connect a provider…"],
    ["claude", "Sign in to Claude in Settings › Providers", "Connect a provider", null, "Connect a provider…"],
  ] as const) {
    const { root } = mount();
    const actions: Action[] = [];
    const p = fixture();
    p.snapshot.catalog = harnessCatalog([["gpt-6-sol", "GPT-6 Sol"]], ["low", "high"]);
    p.snapshot.settings.provider = harness;
    p.native.connection = "unavailable";
    p.native.signedOut = true;
    const asked: Parameters<Menu>[0][] = [];
    const picks = ["providers:"];
    await root.render(
      <AgentView
        publication={p}
        menu={async (r) => {
          asked.push(r);
          return picks.shift();
        }}
        send={(a) => actions.push(a)}
      />,
    );
    expect(segmented(root)).toEqual([]);
    expect(root.inspect().nodes.some((n) => n.props.id === "agent-connect")).toBe(false);
    expect(root.inspect().nodes.some((n) => n.props.id === "agent-sign-in")).toBe(false);
    expect(byId(root, "agent-welcome")).toBeDefined();
    const pill = byId(root, "agent-model");
    expect(pill.props.label).toBe(pillText);
    expect(pill.props.disabled).toBeFalsy();
    const inside = root.inspect().nodes.filter((n) => pill.children.includes(n.id));
    expect(inside.map((n) => n.props.metadata?.symbol ?? textOf(root, n))).toEqual(
      [mark, pillText, "composer.chevron"].filter((x) => x !== null),
    );
    expect(byId(root, "agent-composer-text").props.disabled).toBeFalsy();
    expect(byId(root, "agent-send").props.disabled).toBe(true);
    await root.drive(pill.id, "click");
    await Promise.resolve();
    expect(asked[0]!.picker).toBe("chooser");
    expect(chooserRows(asked[0]!).map(([kind, label, checked]) => [kind, label, checked])).toEqual([
      ["harness", "Aero", harness === "aero"],
      ["harness", "Claude Code", harness === "claude"],
      ["harness", "Codex", harness === "chatgpt"],
      ["status", line, null],
      ["action", action, null],
    ]);
    expect(asked[0]!.usage).toBeUndefined();
    expect(actions).toEqual([{ type: "openSettings" }]);
    root.dispose();
  }
});

test("focused attachment tiles open on Space and Return and opt into focusOnClick", async () => {
  const { root, commits } = mount();
  const p = fixture();
  const actions: Action[] = [];
  p.snapshot.rows.count = 1;
  p.snapshot.rows.range = [0, 1];
  p.snapshot.rows.items = [
    {
      id: "history:sent-1",
      commandId: "history:sent-1",
      value: {
        role: "user",
        text: "Recreate this",
        attachments: [
          { id: "img-a", name: "Screen.png", type: "image/png" },
          { id: "img-b", name: "Wallpaper.png", type: "image/png" },
        ],
      },
      chosen: {},
      open: false,
    },
  ];
  await root.render(
    <AgentView publication={p} send={(a) => actions.push(a)} />,
  );
  const first = byLabel(root, "Open Screen.png");
  expect((first.props.metadata as any)?.focusOnClick).toBe(true);
  await root.drive(first.id, "keydown", { key: " " });
  await root.drive(first.id, "keydown", { key: "Enter" });
  expect(actions).toEqual([
    { type: "openAttachment", id: "img-a" },
    { type: "openAttachment", id: "img-a" },
  ]);
  assertWire(commits);
  root.dispose();
});

test("scrolling to the top or bottom of a windowed thread requests earlier or later rows", async () => {
  const { root, commits } = mount();
  const p = fixture();
  const actions: Action[] = [];
  p.snapshot.rows.count = 400;
  p.snapshot.rows.range = [144, 400];
  p.snapshot.rows.items = [
    {
      id: "history:m144",
      commandId: "history:m144",
      value: { role: "user", text: "Middle" },
      chosen: {},
      open: false,
    },
  ];
  await root.render(
    <AgentView publication={p} send={(a) => actions.push(a)} />,
  );
  const convo = byId(root, "agent-conversation");
  await root.drive(convo.id, "scroll", { value: { away: true, top: true } });
  expect(actions).toEqual([{ type: "loadRows", direction: "earlier" }]);
  actions.length = 0;
  p.snapshot.rows.range = [0, 256];
  await root.render(
    <AgentView publication={p} send={(a) => actions.push(a)} />,
  );
  await root.drive(convo.id, "scroll", { value: { away: false, bottom: true } });
  expect(actions).toEqual([{ type: "loadRows", direction: "later" }]);
  assertWire(commits);
  root.dispose();
});
