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
            if (key === "drawImage" || key === "createPattern")
              args[0] = drawable(args[0]);
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
  // Images decode natively (`image.load`): ImageIO for rasters, resvg for SVG.
  // A realm-local source (blob:, data:) never reaches the page as a URL; the
  // page receives the image's bytes as a resource, like a canvas's pixels.
  const imagePrototype = Object.getPrototypeOf(document.createElement("img"));
  // Records are weak: a dropped `new Image()` releases its pixels on collection.
  const images = new WeakMap(),
    shown = new Set();
  const finalized = new FinalizationRegistry((surface) => {
    try {
      runtime.engine("recorder.release", { args: [surface] });
    } catch (_) {}
  });
  const release = (image) => {
    if (!image || !finalized.unregister(image)) return;
    try {
      runtime.engine("recorder.release", { args: [image.surface] });
    } catch (_) {}
  };
  const imageRecord = (node) => {
    let record = images.get(node);
    if (!record) {
      record = { state: "empty", native: null, token: 0, handlers: {} };
      images.set(node, record);
    }
    return record;
  };
  const decodeBytes = (bytes, type) => {
    let info;
    try {
      info = runtime.bytes("image.load", bytes, { type: type ?? "" });
    } catch (_) {
      throw new DOMException("The source image cannot be decoded.", "EncodingError");
    }
    const image = owner.realm.createImage(
      info.surface,
      info.width,
      info.height,
      info.type === "image/svg+xml",
    );
    finalized.register(image, info.surface, image);
    return image;
  };
  const sourceBytes = async (src) => {
    const local = runtime.local(src);
    if (local) return local;
    if (local === undefined || !/^https?:/i.test(src))
      throw new DOMException("The source image cannot be decoded.", "EncodingError");
    const response = await fetch(src);
    if (!response.ok)
      throw new DOMException("The source image cannot be loaded.", "EncodingError");
    return {
      bytes: new Uint8Array(await response.arrayBuffer()),
      type: (response.headers.get("content-type") ?? "").split(";")[0].trim(),
    };
  };
  const fire = (node, record, type) => {
    const event = new Event(type);
    try {
      record.handlers[type]?.call(node, event);
    } catch (error) {
      reportError?.(error);
    }
    node.dispatchEvent(event);
  };
  const start = (node, record) => {
    const token = ++record.token;
    const src = record.src;
    release(record.native);
    record.native = null;
    record.bytes = null;
    record.state = "loading";
    record.loaded = (async () => {
      if (!src) throw new DOMException("The image has no source.", "EncodingError");
      const { bytes, type } = await sourceBytes(src);
      if (token !== record.token) return;
      record.native = decodeBytes(bytes, type);
      record.bytes = bytes;
    })().then(
      () => {
        if (token !== record.token) return;
        record.state = "ok";
        if (record.local) {
          if (!record.shown) shown.add((record.shown = new WeakRef(node)));
          if (node.isConnected) owner.dom.mark?.(node);
        }
        fire(node, record, "load");
      },
      (error) => {
        if (token !== record.token) return;
        record.state = "broken";
        fire(node, record, "error");
        throw error;
      },
    );
    record.loaded.catch(() => {});
  };
  const setSource = (node, value) => {
    const record = imageRecord(node);
    record.src = value == null ? "" : String(value);
    record.local = /^(blob|data):/i.test(record.src);
    // A remote image on the page loads there; the realm fetches it only when
    // the extension reads it (new Image(), decode(), naturalWidth).
    if (record.local || !node.isConnected || record.wanted) start(node, record);
    else {
      record.token++;
      release(record.native);
      record.native = null;
      record.state = "remote";
    }
  };
  const want = (node) => {
    const record = imageRecord(node);
    record.wanted = true;
    if (record.state === "remote") start(node, record);
    return record;
  };
  const setImageAttribute = imagePrototype.setAttribute;
  const removeImageAttribute = imagePrototype.removeAttribute;
  imagePrototype.setAttribute = function (name, value) {
    const result = setImageAttribute.call(this, name, value);
    if (String(name).toLowerCase() === "src") setSource(this, value);
    return result;
  };
  imagePrototype.removeAttribute = function (name) {
    const result = removeImageAttribute.call(this, name);
    if (String(name).toLowerCase() === "src") setSource(this, "");
    return result;
  };
  const natural = (node, side) => {
    const record = want(node);
    return record.state === "ok" ? record.native[side] : 0;
  };
  Object.defineProperties(imagePrototype, {
    src: {
      configurable: true,
      get() {
        return this.getAttribute("src") ?? "";
      },
      set(value) {
        this.setAttribute("src", String(value));
      },
    },
    currentSrc: {
      configurable: true,
      get() {
        return images.get(this)?.state === "ok" ? images.get(this).src : "";
      },
    },
    naturalWidth: {
      configurable: true,
      get() {
        return natural(this, "width");
      },
    },
    naturalHeight: {
      configurable: true,
      get() {
        return natural(this, "height");
      },
    },
    complete: {
      configurable: true,
      get() {
        const state = images.get(this)?.state ?? "empty";
        return state !== "loading" && state !== "remote";
      },
    },
    width: {
      configurable: true,
      get() {
        const value = Number.parseInt(this.getAttribute("width"), 10);
        return Number.isFinite(value) ? value : this.naturalWidth;
      },
      set(value) {
        this.setAttribute("width", String(Number(value) >>> 0));
      },
    },
    height: {
      configurable: true,
      get() {
        const value = Number.parseInt(this.getAttribute("height"), 10);
        return Number.isFinite(value) ? value : this.naturalHeight;
      },
      set(value) {
        this.setAttribute("height", String(Number(value) >>> 0));
      },
    },
    decode: {
      configurable: true,
      value() {
        const record = want(this);
        if (record.state === "empty")
          return Promise.reject(
            new DOMException("The image has no source.", "EncodingError"),
          );
        return record.state === "ok" ? Promise.resolve() : record.loaded;
      },
    },
  });
  for (const type of ["load", "error"])
    Object.defineProperty(imagePrototype, "on" + type, {
      configurable: true,
      get() {
        return images.get(this)?.handlers[type] ?? null;
      },
      set(fn) {
        imageRecord(this).handlers[type] = typeof fn === "function" ? fn : null;
      },
    });
  const reportError = globalThis.reportError;
  const ImageConstructor = function Image(width, height) {
    if (!new.target) throw new TypeError("Constructor Image requires 'new'");
    const node = document.createElement("img");
    if (width !== undefined) node.width = width;
    if (height !== undefined) node.height = height;
    return node;
  };
  ImageConstructor.prototype = imagePrototype;
  globalThis.Image = ImageConstructor;
  class ImageBitmap {
    #image;
    #bytes;
    constructor(image, bytes) {
      this.#image = image;
      this.#bytes = bytes;
    }
    static bytes(bitmap) {
      ImageBitmap.source(bitmap);
      return bitmap.#bytes;
    }
    get width() {
      return this.#image ? this.#image.width : 0;
    }
    get height() {
      return this.#image ? this.#image.height : 0;
    }
    close() {
      release(this.#image);
      this.#image = null;
      this.#bytes = null;
    }
    static source(bitmap) {
      if (!bitmap.#image)
        throw new DOMException("The image source is detached", "InvalidStateError");
      return bitmap.#image;
    }
  }
  globalThis.ImageBitmap = ImageBitmap;
  const bitmap = (bytes, type) => new ImageBitmap(decodeBytes(bytes, type), bytes);
  globalThis.createImageBitmap = async (source) => {
    if (source instanceof Blob)
      return bitmap(new Uint8Array(await source.arrayBuffer()), source.type);
    if (source && Object.getPrototypeOf(source) === imagePrototype) {
      await source.decode();
      return bitmap(imageRecord(source).bytes, "");
    }
    if (source instanceof ImageBitmap) return bitmap(ImageBitmap.bytes(source), "");
    if (source && records.has(source)) {
      const local = runtime.local(source.toDataURL());
      return bitmap(local.bytes, local.type);
    }
    throw new TypeError(
      "createImageBitmap: the source is not an image, Blob, ImageBitmap or canvas",
    );
  };
  // What drawImage and createPattern receive: the native source for a DOM
  // canvas, an image or a bitmap. A broken image throws, as in a browser.
  const drawable = (source) => {
    if (records.has(source)) return native(source).canvas;
    if (source instanceof ImageBitmap) return ImageBitmap.source(source);
    if (source && Object.getPrototypeOf(source) === imagePrototype) {
      const record = want(source);
      if (record.state === "broken")
        throw new DOMException(
          "The HTMLImageElement provided is in the 'broken' state.",
          "InvalidStateError",
        );
      if (record.state !== "ok")
        throw new DOMException("The image is not decoded yet.", "InvalidStateError");
      return record.native;
    }
    return source;
  };
  owner.pageAttribute = (node, name, value) =>
    name === "src" && images.get(node)?.local ? null : value;
  owner.flushCanvases = () => {
    const bindings = [];
    for (const [node, record] of records) {
      if (!node.isConnected) continue;
      bindings.push({
        surface: record.canvas.surface,
        node: owner.dom.id(node),
      });
    }
    for (const ref of shown) {
      const node = ref.deref(),
        record = node && images.get(node);
      if (!record?.local || record.state !== "ok") {
        shown.delete(ref);
        if (record) record.shown = null;
        continue;
      }
      if (node.isConnected)
        bindings.push({ surface: record.native.surface, node: owner.dom.id(node) });
    }
    owner.dom.flush();
    runtime.engine("recorder.bind", { bindings });
  };
};
