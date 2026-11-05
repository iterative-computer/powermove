// Realm-only Canvas binding. The page receives immutable native PNG resources,
// never extension draw calls or a second JS renderer.
globalThis.__installNativeCanvas = function (owner) {
  const document = owner.dom?.document;
  if (!document) return;
  const prototype = Object.getPrototypeOf(document.createElement("canvas"));
  const records = new Map();
  const native = (node) => {
    let record = records.get(node);
    if (!record) {
      record = {
        canvas: owner.realm.createCanvas(node.width, node.height),
        context: null,
      };
      records.set(node, record);
    }
    return record;
  };
  const dimension = (node, name) => {
    const text = node.getAttribute(name);
    // HTML non-negative integer parsing, including leading whitespace and '+'.
    const match = text?.match(/^\s*\+?(\d+)/);
    const value = match ? Number(match[1]) : NaN;
    return Number.isSafeInteger(value) && value <= 0x7fffffff
      ? value
      : name === "width"
        ? 300
        : 150;
  };
  for (const name of ["width", "height"]) {
    Object.defineProperty(prototype, name, {
      configurable: true,
      get() {
        return dimension(this, name);
      },
      set(value) {
        this.setAttribute(name, String(Number(value) >>> 0));
      },
    });
  }
  const set = prototype.setAttribute;
  const remove = prototype.removeAttribute;
  prototype.setAttribute = function (name, value) {
    const result = set.call(this, name, value);
    if (
      String(name).toLowerCase() === "width" ||
      String(name).toLowerCase() === "height"
    ) {
      const record = records.get(this);
      if (record)
        record.canvas[String(name).toLowerCase()] =
          this[String(name).toLowerCase()];
    }
    return result;
  };
  prototype.removeAttribute = function (name) {
    const result = remove.call(this, name);
    if (
      String(name).toLowerCase() === "width" ||
      String(name).toLowerCase() === "height"
    ) {
      const record = records.get(this);
      if (record)
        record.canvas[String(name).toLowerCase()] =
          this[String(name).toLowerCase()];
    }
    return result;
  };
  prototype.getContext = function (kind, options) {
    if (kind !== "2d") return null;
    const record = native(this);
    if (record.context) return record.context;
    const context = record.canvas.getContext(kind, options);
    const node = this,
      methods = new Map();
    record.context = new Proxy(context, {
      get(target, key) {
        if (key === "canvas") return node;
        const value = Reflect.get(target, key);
        if (typeof value !== "function") return value;
        if (!methods.has(key))
          methods.set(key, (...args) => {
            if (key === "drawImage" && records.has(args[0]))
              args[0] = native(args[0]).canvas;
            return value(...args);
          });
        return methods.get(key);
      },
    });
    return record.context;
  };
  prototype.toDataURL = function (...args) {
    return native(this).canvas.toDataURL(...args);
  };
  owner.flushCanvases = () => {
    const bindings = [];
    for (const [node, record] of records) {
      if (!node.isConnected) continue;
      bindings.push({
        surface: record.canvas.surface,
        node: owner.dom.id(node),
      });
    }
    owner.dom.flush();
    runtime.engine("recorder.bind", { bindings });
  };
};
