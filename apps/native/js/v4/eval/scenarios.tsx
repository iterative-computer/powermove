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
  } else throw new Error("Use exerciseOwner for owner behavior fixtures");
}

/** Synthetic owner fixtures use public snapshots/events. Service replies and
 * command logs below are test seams, never working project or media services.
 * Full acceptance is reported separately by runOwner; unknown props/handles
 * are descriptors, not evidence of native rendering or resource registration.
 */
export async function exerciseOwner(
  index: number,
  e: import("./harness").Evaluation,
) {
  switch (index) {
    case 1: {
      let handles = [0.15, -0.2, 0.85, 1.2];
      let omitted = new Set<number>();
      let targets = [
        { slot: 1, protected: false },
        { slot: 2, protected: true },
      ];
      const edits: number[][] = [];
      const view = () => (
        <div>
          <path id="curve-graph" d={`M0 0 C${handles.join(" ")} 1 1`} />
          {handles.map((value, axis) => (
            <input
              key={axis}
              id={`coordinate-${axis}`}
              value={value}
              onKeyDown={(event) => {
                if (event.key === "ArrowRight") handles[axis] += 0.05;
                if (axis % 2 === 0)
                  handles[axis] = Math.max(0, Math.min(1, handles[axis]));
              }}
            />
          ))}
          <scroll>
            {targets.map((target) => (
              <button
                key={target.slot}
                id={`target-${target.slot}`}
                disabled={target.protected}
                state={{ selected: !omitted.has(target.slot) }}
                onClick={() => omitted.add(target.slot)}
              />
            ))}
          </scroll>
          <button
            id="send-curve"
            onClick={() => {
              const admitted = targets.filter(
                (target) => !target.protected && !omitted.has(target.slot),
              );
              if (admitted.length)
                edits.push(admitted.map((target) => target.slot));
            }}
          />
        </div>
      );
      await e.render(view());
      e.equal(
        e.nodes().filter((node) => node.type === "input").length,
        4,
        "four coordinate controls",
      );
      await e.event("coordinate-0", "keydown", { key: "ArrowRight" });
      await e.render(view());
      e.props("coordinate-0", { value: 0.2 });
      e.props("curve-graph", { d: `M0 0 C${handles.join(" ")} 1 1` });
      e.equal(handles[1], -0.2, "vertical overshoot retained");
      await e.event("send-curve", "click");
      e.equal(edits, [[1]], "protected target excluded");
      await e.event("target-1", "click");
      await e.event("send-curve", "click");
      e.equal(edits.length, 1, "omitted targets create no edit");
      // Synthetic selection notification resets omissions; not a host subscription.
      targets = [{ slot: 3, protected: false }];
      omitted = new Set();
      await e.render(view());
      e.assert(!e.find("target-1"), "retired selection removed");
      await e.event("send-curve", "click");
      e.equal(edits, [[1], [3]], "new selection admitted");
      handles[0] = 1;
      await e.event("coordinate-0", "keydown", { key: "ArrowRight" });
      e.equal(handles[0], 1, "horizontal bound clamped");
      break;
    }
    case 2: {
      let epoch = 0,
        positive = false,
        busy = false,
        sourceVersion = 1;
      let finished: { version: number; frames: number[] } | undefined;
      let progress = 0;
      const insertions: { version: number; frames: number[] }[] = [];
      const begin = () => {
        busy = true;
        finished = undefined;
        return ++epoch;
      };
      const complete = (token: number, version: number) => {
        if (token === epoch) {
          busy = false;
          progress = 1;
          finished = { version, frames: [0, 1, 2] };
        }
      };
      const view = () => (
        <div>
          <canvas id="brush-preview" surface="synthetic-preview" />
          <button
            id="positive-guide"
            onClick={() => {
              positive = true;
            }}
          />
          <button
            id="analyze-clip"
            disabled={!positive || busy}
            onClick={() => {
              if (positive && !busy) begin();
            }}
          />
          <button
            id="cancel-work"
            disabled={!busy}
            onClick={() => {
              ++epoch;
              busy = false;
            }}
          />
          <text id="analysis-progress" attr={{ fraction: progress }} />
          {[0, 1, 2].map((frame) => (
            <button key={frame} id={`review-${frame}`} disabled={!finished} />
          ))}
          <button
            id="insert-cutout"
            disabled={!finished || busy}
            onClick={() => {
              if (finished && !busy && finished.version === sourceVersion) {
                insertions.push(structuredClone(finished));
                finished = undefined;
              }
            }}
          />
        </div>
      );
      await e.render(view());
      e.props("analyze-clip", { disabled: true });
      await e.event("positive-guide", "click");
      await e.render(view());
      e.props("analyze-clip", { disabled: false });
      await e.event("analyze-clip", "click");
      const retired = epoch;
      await e.render(view());
      e.props("insert-cutout", { disabled: true });
      await e.event("cancel-work", "click");
      complete(retired, sourceVersion);
      await e.render(view());
      e.props("insert-cutout", { disabled: true });
      e.equal(insertions, [], "cancelled completion creates no layer");
      const next = begin();
      complete(next, sourceVersion);
      ++sourceVersion;
      await e.render(view());
      await e.event("insert-cutout", "click");
      e.equal(insertions.length, 0, "changed source rejects result");
      finished = undefined;
      const fresh = begin();
      complete(fresh, sourceVersion);
      await e.render(view());
      e.props("review-2", { disabled: false });
      await e.event("insert-cutout", "click");
      await e.render(view());
      e.equal(
        insertions,
        [{ version: 2, frames: [0, 1, 2] }],
        "one whole analysis insertion request",
      );
      e.props("insert-cutout", { disabled: true });
      break;
    }
    case 3: {
      let phase = "waiting",
        permission = false,
        permissionCalls = 0,
        resource = false;
      let take = false,
        imported = false,
        imports = 0,
        refuse = true,
        context = 1;
      let origin = 1,
        insertionAttempts = 0,
        released = 0;
      const duration = 3.5,
        remaining = 2.25;
      const insertions: number[] = [];
      const save = () => {
        if (!take || context !== origin) {
          phase = "recoverable";
          return;
        }
        if (!imported) {
          imported = true;
          imports++;
        }
        insertionAttempts++;
        if (refuse) {
          phase = "recoverable";
          return;
        }
        insertions.push(Math.min(duration, remaining));
        take = false;
        imported = false;
        phase = "waiting";
      };
      const release = () => {
        if (resource) released++;
        resource = false;
      };
      const view = () => (
        <div>
          <button
            id="capture-action"
            disabled={take}
            onClick={() => {
              permissionCalls++;
              if (!permission) {
                phase = "denied";
                return;
              }
              resource = true;
              origin = context;
              phase = "capturing";
            }}
          />
          <text id="take-state">{phase}</text>
          <text id="meter-observation" attr={{ peak: -9, elapsed: 1.25 }} />
          <button
            id="finish-capture"
            onClick={() => {
              if (resource) {
                take = true;
                release();
                save();
              }
            }}
          />
          <button id="retry-save" disabled={!take} onClick={save} />
          <button id="recover-take" disabled={!take} />
        </div>
      );
      await e.render(view());
      e.equal(permissionCalls, 0, "mount has no device access");
      await e.event("capture-action", "click");
      await e.render(view());
      e.equal(imports, 0, "denied capture has no import");
      e.equal(phase, "denied", "permission rejection shown");
      permission = true;
      await e.event("capture-action", "click");
      await e.event("finish-capture", "click");
      await e.render(view());
      e.props("recover-take", { disabled: false });
      e.equal(imports, 1, "take imported once before rejected insertion");
      context = 2;
      await e.event("retry-save", "click");
      e.equal(insertionAttempts, 1, "other composition untouched");
      context = 1;
      refuse = false;
      await e.event("retry-save", "click");
      await e.render(view());
      e.equal(imports, 1, "retry reuses imported take");
      e.equal(insertions, [2.25], "insertion clamped to available duration");
      e.props("recover-take", { disabled: true });
      await e.event("capture-action", "click");
      await e.render(null);
      release();
      e.equal(released, 2, "stop and retirement release devices");
      break;
    }
    case 4: {
      type Card = { slot: number; downloadable: boolean };
      let generation = 0,
        retired = false,
        page = 1,
        failed = true,
        busyItem = false;
      let cards: Card[] = [],
        released = 0;
      const imports: number[] = [],
        requests: { token: number; query: string }[] = [];
      const request = (query: string) => {
        const token = ++generation;
        requests.push({ token, query });
        return token;
      };
      const reply = (token: number, values: Card[]) => {
        if (token === generation && !retired) {
          released += cards.length;
          cards = values;
        }
      };
      const view = () => (
        <div>
          <button id="previous-page" disabled={page === 1} />
          <scroll>
            {cards.map((card) => (
              <div key={card.slot} id={`media-card-${card.slot}`}>
                <img
                  src={`synthetic-media-${card.slot}`}
                  alt={`Example ${card.slot}`}
                />
                <button
                  id={`transfer-${card.slot}`}
                  disabled={!card.downloadable || busyItem}
                  onClick={() => {
                    if (!card.downloadable || busyItem) return;
                    busyItem = true;
                    if (failed) {
                      busyItem = false;
                      return;
                    }
                    imports.push(card.slot);
                    busyItem = false;
                  }}
                />
              </div>
            ))}
          </scroll>
        </div>
      );
      const old = request("first invented query"),
        current = request("second invented query");
      reply(current, [
        { slot: 5, downloadable: true },
        { slot: 6, downloadable: false },
      ]);
      reply(old, [{ slot: 9, downloadable: true }]);
      await e.render(view());
      e.assert(!e.find("media-card-9"), "old response ignored");
      e.props("previous-page", { disabled: true });
      e.props("transfer-6", { disabled: true });
      await e.event("transfer-5", "click");
      e.equal(imports, [], "failed transfer does not import");
      failed = false;
      await e.event("transfer-5", "click");
      e.equal(imports, [5], "retry imports selected original");
      page = 2;
      await e.render(view());
      e.props("previous-page", { disabled: false });
      retired = true;
      released += cards.length;
      cards = [];
      ++generation;
      reply(current, [{ slot: 10, downloadable: true }]);
      await e.render(null);
      e.equal(cards, [], "retired results cannot repopulate");
      e.equal(released, 2, "preview resources released");
      e.equal(
        requests.map((item) => item.token),
        [1, 2],
        "distinct request generations",
      );
      break;
    }
    case 5: {
      type Candidate = { slot: number; transparent: boolean };
      let cards: Candidate[] = [],
        chosen = 0,
        selectionEpoch = 0,
        provenanceLabel = "";
      let originalTransparent = false;
      const imported = new Set<number>(),
        requests: number[] = [];
      const append = (batch: Candidate[]) => {
        for (const candidate of batch)
          if (
            candidate.transparent &&
            !cards.some((card) => card.slot === candidate.slot)
          )
            cards.push(candidate);
      };
      const choose = (slot: number) => {
        chosen = slot;
        provenanceLabel = "";
        return ++selectionEpoch;
      };
      const attribution = (token: number, text: string) => {
        if (token === selectionEpoch) provenanceLabel = text;
      };
      const view = () => (
        <div>
          <scroll>
            {cards.map((card) => (
              <button
                key={card.slot}
                id={`verified-${card.slot}`}
                state={{ selected: chosen === card.slot }}
                onClick={() => choose(card.slot)}
              >
                <img
                  src={`synthetic-verified-${card.slot}`}
                  alt={`Asset ${card.slot}`}
                />
              </button>
            ))}
          </scroll>
          <text id="attribution-result">{provenanceLabel}</text>
          <button
            id="transfer-verified"
            disabled={!chosen || imported.has(chosen)}
            onClick={() => {
              if (chosen && originalTransparent && !imported.has(chosen)) {
                requests.push(chosen);
                imported.add(chosen);
              }
            }}
          />
        </div>
      );
      append([
        { slot: 4, transparent: true },
        { slot: 1, transparent: false },
        { slot: 7, transparent: true },
      ]);
      append([
        { slot: 4, transparent: true },
        { slot: 8, transparent: true },
      ]);
      await e.render(view());
      e.equal(
        cards.map((card) => card.slot),
        [4, 7, 8],
        "verification keeps order and removes opaque duplicates",
      );
      const prior = choose(4),
        current = choose(7);
      attribution(current, "Synthetic license B");
      attribution(prior, "Synthetic license A");
      await e.render(view());
      e.equal(
        provenanceLabel,
        "Synthetic license B",
        "old attribution ignored",
      );
      await e.event("transfer-verified", "click");
      e.equal(requests, [], "opaque original refused");
      originalTransparent = true;
      await e.event("transfer-verified", "click");
      await e.render(view());
      e.props("transfer-verified", { disabled: true });
      e.equal(requests, [7], "verified original imported once");
      break;
    }
    case 6: {
      const reference = [4, 12, 20, 255, 30, 40, 50, 255];
      let actual = [4, 12, 20, 0, 30, 40, 50, 0];
      let geometryValid = true,
        revision = 1,
        changeDuringRead = false,
        busy = false;
      let summary = "waiting",
        frameReads = 0,
        released = 0;
      let stats = { pixels: 0, maximum: 0, mean: 0 };
      const documentWrites: unknown[] = [];
      const compare = () => {
        busy = true;
        try {
          if (!geometryValid) {
            summary = "unexpected";
            return;
          }
          const base = revision;
          frameReads++;
          let changed = 0,
            maximum = 0,
            sum = 0;
          for (let pixel = 0; pixel < actual.length; pixel += 4) {
            let differs = false;
            for (let channel = 0; channel < 3; channel++) {
              const delta = Math.abs(
                actual[pixel + channel] - reference[pixel + channel],
              );
              if (delta) differs = true;
              sum += delta;
              maximum = Math.max(maximum, delta);
            }
            if (differs) changed++;
          }
          if (changeDuringRead) revision++;
          stats = { pixels: changed, maximum, mean: sum / 6 };
          summary = revision === base ? "valid" : "stale";
        } finally {
          if (geometryValid) released++;
          busy = false;
        }
      };
      const view = () => (
        <div>
          <button id="measure-frames" disabled={busy} onClick={compare} />
          <text
            id="comparison-summary"
            attr={{ ...stats, valid: summary === "valid" }}
          >
            {summary}
          </text>
          {summary === "valid" && (
            <img
              id="difference-preview"
              src="synthetic-difference"
              alt="Computed difference preview"
            />
          )}
        </div>
      );
      await e.render(view());
      await e.event("measure-frames", "click");
      await e.render(view());
      e.equal(
        stats,
        { pixels: 0, maximum: 0, mean: 0 },
        "alpha does not affect RGB comparison",
      );
      actual = [4, 12, 23, 255, 30, 40, 50, 255];
      await e.event("measure-frames", "click");
      e.equal(
        stats,
        { pixels: 1, maximum: 3, mean: 0.5 },
        "exact single-channel difference",
      );
      geometryValid = false;
      await e.event("measure-frames", "click");
      e.equal(frameReads, 2, "invalid geometry performs no frame read");
      geometryValid = true;
      changeDuringRead = true;
      await e.event("measure-frames", "click");
      await e.render(view());
      e.assert(!e.find("difference-preview"), "stale preview withheld");
      e.props("measure-frames", { disabled: false });
      e.equal(documentWrites, [], "comparison never edits the document");
      e.equal(released, 3, "each read releases temporary frame");
      break;
    }
    case 7: {
      type Target = {
        protected: boolean;
        expression: boolean;
        animated: boolean;
      };
      const plan = (
        target: Target,
        pivot: number[],
        position: number[],
        next: number[],
      ) => {
        if (
          target.protected ||
          target.expression ||
          next.every((value, axis) => value === pivot[axis])
        )
          return [];
        // Invented transform rotates and scales: [0,-3;2,0].
        const dx = next[0] - pivot[0],
          dy = next[1] - pivot[1];
        return [...next, position[0] - 3 * dy, position[1] + 2 * dx];
      };
      const target = { protected: false, expression: false, animated: false };
      let chosen = -1;
      const writes: number[][] = [];
      const view = () => (
        <div role="group">
          {Array.from({ length: 9 }, (_, slot) => (
            <button
              key={slot}
              id={`pivot-choice-${slot}`}
              state={{ selected: chosen === slot }}
              onClick={() => {
                const next = [(slot % 3) * 20, Math.floor(slot / 3) * 10];
                const command = plan(target, [0, 0], [7, 11], next);
                if (command.length) writes.push(command);
                chosen = slot;
              }}
            />
          ))}
        </div>
      );
      await e.render(view());
      e.equal(
        e.nodes().filter((node) => node.type === "button").length,
        9,
        "nine pivot choices",
      );
      await e.event("pivot-choice-4", "click");
      await e.render(view());
      e.equal(writes, [[20, 10, -23, 51]], "compensated edit values");
      // For any local point q, M(q-newPivot)+newPosition equals Mq+initial offset.
      const oldScreen = [7 - 3 * 6, 11 + 2 * 8];
      const newScreen = [-23 - 3 * (6 - 10), 51 + 2 * (8 - 20)];
      e.equal(
        newScreen,
        oldScreen,
        "screen position preserved under rotation and scale",
      );
      e.props("pivot-choice-4", { state: { selected: true } });
      for (const blocked of [
        { ...target, protected: true },
        { ...target, expression: true },
      ])
        e.equal(
          plan(blocked, [0, 0], [7, 11], [20, 10]),
          [],
          "protected channel has no edit",
        );
      e.equal(
        plan(target, [20, 10], [7, 11], [20, 10]),
        [],
        "aligned pivot is a no-op",
      );
      const seen = new Set([1]);
      const automatic: number[] = [];
      const notify = (slot: number, kind: string, animated: boolean) => {
        const fresh = !seen.has(slot);
        seen.add(slot);
        if (kind === "creation" && fresh && !animated) automatic.push(slot);
      };
      notify(1, "creation", false);
      notify(2, "load", false);
      notify(3, "creation", true);
      notify(4, "creation", false);
      notify(4, "creation", false);
      e.equal(
        automatic,
        [4],
        "automatic edits limited to genuinely new unanimated artwork",
      );
      break;
    }
    case 8: {
      let rows = [
        { slot: 1, protected: false, attached: true, color: "#123456" },
        { slot: 2, protected: false, attached: false, color: "#654321" },
        { slot: 3, protected: true, attached: false, color: "#445566" },
      ];
      let selected = new Set([1, 2, 3]),
        busy = false,
        rejectInstance = true;
      const edits: string[] = [];
      const view = () => (
        <div>
          <scroll id="effect-rows">
            {rows.map((row) => (
              <button
                key={row.slot}
                id={`effect-row-${row.slot}`}
                disabled={row.protected}
                attr={{ swatch: row.color }}
                state={{ selected: selected.has(row.slot) }}
                onClick={(event) => {
                  if (row.protected) return;
                  if (!event.shiftKey) selected = new Set();
                  selected.add(row.slot);
                }}
              />
            ))}
          </scroll>
          <button
            id="batch-effect"
            disabled={
              busy ||
              !rows.some((row) => selected.has(row.slot) && !row.protected)
            }
            onClick={() => {
              if (busy) return;
              busy = true;
              try {
                const admitted = rows.filter(
                  (row) => selected.has(row.slot) && !row.protected,
                );
                if (!admitted.length) return;
                if (admitted.every((row) => row.attached)) {
                  for (const row of admitted) {
                    edits.push(`remove-instance-${row.slot}`);
                    if (rejectInstance) edits.push(`remove-type-${row.slot}`);
                  }
                } else
                  for (const row of admitted)
                    if (!row.attached) edits.push(`attach-${row.slot}`);
              } finally {
                busy = false;
              }
            }}
          />
        </div>
      );
      await e.render(view());
      const before = new Map(
        rows.map((row) => [row.slot, e.node(`effect-row-${row.slot}`).id]),
      );
      rows = [...rows].reverse();
      await e.render(view());
      for (const row of rows) {
        e.equal(
          e.node(`effect-row-${row.slot}`).id,
          before.get(row.slot),
          "keyed row identity retained",
        );
        e.props(`effect-row-${row.slot}`, { attr: { swatch: row.color } });
      }
      await e.event("batch-effect", "click");
      e.equal(
        edits,
        ["attach-2"],
        "mixed action adds missing admitted effect only",
      );
      selected = new Set([1]);
      await e.render(view());
      await e.event("batch-effect", "click");
      await e.render(view());
      e.equal(
        edits.slice(1),
        ["remove-instance-1", "remove-type-1"],
        "instance miss retries by type",
      );
      e.props("batch-effect", { disabled: false });
      busy = true;
      await e.render(view());
      await e.event("batch-effect", "click");
      e.equal(edits.length, 3, "busy action cannot repeat");
      busy = false;
      await e.event("effect-row-2", "click");
      await e.event("effect-row-1", "click", { shiftKey: true });
      e.equal([...selected], [2, 1], "shift extends selection");
      break;
    }
    case 9: {
      let mode = "waiting",
        distance = 0,
        score = 0,
        best = 0,
        warning = false,
        retired = false;
      const stored: number[] = [];
      const readBest = (value: number) => {
        if (Number.isFinite(value) && value >= 0)
          best = Math.max(best, Math.floor(value));
      };
      const pause = () => {
        if (mode === "active") mode = "suspended";
      };
      const tick = (seconds: number) => {
        if (mode === "active" && !retired) distance += Math.min(seconds, 0.04);
      };
      const collide = (fail: boolean) => {
        mode = "ended";
        best = Math.max(best, score);
        if (fail) warning = true;
        else stored.push(best);
      };
      const view = () => (
        <div>
          <button
            id="arcade-input"
            preventDefault={{ keydown: ["Enter", "Escape"] }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                if (mode === "suspended") mode = "active";
                else pause();
              }
              if (event.key === "Enter") mode = "active";
            }}
          >
            <canvas surface="synthetic-playfield" />
          </button>
          <text id="arcade-summary" attr={{ score, best, warning }}>
            {mode}
          </text>
        </div>
      );
      readBest(NaN);
      readBest(-10);
      readBest(6.8);
      await e.render(view());
      e.equal(best, 6, "only finite nonnegative saved score accepted");
      e.listener(e.node("arcade-input").id, "keydown", {
        capture: false,
        bubble: true,
        preventDefault: true,
        keys: ["Enter", "Escape"],
      });
      await e.event("arcade-input", "keydown", { key: "Enter" });
      tick(3);
      e.equal(distance, 0.04, "long frame bounded");
      await e.event("arcade-input", "keydown", { key: "Escape" });
      tick(1);
      e.equal(distance, 0.04, "paused frame does not advance");
      await e.event("arcade-input", "keydown", { key: "Escape" });
      pause();
      tick(1);
      e.equal(distance, 0.04, "synthetic visibility loss pauses");
      score = 8;
      collide(false);
      score = 2;
      collide(false);
      score = 10;
      collide(true);
      await e.render(view());
      e.equal(stored, [8, 8], "persisted high score never decreases");
      e.props("arcade-summary", {
        attr: { score: 10, best: 10, warning: true },
      });
      mode = "active";
      retired = true;
      tick(1);
      e.equal(distance, 0.04, "retired scheduler cannot advance");
      break;
    }
    case 10: {
      let time = 2.5,
        value = "",
        status = "waiting",
        reject = false,
        active = true;
      const annotations = [{ slot: 9, time: 2.5, value: "Synthetic entry" }];
      const writes: { slot?: number; time: number; value: string }[] = [];
      const observe = (next: number) => {
        if (!active) return;
        time = next;
        value =
          annotations.find((annotation) => annotation.time === time)?.value ??
          "";
      };
      const submit = () => {
        if (!value.trim()) {
          status = "missing-text";
          return;
        }
        if (reject) {
          status = "edit-refused";
          return;
        }
        const existing = annotations.find(
          (annotation) => annotation.time === time,
        );
        writes.push({ slot: existing?.slot, time, value: value.trim() });
        status = "acknowledged";
      };
      const view = () => (
        <div>
          <Row label="Annotation content">
            <TextField
              id="annotation-input"
              binding="synthetic-annotation"
              value={value}
              onInput={(event) => {
                value = String(event.value);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") submit();
              }}
            />
          </Row>
          <button id="submit-annotation" onClick={submit} />
          <text id="annotation-status" role="status">
            {status}
          </text>
        </div>
      );
      observe(time);
      await e.render(view());
      e.props("annotation-input", { value: "Synthetic entry" });
      e.assert(
        typeof e.node("annotation-input").props.labelledBy === "string",
        "annotation field has a label",
      );
      // No UI input echo is claimed: replies are explicitly republished through
      // render. The full native-input inspector limitation remains in apiGaps.
      await e.event("annotation-input", "input", { value: "  " });
      await e.render(view());
      await e.event("submit-annotation", "click");
      e.equal(writes, [], "blank annotation has no edit");
      await e.event("annotation-input", "input", {
        value: "  Invented replacement  ",
      });
      await e.render(view());
      await e.event("annotation-input", "keydown", { key: "Enter" });
      e.equal(
        writes,
        [{ slot: 9, time: 2.5, value: "Invented replacement" }],
        "exact-time annotation updated by identity",
      );
      observe(4.75);
      await e.render(view());
      e.props("annotation-input", { value: "" });
      await e.event("annotation-input", "input", {
        value: "Invented new entry",
      });
      await e.render(view());
      reject = true;
      await e.event("submit-annotation", "click");
      await e.render(view());
      e.equal(
        e.nativeObservations.get(e.node("annotation-input").id),
        "Invented new entry",
        "test host retains the rejected draft observation",
      );
      e.equal(status, "edit-refused", "negative acknowledgement visible");
      reject = false;
      await e.event("submit-annotation", "click");
      e.equal(
        writes[1],
        { slot: undefined, time: 4.75, value: "Invented new entry" },
        "empty playhead position creates annotation",
      );
      active = false;
      observe(12);
      e.equal(time, 4.75, "retired time subscription is detached");
      break;
    }
    default:
      throw Error(`Unknown owner fixture ${index}`);
  }
}
