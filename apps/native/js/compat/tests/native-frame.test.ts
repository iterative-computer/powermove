import { expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { readFileSync, writeFileSync } from "node:fs";

test("native panel frame uses native tokens and leaves control decoration intact", () => {
  const base = readFileSync(
    new URL("../../../crates/compat-host/src/pm-native.css", import.meta.url),
    "utf8",
  );
  const golden = readFileSync(
    new URL(
      "../../../crates/design/tests/pm-native.snapshot.css",
      import.meta.url,
    ),
    "utf8",
  );
  const frameRules = base.slice(base.indexOf("/* Native panel frame:"));
  expect(frameRules).toContain("padding: var(--pm-inset)");
  expect(base).toContain("background: var(--pm-fill)");
  for (const scheme of ["light", "dark"]) {
    const tokens =
      scheme === "light"
        ? golden.split(":root[data-pm-scheme=dark]")[0]
        : golden.split(":root[data-pm-scheme=dark]")[1];
    const inset = tokens.match(/--pm-inset:([^;]+);/)![1];
    const css = frameRules.replaceAll("var(--pm-inset)", inset);
    const authored =
      "html,body,[data-pm-contribution],[data-pm-panel-body]{background:#424242;border:1px solid black;border-radius:8px;box-shadow:0 0 2px black;margin:9px}button{background:#888888;border:1px solid black}";
    const dom = new JSDOM(
      `<style>${authored}\n${css}</style><section data-pm-contribution><div data-pm-panel-body><button>Apply</button></div></section>`,
    );
    const frame = [
      ...dom.window.document.querySelectorAll(
        "html, body, [data-pm-contribution], [data-pm-panel-body]",
      ),
    ].map((el) => {
      const style = dom.window.getComputedStyle(el);
      expect(style.backgroundColor).toBe("rgba(0, 0, 0, 0)");
      expect(style.borderTopWidth).toBe("0px");
      expect(style.borderRadius).toBe("0");
      expect(style.margin).toBe("0px");
      return {
        background: style.backgroundColor,
        border: style.borderTopWidth,
        radius: style.borderRadius,
      };
    });
    expect(
      dom.window.getComputedStyle(
        dom.window.document.querySelector("[data-pm-panel-body]")!,
      ).padding,
    ).toBe(inset);
    const button = dom.window.getComputedStyle(
      dom.window.document.querySelector("button")!,
    );
    expect(button.backgroundColor).toBe("rgb(136, 136, 136)");
    expect(button.borderTopWidth).toBe("1px");
    if (process.env.PM_FRAME_CAPTURE_DIR)
      writeFileSync(
        `${process.env.PM_FRAME_CAPTURE_DIR}/${scheme}-frame.json`,
        JSON.stringify(
          {
            scheme,
            frame,
            surface: "colors.panel",
            alpha: "translucency.panel",
            corner: "metrics.panel_radius",
            header: "metrics.header",
            padding: inset,
          },
          null,
          2,
        ),
      );
    dom.window.close();
  }
});
