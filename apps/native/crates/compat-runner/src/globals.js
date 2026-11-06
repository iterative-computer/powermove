// Browser globals some shared modules read while loading, before the remote
// DOM exists. Answers come from the host's appearance facts once it is attached.
globalThis.window ??= globalThis;
globalThis.devicePixelRatio ??= 2;
globalThis.__mediaFacts = {
  "prefers-reduced-motion": false,
  "prefers-color-scheme": "light",
};
globalThis.matchMedia ??= (query) => {
  const list = new EventTarget();
  const [, name, value] = /\(\s*([\w-]+)\s*:\s*([\w-]+)\s*\)/.exec(query) ?? [];
  const answer = () =>
    name === "prefers-reduced-motion"
      ? __mediaFacts[name] === (value === "reduce")
      : __mediaFacts[name] === value;
  Object.defineProperties(list, {
    media: { value: String(query) },
    matches: { get: answer },
  });
  list.addListener = (fn) => list.addEventListener("change", fn);
  list.removeListener = (fn) => list.removeEventListener("change", fn);
  return list;
};
