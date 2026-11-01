import { h, Fragment, type ComponentChild, type SurfaceRoot } from "../src";
import { useState } from "../src/hooks";
import {
  Row,
  NumField,
  TextField,
  Section,
  ToggleField,
  SelectField,
  ToolbarItem,
  Glass,
  Disclosure,
  PointField,
  FillField,
  RampField,
} from "../src/ui";
import * as Refresh from "../src/refresh";
function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const noop = () => {};
/** Exercises candidate roots through their public API; expected values come from each spec. */
export async function exercise(
  id: string,
  root: SurfaceRoot,
  factory?: typeof import("../src").createRoot,
) {
  const index = Number(id.slice(-2));
  const group = id.slice(0, id.lastIndexOf("-"));
  const render = (v: ComponentChild) => root.render(v);
  if (group === "builtin") {
    switch (index) {
      case 1:
        await render(
          <Row label="Opacity">
            <NumField value={50} binding="opacity" />
          </Row>,
        );
        assert(
          [...root.host.nodes.values()].some(
            (n) =>
              n.props.binding === "opacity" &&
              typeof n.props.labelledBy === "string",
          ),
          "numeric binding and label",
        );
        break;
      case 2: {
        const rows = (keys: string[]) => (
          <scroll>
            {keys.map((k) => (
              <div key={k} id={k}>
                {k}
              </div>
            ))}
          </scroll>
        );
        await render(rows(["a", "b", "c"]));
        const ids = root.host.root.firstChild!.children.map((n) => n.id);
        await render(rows(["c", "b", "a"]));
        assert(
          root.host.root.firstChild!.children.map((n) => n.id).join() ===
            ids.reverse().join(),
          "timeline keyed identity",
        );
        break;
      }
      case 3:
        await render(
          <scroll>
            <img src="media:1" alt="source" />
            <img src="media:2" alt="source two" />
          </scroll>,
        );
        assert(root.host.nodes.size === 4, "media resource tree");
        break;
      case 4:
        await render(
          <div>
            <path d="M0 0 C1 2 3 4 5 6" />
            <frame surface="engine:frame" />
          </div>,
        );
        assert(
          root.host.root.firstChild!.children[0].props.d ===
            "M0 0 C1 2 3 4 5 6",
          "exact curve",
        );
        break;
      case 5: {
        await render(<Disclosure label="Layer">child</Disclosure>);
        const button = [...root.host.nodes.values()].find(
          (n) => n.type === "button",
        )!;
        await root.dispatch(
          {
            revision: root.inspect().revision,
            target: button.id,
            preventDefault: false,
            deliveries: [{ id: button.id, capture: false }],
          },
          "click",
        );
        assert(
          root.host.root.textContent === "Layerchild",
          "disclosure expand",
        );
        break;
      }
      case 6: {
        let set!: (s: string) => void;
        function Caption() {
          const [v, s] = useState("a");
          set = s;
          return <input value={v} onInput={(e) => set(String(e.value))} />;
        }
        await render(<Caption />);
        const field = root.host.root.firstChild!;
        await root.dispatch(
          {
            revision: root.inspect().revision,
            target: field.id,
            preventDefault: false,
            deliveries: [{ id: field.id, capture: false }],
          },
          "input",
          { value: "native text" },
        );
        assert(field.value === "native text", "controlled observation");
        break;
      }
      case 7:
        await render(
          <Section title="Settings">
            <ToggleField value={true} binding="enabled" />
            <SelectField
              value={2}
              binding="mode"
              options={[{ v: 2, label: "two" }]}
            />
          </Section>,
        );
        assert(
          [...root.host.nodes.values()].some((n) => n.props.value === true),
          "typed boolean",
        );
        break;
      case 8:
        await render(
          <div>
            <TextField value="output.mov" binding="path" />
            <NumField value={60} binding="fps" />
            <text attr={{ progress: 0.5 }}>Export</text>
          </div>,
        );
        assert(root.host.root.textContent === "Export", "export form");
        break;
      case 9:
        await render(
          <div role="listbox">
            <button disabled={true} label="Unavailable" />
            <button label="Save" />
          </div>,
        );
        assert(
          root.host.root.firstChild!.children[0].props.disabled === true,
          "disabled choice",
        );
        break;
      case 10:
        await render(
          <Glass surface="Toolbar">
            <ToolbarItem label="Play" active={true} />
          </Glass>,
        );
        assert(root.host.root.firstChild!.type === "Glass", "Glass primitive");
        break;
    }
  } else if (group === "wild") {
    const trees: ComponentChild[] = [
      <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)" }}>
        {[1, 2, 3, 4].map((n) => (
          <text key={n}>{n}</text>
        ))}
      </div>,
      <div>
        <path d="M0 0 L10 10" />
        <anchored anchor={1}>
          <text>node</text>
        </anchored>
      </div>,
      <scroll>
        <canvas surface="recorder:1" />
        <frame surface="engine:1" />
      </scroll>,
      <div>
        <markdown>{"# Title\n\n  exact"}</markdown>
        <code language="rust">{"fn main() {\n}\n"}</code>
      </div>,
      <virtual-list count={1000000} range={[100, 110]} itemSize={28}>
        <div key="100">100</div>
      </virtual-list>,
      <anchored anchor={1}>
        <button role="menuitem">Close</button>
      </anchored>,
      <div focusScope={true}>
        <input label="First" value="" />
        <textarea label="Second" value="" />
      </div>,
      <div>
        <PointField value={[1, 2]} binding="point" xBinding="x" yBinding="y" />
        <FillField value={{ type: "none" }} binding="fill" />
        <RampField
          value={[{ id: "a", color: "#fff", position: 0 }]}
          binding="ramp"
        />
      </div>,
      <webview src="https://example.test/" />,
      <div
        id="semantic"
        class="a b"
        role="group"
        attr={{ kind: "edit" }}
        state={{ hover: true }}
      />,
    ];
    await render(trees[index - 1]);
    assert(root.host.nodes.size > 1, "wild tree exists");
    if (index === 4)
      assert(
        root.host.root.textContent === "# Title\n\n  exactfn main() {\n}\n",
        "whitespace exact",
      );
    if (index === 5)
      assert(root.host.nodes.size === 4, "virtual list remains bounded");
    if (index === 6) {
      await render(null);
      assert(root.host.nodes.size === 1, "menu close removes tree");
    }
    if (index === 10)
      assert(
        root.host.root.firstChild!.props.state !== undefined,
        "state metadata retained",
      );
  } else if (group === "held-out") {
    switch (index) {
      case 1: {
        await render(
          <div>
            {["a", "b"].map((k) => (
              <button key={k}>{k}</button>
            ))}
          </div>,
        );
        const id = root.host.root.firstChild!.children[1].id;
        await render(
          <div>
            {["b", "a"].map((k) => (
              <button key={k}>{k}</button>
            ))}
          </div>,
        );
        assert(
          root.host.root.firstChild!.children[0].id === id,
          "key identity",
        );
        break;
      }
      case 2:
        await render(
          <div>
            <button onClick={noop}>old</button>
          </div>,
        );
        await render(<text>new</text>);
        assert(
          root.host.nodes.size === 3 && root.host.root.textContent === "new",
          "subtree removal",
        );
        break;
      case 3: {
        await render(<button onClick={noop} />);
        const id = root.host.root.firstChild!.id;
        await render(
          <button onClickCapture={noop} preventDefault={{ click: true }} />,
        );
        assert(
          root.host.root.firstChild!.id === id &&
            root.host.root.firstChild!.handlers.has("click:true"),
          "listener update",
        );
        break;
      }
      case 4:
        await render(<div title="before" attr={{ value: false }} />);
        await render(<div title={null} attr={{ value: false }} label="" />);
        assert(
          !("title" in root.host.root.firstChild!.props) &&
            root.host.root.firstChild!.props.label === "",
          "null removal",
        );
        break;
      case 5:
        await render(
          <Fragment>
            {false}
            {null}
            <text>a</text>
            <Fragment>
              <text>b</text>
            </Fragment>
          </Fragment>,
        );
        assert(
          root.host.nodes.size === 5 && root.host.root.textContent === "ab",
          "fragment shape",
        );
        break;
      case 6: {
        let set!: (n: number) => void;
        function Count() {
          const [n, s] = useState(0);
          set = s;
          return <text>{n}</text>;
        }
        const tree = (reverse: boolean) => (
          <div>
            {reverse && <button key="b" />}
            <Count key="count" />
            {!reverse && <button key="b" />}
          </div>
        );
        await render(tree(false));
        set(17);
        await root.flush();
        await render(tree(true));
        assert(root.host.root.textContent === "17", "nested state");
        break;
      }
      case 7: {
        let set!: (n: number) => void;
        function A() {
          const [n, s] = useState(0);
          set = s;
          return <text>{`a${n}`}</text>;
        }
        function B() {
          const [n] = useState(0);
          return <text>{`b${n}`}</text>;
        }
        Refresh.register(A, "eval:held-7");
        await render(<A />);
        set(17);
        await root.flush();
        Refresh.register(B, "eval:held-7");
        Refresh.refresh();
        await root.flush();
        assert(root.host.root.textContent === "b17", "refresh state");
        break;
      }
      case 8: {
        function Nested() {
          return <TextField binding="name" value="" />;
        }
        await render(
          <Row label="Name">
            <Nested />
          </Row>,
        );
        const field = [...root.host.nodes.values()].find(
          (n) => n.type === "input",
        )!;
        assert(
          typeof field.props.labelledBy === "string",
          "context association",
        );
        break;
      }
      case 9:
        await render(
          <div
            attr={{
              n: -0.125,
              text: "😀\u0000",
              nested: [false, null, { x: 1e-10 }],
            }}
          />,
        );
        assert(
          (root.host.root.firstChild!.props.attr as { text: string }).text ===
            "😀\u0000",
          "exact JSON",
        );
        break;
      case 10: {
        // Dedicated rejection root verifies the fail-closed public contract.
        const { createRoot } = await import("../src");
        let reject = false;
        const candidate = (factory ?? createRoot)(async (c) => ({
          accepted: !reject,
          revision: c.baseRevision + (reject ? 0 : 1),
        }));
        await candidate.render(<text>last good</text>);
        const before = JSON.stringify(candidate.inspect().nodes);
        reject = true;
        try {
          await candidate.render(<text>bad</text>);
          throw Error("accepted invalid commit");
        } catch (e) {
          assert(String(e).includes("rejected"), "rejection reported");
        }
        assert(
          JSON.stringify(candidate.inspect().nodes) === before,
          "last tree retained",
        );
        await render(<text>rejection verified</text>);
        break;
      }
    }
  } else throw new Error("Owner specification pending");
}
