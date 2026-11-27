import { expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { enhanceSelect } from "../src/frozen/reference/controls/select/enhance";

test("compat SelectField toggles, consumes an outside click, and returns focus on Escape", () => {
  const page = new JSDOM("<!doctype html><body></body>").window;
  const globals = {
    window: page,
    document: page.document,
    matchMedia: () => ({ matches: true }),
    MutationObserver: page.MutationObserver,
    HTMLSelectElement: page.HTMLSelectElement,
    HTMLOptGroupElement: page.HTMLOptGroupElement,
  };
  const previous = Object.fromEntries(
    Object.keys(globals).map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]),
  );
  for (const [key, value] of Object.entries(globals))
    Object.defineProperty(globalThis, key, { configurable: true, value });

  try {
    const makeSelect = () => {
      const select = page.document.createElement("select");
      select.innerHTML = '<option value="a">A</option><option value="b">B</option>';
      page.document.body.append(select);
      const dispose = enhanceSelect(select as unknown as HTMLSelectElement);
      return { trigger: select.previousElementSibling!, dispose };
    };
    const a = makeSelect();
    const b = makeSelect();
    const outside = page.document.createElement("button");
    page.document.body.append(outside);
    let clicks = 0;
    outside.addEventListener("click", () => clicks++);
    const press = (target: typeof outside | typeof a.trigger, button = 0) =>
      target.dispatchEvent(
        new page.MouseEvent("pointerdown", { bubbles: true, cancelable: true, button }),
      );
    const click = (target: typeof outside | typeof a.trigger) =>
      target.dispatchEvent(new page.MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    const open = (target: typeof a.trigger) => target.getAttribute("aria-expanded") === "true";
    click(a.trigger);
    expect(open(a.trigger)).toBe(true);
    expect(press(a.trigger)).toBe(false);
    expect(open(a.trigger)).toBe(false);
    click(a.trigger);
    expect(open(a.trigger)).toBe(false);
    click(a.trigger);
    press(b.trigger);
    click(b.trigger);
    expect(open(a.trigger)).toBe(false);
    expect(open(b.trigger)).toBe(true);
    page.document.activeElement!.dispatchEvent(
      new page.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
    );
    expect(page.document.activeElement).toBe(b.trigger);
    for (const key of ["Enter", " "]) {
      b.trigger.dispatchEvent(new page.KeyboardEvent("keydown", { key, bubbles: true }));
      expect(open(b.trigger)).toBe(true);
      b.trigger.dispatchEvent(new page.KeyboardEvent("keydown", { key, bubbles: true }));
      expect(open(b.trigger)).toBe(false);
    }
    click(a.trigger);
    expect(press(outside)).toBe(false);
    click(outside);
    expect(clicks).toBe(0);
    expect(open(a.trigger)).toBe(false);
    press(outside);
    click(outside);
    expect(clicks).toBe(1);

    const secondary = (type: string, button: number) =>
      outside.dispatchEvent(new page.MouseEvent(type, { bubbles: true, cancelable: true, button }));
    click(a.trigger);
    expect(press(outside, 2)).toBe(false);
    expect(open(a.trigger)).toBe(false);
    expect(secondary("contextmenu", 2)).toBe(false);
    expect(secondary("auxclick", 2)).toBe(false);
    press(outside, 2);
    expect(secondary("contextmenu", 2)).toBe(true);

    click(a.trigger);
    expect(press(outside, 1)).toBe(false);
    expect(open(a.trigger)).toBe(false);
    expect(secondary("auxclick", 1)).toBe(false);
    a.dispose();
    b.dispose();
  } finally {
    for (const [key, descriptor] of Object.entries(previous)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
    void page.close();
  }
});
