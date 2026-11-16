import { expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { readFileSync, writeFileSync } from "node:fs";

test("native panel frame: page roots reveal the host surface, controls keep PM-295", () => {
  const source = readFileSync(new URL("../../../crates/compat-host/src/transport.rs", import.meta.url), "utf8");
  const css = source.slice(source.indexOf("<style>") + 7, source.indexOf("</style>"))
    .replaceAll("{{", "{").replaceAll("}}", "}");
  const base = readFileSync(new URL("../../../crates/compat-runner/src/extension-base.css", import.meta.url), "utf8");
  for (const scheme of ["light", "dark"]) {
    const theme = readFileSync(new URL(`../../themes/editions/v1/${scheme}.css`, import.meta.url), "utf8");
    const dom = new JSDOM(`<style>${theme}\n${css}\n${base}</style><section data-pm-contribution><div data-pm-panel-body><button>Apply</button></div></section>`);
    const frame = [...dom.window.document.querySelectorAll("[data-pm-contribution], [data-pm-panel-body]")].map(el => {
      const s = dom.window.getComputedStyle(el);
      expect(s.backgroundColor).toBe("rgba(0, 0, 0, 0)");
      expect(s.borderTopWidth).toBe("0px");
      expect(s.borderRadius).toBe("0");
      return { background: s.backgroundColor, border: s.borderTopWidth, radius: s.borderRadius };
    });
    expect(base).toContain("background: var(--bg-field)");
    if (process.env.PM_FRAME_CAPTURE_DIR) writeFileSync(`${process.env.PM_FRAME_CAPTURE_DIR}/${scheme}-frame.json`, JSON.stringify({ scheme, frame, surface: "colors.panel", alpha: "translucency.panel", corner: "metrics.panel_radius", header: "metrics.header", padding: "metrics.inset" }, null, 2));
    dom.window.close();
  }
});
