import { describe, test, expect } from "bun:test";
import { h, createRoot, type Commit } from "../src";
import { useState } from "../src/hooks";
import { recorder } from "./helpers";
import * as Refresh from "../src/refresh";

describe("Preact native host", () => {
  test("JSX, every primitive, structured props, false, empty strings and null patches", async () => {
    const { root, commits } = recorder();
    await root.render(
      <div
        id=""
        attr={{ exact: [1, true] }}
        style={{ display: "grid", gap: { token: "metrics.gap" } }}
      >
        <input value="" disabled={false} />
        <button>OK</button>
      </div>,
    );
    expect(root.host.root.textContent).toBe("OK");
    const props = root.host.root.children[0].props;
    expect(props.id).toBe("");
    expect(props.style).toEqual({
      display: "grid",
      gap: { token: "metrics.gap" },
    });
    expect(root.host.root.children[0].children[0].props.disabled).toBe(false);
    await root.render(
      <div>
        <input value="" />
        <button>OK</button>
      </div>,
    );
    expect(
      commits[1].ops.some((o) => o.op === "setProps" && o.patch.id === null),
    ).toBe(true);
  });
  test("keyed reorder retains native IDs and hooks", async () => {
    const { root, commits } = recorder();
    let update!: (v: number) => void;
    function Counter() {
      const [n, set] = useState(1);
      update = set;
      return <text>{n}</text>;
    }
    const tree = (reverse: boolean) => (
      <div>
        <Counter />
        {(reverse ? ["b", "a"] : ["a", "b"]).map((key) => (
          <button key={key} id={key}>
            {key}
          </button>
        ))}
      </div>
    );
    await root.render(tree(false));
    update(17);
    await root.flush();
    const ids = root.host.root.children[0].children.slice(1).map((n) => n.id);
    await root.render(tree(true));
    expect(
      root.host.root.children[0].children.slice(1).map((n) => n.id),
    ).toEqual([...ids].reverse());
    expect(root.host.root.textContent).toBe("17ba");
    expect(commits.at(-1)!.ops.some((o) => o.op === "insertBefore")).toBe(true);
    expect(commits.at(-1)!.ops.some((o) => o.op === "create")).toBe(false);
  });
  test("all six ops, listener removal and subtree removal", async () => {
    const { root, commits } = recorder();
    await root.render(<button onClick={() => {}}>one</button>);
    await root.render(<div>two</div>);
    await root.render(<div id="updated">three</div>);
    await root.dispose();
    expect(new Set(commits.flatMap((c) => c.ops.map((o) => o.op)))).toEqual(
      new Set([
        "create",
        "insertBefore",
        "remove",
        "setProps",
        "setText",
        "setListeners",
      ]),
    );
    expect(root.host.nodes.size).toBe(1);
  });
  test("rejected commit keeps accepted inspector snapshot and fences subsequent work", async () => {
    let reject = false;
    const root = createRoot(async (c) => ({
      accepted: !reject,
      revision: c.baseRevision + (reject ? 0 : 1),
    }));
    await root.render(<text>accepted</text>);
    const before = root.inspect();
    reject = true;
    await expect(root.render(<text>rejected</text>)).rejects.toThrow(
      "rejected",
    );
    expect(root.inspect().nodes).toEqual(before.nodes);
    expect(root.inspect().revision).toBe(before.revision);
    expect(() => root.render(<text />)).toThrow("fenced");
  });
  test("async commits serialize revision and transport errors fence", async () => {
    const revisions: number[] = [];
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const root = createRoot(async (c) => {
      revisions.push(c.baseRevision);
      if (c.baseRevision === 0) await gate;
      return { accepted: true, revision: c.baseRevision + 1 };
    });
    const first = root.render(<text>a</text>),
      second = root.render(<text>b</text>);
    release();
    await Promise.all([first, second]);
    expect(revisions).toEqual([0, 1]);
    const failed = createRoot(async () => {
      throw Error("transport");
    });
    await expect(failed.render(<text />)).rejects.toThrow("transport");
  });
  test("capture/bubble, propagation, predeclared defaults and stale events", async () => {
    const { root, commits } = recorder();
    const order: string[] = [];
    await root.render(
      <div
        onClickCapture={() => order.push("capture")}
        onClick={() => order.push("bubble")}
      >
        <button
          preventDefault={{ click: true }}
          onClick={(e) => {
            order.push("target");
            e.stopPropagation();
          }}
        >
          run
        </button>
      </div>,
    );
    const parent = root.host.root.children[0],
      button = parent.children[0];
    const flags = commits[0].ops
      .filter((o) => o.op === "setListeners" && o.id === button.id)
      .at(-1);
    expect(
      flags?.op === "setListeners" && flags.flags.click.preventDefault,
    ).toBe(true);
    const plan = {
      revision: 1,
      target: button.id,
      preventDefault: true,
      deliveries: [
        { id: parent.id, capture: true },
        { id: button.id, capture: false },
        { id: parent.id, capture: false },
      ],
    };
    await root.dispatch(plan, "click");
    expect(order).toEqual(["capture", "target"]);
    await expect(
      root.dispatch({ ...plan, revision: 0 }, "click"),
    ).rejects.toThrow("Stale");
  });
  test("an event planned before a commit lands only if its target's subtree is unchanged", async () => {
    const { root } = recorder();
    let clicks = 0;
    const view = (rows: string[], label: string) => (
      <div>
        <div>
          {rows.map((r) => (
            <text key={r}>{r}</text>
          ))}
        </div>
        <button onClick={() => clicks++}>{label}</button>
      </div>
    );
    await root.render(view(["a"], "Stop"));
    const button = root.host.root.children[0].children[1];
    const plan = (revision: number) => ({
      revision,
      target: button.id,
      preventDefault: false,
      deliveries: [{ id: button.id, capture: false }],
    });
    const seen = root.inspect().revision;
    // A streamed row lands in the list; the button is untouched.
    await root.render(view(["a", "b"], "Stop"));
    await root.dispatch(plan(seen), "click");
    expect(clicks).toBe(1);
    await expect(root.dispatch(plan(seen + 5), "click")).rejects.toThrow("Stale");
    // Its label changes: an older click no longer lands.
    const before = root.inspect().revision;
    await root.render(view(["a", "b"], "Stopping"));
    await expect(root.dispatch(plan(before), "click")).rejects.toThrow("Stale");
    expect(clicks).toBe(1);
    // Keys follow focus: they reach the element as it is now.
    await root.dispatch(plan(before), "keydown", { key: "a" });
  });
  test("an activation planned on an identified element lands after it restyles", async () => {
    const { root } = recorder();
    let picks = 0;
    const row = (hovered: boolean) => (
      <div id="row:a" onClick={() => picks++}>
        <text>a</text>
        {hovered && <text>switches</text>}
      </div>
    );
    await root.render(row(false));
    const target = root.host.root.children[0].id;
    const seen = root.inspect().revision;
    await root.render(row(true));
    const plan = (semantic?: string) => ({
      revision: seen,
      target,
      preventDefault: false,
      deliveries: [{ id: target, capture: false }],
      ...(semantic === undefined ? {} : { semantic }),
    });
    await expect(root.dispatch(plan(), "click")).rejects.toThrow("Stale");
    await expect(root.dispatch(plan("row:b"), "click")).rejects.toThrow("Stale");
    await root.dispatch(plan("row:a"), "click");
    expect(picks).toBe(1);
  });
  test("native input observation prevents redundant controlled value writes", async () => {
    const { root, commits } = recorder();
    let set!: (s: string) => void;
    function Field() {
      const [v, update] = useState("a");
      set = update;
      return <input value={v} onInput={(e) => set(String(e.value))} />;
    }
    await root.render(<Field />);
    const id = root.host.root.firstChild!.id;
    await root.dispatch(
      {
        revision: 1,
        target: id,
        preventDefault: false,
        deliveries: [{ id, capture: false }],
      },
      "input",
      { value: "ab" },
    );
    expect(commits).toHaveLength(1);
    set("server");
    await root.flush();
    expect(commits.at(-1)!.ops).toContainEqual({
      op: "setProps",
      id,
      patch: { value: "server" },
    });
  });
  test("Fast Refresh preserves changed hook state and native IDs", async () => {
    const { root } = recorder();
    let set!: (v: number) => void;
    function Counter() {
      const [n, s] = useState(0);
      set = s;
      return <text>{`before:${n}`}</text>;
    }
    function Updated() {
      const [n, s] = useState(0);
      set = s;
      return <text>{`after:${n}`}</text>;
    }
    Refresh.signature(Counter, "useState:number");
    Refresh.signature(Updated, "useState:number");
    Refresh.register(Counter, "test:counter");
    await root.render(<Counter />);
    set(17);
    await root.flush();
    const id = root.host.root.firstChild!.id;
    Refresh.register(Updated, "test:counter");
    Refresh.refresh();
    await root.flush();
    expect(root.host.root.textContent).toBe("after:17");
    expect(root.host.root.firstChild!.id).toBe(id);
    set(18);
    await root.flush();
    expect(root.host.root.textContent).toBe("after:18");
  });
  test("multiple roots own documents and state independently", async () => {
    const a = recorder(),
      b = recorder();
    let set!: (v: number) => void;
    function A() {
      const [n, s] = useState(1);
      set = s;
      return <text>{n}</text>;
    }
    await a.root.render(<A />);
    await b.root.render(<text>b</text>);
    set(2);
    await a.root.flush();
    expect(a.root.host.root.textContent).toBe("2");
    expect(b.root.host.root.textContent).toBe("b");
  });
  test("rejects non-data props and HTML; no getter evaluation by serializer", () => {
    expect(() => h("div", { bad: () => 1 })).toThrow("JSON");
    expect(() =>
      h("div", { dangerouslySetInnerHTML: { __html: "x" } }),
    ).toThrow("HTML");
  });
});
test("async hook updates publish without waiting for another event", async () => {
  const { root, commits } = recorder();
  let update!: (n: number) => void;
  function Counter() {
    const [n, set] = useState(0);
    update = set;
    return <text>{n}</text>;
  }
  await root.render(<Counter />);
  update(22);
  // Drain worker microtasks; no explicit root.flush or incoming UI event.
  for (let n = 0; n < 10; n++) await Promise.resolve();
  expect(commits).toHaveLength(2);
  expect(root.inspect().revision).toBe(2);
});
test("Fast Refresh resets incompatible hook signatures", async () => {
  const { root } = recorder();
  let set!: (n: number) => void;
  function Before() {
    const [n, s] = useState(0);
    set = s;
    return <text>{n}</text>;
  }
  function After() {
    const [n] = useState(5);
    return <text>{n}</text>;
  }
  Refresh.signature(Before, "useState:number");
  Refresh.register(Before, "test:reset");
  await root.render(<Before />);
  set(17);
  await root.flush();
  Refresh.signature(After, "changed:useState:number");
  Refresh.register(After, "test:reset");
  Refresh.refresh();
  await root.flush();
  expect(root.host.root.textContent).toBe("5");
});
test("checked and controlled value removal never emit DOM empty-string coercions", async () => {
  const { root, commits } = recorder();
  await root.render(<input checked={true} value="a" />);
  await root.render(<input checked={false} value="" />);
  expect(root.host.root.firstChild!.props.checked).toBe(false);
  expect(root.host.root.firstChild!.props.value).toBe("");
  const count = commits.length;
  await root.render(<input checked={false} value="" />);
  expect(commits).toHaveLength(count);
  await root.render(<input />);
  expect(root.host.root.firstChild!.props).not.toHaveProperty("checked");
  expect(root.host.root.firstChild!.props).not.toHaveProperty("value");
  expect(commits.at(-1)!.ops).toContainEqual({
    op: "setProps",
    id: root.host.root.firstChild!.id,
    patch: { value: null },
  });
});
