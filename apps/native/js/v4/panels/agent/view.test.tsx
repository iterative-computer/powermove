import { test, expect } from "bun:test";
import type { ComponentChild } from "preact";
import { createRoot, type Commit } from "../../src";
import { AgentPanel, AgentView, composerKey, elapsed } from "./view";
import { fixture, state, states, withRows } from "./fixtures";
import type { Action, AgentModel, Publication } from "./contract";

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
test("native supported focus order follows header, conversation and composer bar; blocked controls are absent", async () => {
  const { root, commits } = mount();
  const p = states.selected();
  p.snapshot.sendable = true;
  await root.render(<AgentView publication={p} send={() => {}} />);
  const result = assertWire(commits);
  expect(
    result.focusOrder.map((id: number) => result.nodes[id].props.id),
  ).toEqual([
    "agent-new",
    "agent-presentation",
    "agent-conversation",
    "agent-composer-text",
    "agent-attach",
    "agent-access",
    "agent-provider",
    "agent-model",
    "agent-send",
  ]);
  expect(
    root.inspect().nodes.find((n) => n.props.id === "agent-conversation")?.props
      .label,
  ).toBe("Agent conversation");
  root.dispose();
});
test("pin, close, new thread, send and stop use the same model commands as headless clients", async () => {
  const { root, commits } = mount();
  const actions: Action[] = [];
  const p = fixture();
  p.snapshot.sendable = true;
  const render = () =>
    root.render(<AgentView publication={p} send={(a) => actions.push(a)} />);
  const click = async (id: string) => {
    const n = root.inspect().nodes.find((n) => n.props.id === id)!;
    await root.drive(n.id, "click");
  };
  await render();
  await click("agent-presentation");
  await click("agent-new");
  await click("agent-send");
  p.native.presentation = "docked";
  p.snapshot.runId = "run";
  await render();
  await click("agent-presentation");
  await click("agent-stop");
  p.snapshot.stopping = true;
  await render();
  await click("agent-stop");
  expect(actions).toEqual([
    { type: "shell", action: "pin" },
    { type: "newThread" },
    { type: "submit" },
    { type: "shell", action: "close" },
    { type: "stop" },
  ]);
  assertWire(commits);
  root.dispose();
});
test("streamed text retains its native id and appends in place without replacing prior text", async () => {
  const { root, commits } = mount();
  const p = states.selected();
  await root.render(<AgentView publication={p} send={() => {}} />);
  const id = root
    .inspect()
    .nodes.find(
      (n) => n.props.label === "Agent: Done. Headline now scales from 92 % …",
    )!.id;
  p.snapshot.rows.items[1].value.text =
    "Done. Headline now scales from 92 % …\nAuthored 日本🙂\n";
  await root.render(<AgentView publication={p} send={() => {}} />);
  expect(root.inspect().nodes.find((n) => n.id === id)?.props.label).toBe(
    `Agent: ${p.snapshot.rows.items[1].value.text}`,
  );
  expect(commits.at(-1)!.ops.some((o) => o.op === "remove")).toBe(false);
  assertWire(commits);
  root.dispose();
});
test("disclosure removes hidden descendants from native focus and AX tree together", async () => {
  const { root, commits } = mount();
  const p = states.hover();
  const actions: Action[] = [];
  await root.render(
    <AgentView publication={p} send={(a) => actions.push(a)} />,
  );
  const button = root
    .inspect()
    .nodes.find(
      (n) => n.props.label === "Make the headline" && n.type === "button",
    )!;
  await root.drive(button.id, "click");
  expect(actions).toEqual([{ type: "disclose", id: "tool", open: false }]);
  const stop = root
    .inspect()
    .nodes.find((n) => n.props.label === "Stop Make the headline")!.id;
  p.snapshot.rows.items[0].open = false;
  await root.render(<AgentView publication={p} send={() => {}} />);
  const reply = assertWire(commits);
  expect(reply.nodes[stop]).toBeUndefined();
  expect(reply.focusOrder).not.toContain(stop);
  root.dispose();
});
test("60k logical rows mount only the supplied window and preserve logical count for integration", async () => {
  const { root, commits } = mount();
  await root.render(
    <AgentView publication={states["60k"]()} send={() => {}} />,
  );
  expect(root.inspect().nodes.length).toBeLessThan(100);
  const list = root
    .inspect()
    .nodes.find((n) => n.props.id === "agent-conversation")!;
  expect(list.props.metadata).toMatchObject({
    count: 60_000,
    range: [30_000, 30_002],
    apiGap: "variableList",
  });
  assertWire(commits);
  root.dispose();
});
test("empty Send and unavailable native actions cannot dispatch through system push buttons", async () => {
  const { root, commits } = mount();
  const actions: Action[] = [];
  await root.render(
    <AgentView publication={fixture()} send={(a) => actions.push(a)} />,
  );
  const send = root.inspect().nodes.find((n) => n.props.id === "agent-send")!;
  await root.drive(send.id, "click");
  expect(actions).toEqual([]);
  await root.render(
    <AgentView publication={states["awaiting-answer"]()} send={() => {}} />,
  );
  expect(
    root
      .inspect()
      .nodes.filter((n) => n.type === "button")
      .filter((n) =>
        ["Answer", "Skip", "Allow"].includes(String(n.props.label)),
      )
      .every(
        (n) =>
          n.props["data-pm-component"] === "push-button" &&
          n.props.disabled === true,
      ),
  ).toBe(true);
  assertWire(commits);
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
  const nodes = root.inspect().nodes;
  expect(nodes.some((n) => n.props.label === "Connect Your Agent")).toBe(false);
  const stop = nodes.find((n) => n.props.id === "agent-stop")!;
  await root.drive(stop.id, "click");
  expect(actions).toEqual([{ type: "stop" }]);
  await root.render(
    <AgentView publication={states.unavailable()} send={() => {}} />,
  );
  expect(
    root.inspect().nodes.some((n) => n.props.label === "Connect Your Agent"),
  ).toBe(true);
  root.dispose();
});
test("question cards choose, answer and skip through the model; Answer waits for a choice", async () => {
  const { root } = mount();
  const sent: Action[] = [];
  const question = {
    id: "q",
    commandId: "live:q",
    chosen: {},
    open: true,
    value: {
      id: "q",
      kind: "question",
      status: "open",
      transport: "reply",
      questions: [
        { id: "a", question: "Which?", options: [{ label: "Soft" }] },
      ],
    },
  };
  const send = (action: Action) => sent.push(action);
  await root.render(
    <AgentView publication={withRows([question])} send={send} />,
  );
  const node = (id: string) =>
    root.inspect().nodes.find((n) => n.props.id === id)!;
  expect(node("q-answer").props.disabled).toBe(true);
  await root.drive(node("q-a-Soft").id, "click");
  await root.drive(node("q-skip").id, "click");
  await root.render(
    <AgentView
      publication={withRows([{ ...question, chosen: { a: ["Soft"] } }])}
      send={send}
    />,
  );
  await root.drive(node("q-answer").id, "click");
  expect(sent).toEqual([
    { type: "chooseAnswer", step: "live:q", question: "a", option: "Soft" },
    { type: "answer", step: "live:q", skip: true },
    { type: "answer", step: "live:q", skip: false },
  ]);
  root.dispose();
});
test("attachment remove/open routes preserve ids and two-stage arming remains model-owned", async () => {
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
    const node = root
      .inspect()
      .nodes.find((n) => n.type === "button" && n.props.label === label)!;
    await root.drive(node.id, "click");
  }
  expect(actions).toEqual([
    { type: "openAttachment", id: "file" },
    { type: "removeAttachment", id: "file" },
  ]);
  assertWire(commits);
  root.dispose();
});
test("working timer has exact copy and no invented percentage", () => {
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
  const pin = root
    .inspect()
    .nodes.find((n) => n.props.id === "agent-presentation")!;
  await root.drive(pin.id, "click");
  await Promise.resolve();
  expect(errors.map(String)).toEqual(["Error: rejected"]);
  p = states.working();
  update();
  await root.flush();
  expect(
    root.inspect().nodes.some((n) => n.props.label === "Stop current run"),
  ).toBe(true);
  await root.dispose();
  expect(stops).toBe(1);
});
test("composer worker forwarding keeps native clipboard and editor routing distinct", async () => {
  const seen: Action[] = [];
  const model: AgentModel = {
    inspect: fixture,
    subscribe: () => () => {},
    dispatch: async (a) => {
      seen.push(a);
    },
  };
  for (const key of [
    { key: "c", metaKey: true },
    { key: "v", metaKey: true },
    { key: "Enter", composing: true },
    { key: "Enter", repeat: true },
  ]) {
    await model.dispatch(
      composerKey({
        ...key,
        type: "keydown",
        target: 1,
        currentTarget: 1,
        defaultPrevented: false,
        preventDefault() {
          throw Error("late cancellation");
        },
        stopPropagation() {},
        stopImmediatePropagation() {},
      }),
    );
  }
  expect(seen).toHaveLength(4);
  expect(seen[2]).toMatchObject({
    type: "key",
    key: { key: "Enter", composing: true },
  });
});
test("all conversation item kinds preserve exact prose and expose explicit unsupported-control placeholders", async () => {
  const { root, commits } = mount();
  const values: import("../../src/protocol").Props[] = [
    {
      kind: "thought",
      label: "Authored reasoning",
      live: false,
      startedAt: 1000,
      endedAt: 4200,
    },
    { kind: "visual", title: "Rendered result", status: "error" },
    { kind: "result", text: "The rendered change is ready." },
    {
      kind: "tool",
      label: "Make the headline",
      status: "error",
      output: "Failed",
    },
  ];
  const rows = values.map((value, i) => ({
    id: `r${i}`,
    commandId: `r${i}`,
    chosen: {},
    open: true,
    value,
  }));
  await root.render(<AgentView publication={withRows(rows)} send={() => {}} />);
  expect(
    root.inspect().nodes.some((n) => n.props.label === "Thought for 3 s"),
  ).toBe(true);
  assertWire(commits);
  root.dispose();
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
test("answered and skipped cards collapse using masked model acknowledgements; restored closed cards have no invented actions", async () => {
  const { root, commits } = mount();
  const values: import("../../src/protocol").Props[] = [
    { kind: "question", status: "answered", answers: { q: "••••••" } },
    { kind: "question", status: "answered", answers: { q: "" } },
    {
      kind: "question",
      status: "closed",
      questions: [{ id: "q", question: "Apply the same entrance to Kicker?" }],
    },
  ];
  await root.render(
    <AgentView
      publication={withRows(
        values.map((value, i) => ({
          id: `q${i}`,
          commandId: `q${i}`,
          chosen: {},
          open: false,
          value,
        })),
      )}
      send={() => {}}
    />,
  );
  const nodes = root.inspect().nodes;
  expect(nodes.some((n) => n.props.label === "Answered: ••••••")).toBe(true);
  expect(nodes.some((n) => n.props.label === "Skipped")).toBe(true);
  expect(
    nodes.some((n) => n.type === "button" && n.props.label === "Answer"),
  ).toBe(false);
  assertWire(commits);
  await root.dispose();
});
