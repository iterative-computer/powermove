import { expect, test } from "bun:test";
import { createRequire } from "node:module";
const require = createRequire(new URL("../../../js/remote-dom/package.json", import.meta.url));
const { JSDOM } = require("jsdom");
import { readFileSync } from "node:fs";

test("extension cssText padding cannot increase the panel body's border box width", () => {
  const source = readFileSync(new URL("../src/transport.rs", import.meta.url), "utf8");
  const css = source.match(/\[data-pm-contribution\]>div\{\{([^}]+)\}\}/)![1];
  const dom = new JSDOM(`<style>[data-pm-contribution]>div{${css}}</style><section data-pm-contribution="logos" style="width:280px"><div></div></section>`);
  const body = dom.window.document.querySelector("div")!;
  // Extensions replace cssText, removing the old host's inline box-sizing.
  body.style.cssText = "width:100%;padding:12px 16px";
  const style = dom.window.getComputedStyle(body);
  expect(style.boxSizing).toBe("border-box");
  expect(style.maxWidth).toBe("100%");
  expect(style.minWidth).toBe("0");
  expect(style.paddingLeft).toBe("16px");
  dom.window.close();
});
