// Background keyboard check; Node 22+ and a headless Chromium binary.
// node native-style-keyboard.mjs /path/to/chrome-headless-shell /path/to/pm-native.snapshot.css [capture-directory]
import { spawn } from "node:child_process";
import { readFile, writeFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import assert from "node:assert/strict";
const binary = process.argv[2],
  css = await readFile(process.argv[3], "utf8"),
  out = process.argv[4];
const profile = await mkdtemp(join(tmpdir(), "pm-style-keyboard-"));
const child = spawn(
  binary,
  [
    "--headless",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "about:blank",
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
let socket;
try {
  const endpoint = await new Promise((resolve, reject) => {
    let log = "";
    const timeout = setTimeout(
      () => reject(Error("headless startup timed out")),
      15000,
    );
    child.stderr.on("data", (b) => {
      log += b;
      const m = log.match(/DevTools listening on (ws:\/\/\S+)/);
      if (m) {
        clearTimeout(timeout);
        resolve(m[1]);
      }
    });
    child.on("error", reject);
  });
  socket = new WebSocket(endpoint);
  await new Promise((r, j) => {
    socket.onopen = r;
    socket.onerror = j;
  });
  let id = 0;
  const pending = new Map();
  socket.onmessage = ({ data }) => {
    const m = JSON.parse(data);
    if (m.id) {
      const p = pending.get(m.id);
      pending.delete(m.id);
      m.error
        ? p?.reject(Error(JSON.stringify(m.error)))
        : p?.resolve(m.result);
    }
  };
  const send = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      const n = ++id;
      pending.set(n, { resolve, reject });
      socket.send(JSON.stringify({ id: n, method, params, sessionId }));
    });
  const { targetId } = await send("Target.createTarget", {
    url: "about:blank",
  });
  const { sessionId } = await send("Target.attachToTarget", {
    targetId,
    flatten: true,
  });
  const call = (method, params) => send(method, params, sessionId);
  await call("Page.enable");
  const { frameTree } = await call("Page.getFrameTree");
  const evaluate = async (expression) => {
    const result = await call("Runtime.evaluate", {
      expression,
      returnByValue: true,
    });
    assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  for (const scheme of ["light", "dark"]) {
    await call("Page.setDocumentContent", {
      frameId: frameTree.frame.id,
      html: `<!doctype html><html data-pm-scheme="${scheme}" style="color-scheme:${scheme}"><head><style>${css}</style></head><body><div class="pm-toolbar"><input class="pm-search" type="search" placeholder="Search logos" aria-label="Search logos"><button class="pm-button">Import</button></div><div class="pm-segmented" aria-label="Format"><button aria-pressed="true">All</button><button aria-pressed="false">SVG</button></div><span id="swatch" style="color:var(--pm-focus)"></span></body></html>`,
    });
    for (const expected of ["INPUT", "BUTTON"]) {
      await call("Input.dispatchKeyEvent", {
        type: "keyDown",
        key: "Tab",
        code: "Tab",
        windowsVirtualKeyCode: 9,
      });
      await call("Input.dispatchKeyEvent", {
        type: "keyUp",
        key: "Tab",
        code: "Tab",
        windowsVirtualKeyCode: 9,
      });
      const result = await evaluate(
        `(()=>{const e=document.activeElement,s=getComputedStyle(e),r=getComputedStyle(document.documentElement);return {tag:e.tagName,visible:e.matches(':focus-visible'),count:document.querySelectorAll(':focus-visible').length,color:s.outlineColor,expected:getComputedStyle(document.getElementById('swatch')).color,width:s.outlineWidth,expectedWidth:r.getPropertyValue('--pm-focus-width').trim(),offset:s.outlineOffset,expectedOffset:r.getPropertyValue('--pm-focus-offset').trim()}})()`,
      );
      assert.equal(result.tag, expected);
      assert.equal(result.visible, true);
      assert.equal(result.count, 1);
      assert.equal(result.color, result.expected);
      assert.equal(result.width, result.expectedWidth);
      assert.equal(result.offset, result.expectedOffset);
    }
    if (out) {
      const { data } = await call("Page.captureScreenshot", { format: "png" });
      await writeFile(
        join(out, `kit-tab-${scheme}.png`),
        Buffer.from(data, "base64"),
      );
    }
    console.log(
      `HEADLESS KEYBOARD PASS ${scheme}: Tab moves focus, exactly one native neutral ring, recipe width and offset`,
    );
    await call("Page.setDocumentContent", {
      frameId: frameTree.frame.id,
      html: `<!doctype html><html data-pm-scheme="${scheme}" style="color-scheme:${scheme}"><head><style>${css}</style></head><body>
        <label><input id="toggle" type="checkbox">Visible</label>
        <input id="disabled" type="checkbox" checked disabled>
        <input id="mixed" type="checkbox">
        <div data-pm-style="tokens-only"><input id="system" type="checkbox"></div>
        <i id="accent" style="color:var(--pm-accent)"></i>
        </body></html>`,
    });
    const checkbox = await evaluate(`(() => {
      const toggle = document.querySelector('#toggle');
      const idle = getComputedStyle(toggle).boxShadow;
      toggle.closest('label').click();
      const checked = getComputedStyle(toggle);
      const mixed = document.querySelector('#mixed');
      mixed.indeterminate = true;
      return {
        idle, checked: toggle.checked, size: toggle.getBoundingClientRect().width,
        fill: checked.backgroundColor, mark: checked.backgroundImage,
        accent: getComputedStyle(document.querySelector('#accent')).color,
        disabled: getComputedStyle(document.querySelector('#disabled')).opacity,
        mixed: getComputedStyle(mixed).backgroundImage,
        optOut: getComputedStyle(document.querySelector('#system')).appearance,
      };
    })()`);
    assert.notEqual(checkbox.idle, "none");
    assert.equal(checkbox.checked, true);
    assert.equal(checkbox.size, 16);
    assert.equal(checkbox.fill, checkbox.accent);
    assert.notEqual(checkbox.mark, "none");
    assert.notEqual(checkbox.mixed, "none");
    assert.notEqual(checkbox.mixed, checkbox.mark);
    assert.equal(checkbox.disabled, "0.45");
    assert.equal(checkbox.optOut, "auto");
    console.log(
      `HEADLESS CHECKBOX PASS ${scheme}: label toggle, accent, bezel, mixed, disabled and opt-out`,
    );
    await call("Page.setDocumentContent", {
      frameId: frameTree.frame.id,
      html: `<!doctype html><html data-pm-scheme="${scheme}"><head><style>
        ${css}
        .insp { width: 240px; }
        .insp .row.split { height: 30px; margin: 2px 0; padding: 0 16px; }
        .insp .row.split .k { width: 78px; }
        </style></head><body><div class="insp">
        ${["Width", "Height", "Duration", "Frame rate", "Background"]
          .map(
            (label, index) => `<div class="row split">
            <div class="k" id="pm-control-label-${index}">${label}</div>
            <div class="vwrap"><input value="${["1920", "1080", "4s", "30", "#202024"][index]}"></div>
          </div>`,
          )
          .join("")}
        </div></body></html>`,
    });
    const rows =
      await evaluate(`Array.from(document.querySelectorAll('.row.split'), row => ({
      row: row.getBoundingClientRect().toJSON(),
      label: row.querySelector('.k').getBoundingClientRect().toJSON(),
      value: row.querySelector('input').getBoundingClientRect().toJSON(),
    }))`);
    for (const [index, { row, label, value }] of rows.entries()) {
      assert(
        label.right <= value.left,
        `Row ${index} has no label and value columns`,
      );
      assert(
        value.top >= row.top &&
          value.bottom <= row.bottom &&
          value.right <= row.right,
        `Row ${index} does not contain its value`,
      );
      assert.equal(value.left, rows[0].value.left);
      if (index) assert(rows[index - 1].value.bottom <= label.top);
    }
    console.log(
      `HEADLESS ROWS PASS ${scheme}: labels and values align without overlap`,
    );
  }
} finally {
  socket?.close();
  child.kill();
  await new Promise((r) => child.once("exit", r));
  await rm(profile, { recursive: true, force: true });
}
