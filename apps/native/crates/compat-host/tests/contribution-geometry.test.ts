import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { PageApplier } from "../../../js/remote-dom/src/applier";
import { RemoteDOM } from "../../../js/remote-dom/src/shim";

const require = createRequire(new URL("../../../js/remote-dom/package.json", import.meta.url));
const { JSDOM } = require("jsdom");
const source = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");

test("a worker style update preserves the dock bounds before the next paint", async () => {
  const { window } = new JSDOM("<!doctype html>", { runScripts: "outside-only" });
  const session = { panel: "fixture", generation: 1 };
  const frames: (() => void)[] = [];
  const acks: any[] = [];
  let respond: ((reply: any) => void) | undefined;
  const host = Object.assign(window, {
    PM_HOST_CONFIG: { base: "", generation: 1 },
    PMRemoteDOM: { PageApplier },
    requestAnimationFrame: (f: () => void) => frames.push(f),
    fetch: () => new Promise(resolve => { respond = resolve; }),
    XMLHttpRequest: class {
      payload: any;
      open(_method: string, url: string) {
        this.payload = JSON.parse(decodeURIComponent(url.split("?q=")[1]));
      }
      send() {
        if (this.payload.kind === "panel_layout_applied") acks.push(this.payload);
        if (this.payload.kind === "remote_dom") realm.receive(this.payload.payload);
      }
      responseText = '{"ok":true,"value":null}';
    },
  });
  const waitForPoll = async () => {
    for (let i = 0; !respond && i < 20; i++) await Bun.sleep(0);
    if (!respond) throw new Error("Host did not poll");
    return respond;
  };
  let seq = 0;
  const send = async (...events: any[]) => {
    const next = await waitForPoll();
    respond = undefined;
    next({ json: async () => ({ generation: 1, seq: ++seq, events }) });
    await waitForPoll();
  };
  const tick = () => frames.splice(0).forEach(f => f());
  const batches: any[] = [];
  const realm = new RemoteDOM({ session, send: batch => (batches.push(batch), true) });
  try {
    host.eval(source("panel-layout.js"));
    host.eval(source("remote-dom.js"));
    const root = realm.document.createElement("section");
    root.setAttribute("data-pm-contribution", "fixture");
    root.style.cssText = "width:420px;overflow:auto";
    const fill = realm.document.createElement("div");
    root.append(fill);
    realm.document.body.append(root);
    realm.flush();
    await send({ kind: "create", panel: 1 }, ...batches.splice(0).map(batch => ({ kind: "batch", panel: 1, batch })), {
      kind: "panel_layout",
      placements: [{ id: "fixture", generation: 1, rect: { x: 10, y: 20, w: 240, h: 120 } }],
    });
    tick();
    tick();
    expect(acks).toHaveLength(1);

    root.style.cssText = "width:420px;overflow:scroll";
    fill.style.width = "55%";
    const measured = realm.measure(root, ["width"]);
    realm.flush();
    await send(...batches.splice(0).map(batch => ({ kind: "batch", panel: 1, batch })));

    const painted = window.document.querySelector("section");
    expect(painted.style.width).toBe("240px");
    expect(painted.style.overflow).toBe("scroll");
    expect(painted.firstElementChild.style.width).toBe("55%");
    expect((await measured).styles.width).toBe("240px");
  } finally {
    realm.dispose();
    window.close();
  }
});
