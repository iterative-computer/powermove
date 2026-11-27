// Realm-only Canvas binding. The page receives immutable native PNG resources,
// never extension draw calls or a second JS renderer.
globalThis.__installNativeCanvas = function (owner) {
  const document = owner.dom?.document;
  if (!document) return;
  const prototype = Object.getPrototypeOf(document.createElement("canvas"));
  // Weak: a canvas element an extension drops goes with its native surface.
  const records = new WeakMap(),
    bound = new Set();
  const isCanvas = (node) => !!node && Object.getPrototypeOf(node) === prototype;
  const native = (node) => {
    let record = records.get(node);
    if (!record) {
      record = {
        canvas: owner.realm.createCanvas(node.width, node.height),
        context: null,
      };
      records.set(node, record);
      bound.add(new WeakRef(node));
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
            // A video with no frame yet draws nothing, as in a browser.
            if (args[0] === NO_FRAME) return key === "drawImage" ? undefined : null;
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
  // createImageBitmap(source[, sx, sy, sw, sh][, options]): crop and resize
  // draw through a recorder canvas, as a browser resamples them.
  const bitmapOf = async (source) => {
    if (source instanceof Blob)
      return bitmap(new Uint8Array(await source.arrayBuffer()), source.type);
    if (source && Object.getPrototypeOf(source) === imagePrototype) {
      await source.decode();
      return bitmap(imageRecord(source).bytes, "");
    }
    if (source instanceof ImageBitmap) return bitmap(ImageBitmap.bytes(source), "");
    const canvas = isCanvas(source)
      ? native(source).canvas
      : typeof source?.data?.length === "number" &&
          Number.isInteger(source.width) &&
          Number.isInteger(source.height)
        ? (() => {
            // ImageData: its pixels exactly, through a scratch canvas.
            const scratch = owner.realm.createCanvas(source.width, source.height);
            const { width, height } = source;
            scratch
              .getContext("2d")
              .putImageData({ width, height, data: Array.from(source.data) }, 0, 0);
            return scratch;
          })()
        : null;
    if (!canvas)
      throw new TypeError(
        "createImageBitmap: the source is not an image, Blob, ImageBitmap, ImageData or canvas",
      );
    if (!canvas.width || !canvas.height)
      throw new DOMException("The source has no pixels.", "InvalidStateError");
    const local = runtime.local(canvas.toDataURL());
    return bitmap(local.bytes, local.type);
  };
  globalThis.createImageBitmap = async (source, ...rest) => {
    const crop = rest.length >= 4 ? rest.slice(0, 4).map(Number) : null;
    const options = (crop ? rest[4] : rest[0]) ?? {};
    if (crop && (!crop[2] || !crop[3]))
      throw new RangeError("createImageBitmap: the crop is empty");
    const base = await bitmapOf(source);
    const resize = options.resizeWidth !== undefined || options.resizeHeight !== undefined;
    if (!crop && !resize) return base;
    const [sx, sy, sw, sh] = crop ?? [0, 0, base.width, base.height];
    const ratio = Math.abs(sw) / Math.abs(sh);
    const width = Math.round(
      options.resizeWidth ??
        (options.resizeHeight !== undefined ? options.resizeHeight * ratio : Math.abs(sw)),
    );
    const height = Math.round(
      options.resizeHeight ??
        (options.resizeWidth !== undefined ? options.resizeWidth / ratio : Math.abs(sh)),
    );
    if (!(width > 0 && height > 0))
      throw new DOMException("The bitmap size is invalid.", "InvalidStateError");
    const scratch = owner.realm.createCanvas(width, height);
    try {
      const context = scratch.getContext("2d");
      context.imageSmoothingEnabled = options.resizeQuality !== "pixelated";
      context.drawImage(ImageBitmap.source(base), sx, sy, sw, sh, 0, 0, width, height);
      const local = runtime.local(scratch.toDataURL());
      return bitmap(local.bytes, local.type);
    } finally {
      base.close();
      scratch.dispose();
    }
  };
  // What drawImage and createPattern receive: the native source for a DOM
  // canvas, an image or a bitmap. A broken image throws, as in a browser.
  const drawable = (source) => {
    if (isCanvas(source)) return native(source).canvas;
    if (source && Object.getPrototypeOf(source) === videoPrototype)
      return videos.get(source)?.frame ?? NO_FRAME;
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
  // `<video>`: its file opened natively for its frames (`video.open`,
  // `video.frame`), so an extension reads a clip's duration and size, seeks
  // and draws the frame it is on. Playing it is the page's own player.
  // The DOM gives `<video>` the generic element prototype: video nodes get
  // one of their own on it, so nothing here touches any other element.
  const genericPrototype = Object.getPrototypeOf(document.createElement("video"));
  const videoPrototype = Object.create(genericPrototype);
  const asVideo = (node, name) => {
    if (String(name).toLowerCase() === "video" && Object.getPrototypeOf(node) === genericPrototype)
      Object.setPrototypeOf(node, videoPrototype);
    return node;
  };
  const createElement = document.createElement;
  document.createElement = function (name, options) {
    return asVideo(createElement.call(this, name, options), name);
  };
  const createElementNS = document.createElementNS;
  document.createElementNS = function (namespace, name, options) {
    return asVideo(createElementNS.call(this, namespace, name, options), name);
  };
  /** What a `<video>` with no frame yet draws: nothing. */
  const NO_FRAME = Symbol("no video frame");
  const videos = new WeakMap();
  const MEDIA_EVENTS = ["loadstart", "durationchange", "loadedmetadata", "loadeddata", "canplay",
    "canplaythrough", "seeking", "seeked", "timeupdate", "play", "playing", "pause", "ended", "error"];
  const videoRecord = (node) => {
    let record = videos.get(node);
    if (!record) {
      record = { src: "", token: 0, id: null, staged: null, duration: NaN, width: 0, height: 0,
        time: 0, frame: null, seeking: false, ready: 0, error: null, handlers: {}, muted: false };
      videos.set(node, record);
    }
    return record;
  };
  const videoFired = (node, record, type) => {
    const event = new Event(type);
    try {
      record.handlers[type]?.call(node, event);
    } catch (error) {
      reportError?.(error);
    }
    node.dispatchEvent(event);
  };
  const closeVideo = (record) => {
    if (record.id) {
      try {
        runtime.engine("video.close", { video: record.id });
      } catch (_) {}
    }
    // Bytes staged for it (a downloaded clip): the host lets the file go.
    if (record.staged) {
      try {
        runtime.engine("file.release", { token: record.staged });
      } catch (_) {}
    }
    Object.assign(record, { id: null, staged: null, frame: null, duration: NaN, width: 0,
      height: 0, time: 0, seeking: false, ready: 0, error: null });
  };
  // The file a source names: a held file (`PM.MediaStore.get`) as it is,
  // anything else staged as one.
  const videoFile = async (src, record) => {
    const held = runtime.urlFileToken(src);
    if (held) return held;
    let local = runtime.local(src);
    if (local === null && /^https?:/i.test(src)) {
      const response = await fetch(src);
      if (!response.ok) throw new DOMException("The video cannot be loaded.", "NotSupportedError");
      local = { bytes: new Uint8Array(await response.arrayBuffer()) };
    }
    if (!local) throw new DOMException("The video cannot be loaded.", "NotSupportedError");
    // The decoder knows a container by its file's extension: from the type,
    // else from the bytes (QuickTime's `ftypqt`, else MP4).
    const type = String(local.type ?? "").toLowerCase();
    const brand = String.fromCharCode(...(local.bytes?.subarray?.(4, 10) ?? []));
    const extension = type.includes("quicktime") || brand === "ftypqt" ? "mov"
      : type.includes("webm") ? "webm" : type.includes("x-m4v") ? "m4v" : "mp4";
    record.staged = runtime.bytes("assets.stage", local.bytes, { name: `video.${extension}` }).token;
    return record.staged;
  };
  const showFrame = (record, at) => {
    const reply = runtime.engine("video.frame", { video: record.id, time: at });
    const image = owner.realm.createImage(reply.surface, reply.width, reply.height, false);
    finalized.register(image, reply.surface, image);
    record.frame = image;
    record.time = at;
  };
  const loadVideo = (node) => {
    const record = videoRecord(node);
    closeVideo(record);
    const token = ++record.token;
    const src = record.src;
    if (!src) return;
    queueMicrotask(() => token === record.token && videoFired(node, record, "loadstart"));
    (async () => {
      const file = await videoFile(src, record);
      if (token !== record.token) return;
      const info = runtime.engine("video.open", { token: String(file) });
      if (token !== record.token) {
        runtime.engine("video.close", { video: info.video });
        return;
      }
      Object.assign(record, { id: info.video, duration: info.duration, width: info.width,
        height: info.height, ready: 1 });
      videoFired(node, record, "durationchange");
      videoFired(node, record, "loadedmetadata");
      showFrame(record, Math.min(record.time, record.duration));
      record.ready = 4;
      for (const type of ["loadeddata", "canplay", "canplaythrough"]) videoFired(node, record, type);
    })().catch((error) => {
      if (token !== record.token) return;
      record.error = { code: 4, message: String(error?.message ?? error) };
      videoFired(node, record, "error");
    });
  };
  const seekVideo = (node, at) => {
    const record = videoRecord(node);
    const time = Math.max(0, Number(at) || 0);
    if (record.ready < 1) {
      record.time = time;
      return;
    }
    const token = record.token;
    record.seeking = true;
    record.time = Math.min(time, record.duration);
    videoFired(node, record, "seeking");
    setTimeout(() => {
      if (token !== record.token) return;
      try {
        showFrame(record, record.time);
      } catch (error) {
        record.error = { code: 3, message: String(error?.message ?? error) };
        record.seeking = false;
        videoFired(node, record, "error");
        return;
      }
      record.seeking = false;
      videoFired(node, record, "timeupdate");
      videoFired(node, record, "seeked");
    }, 0);
  };
  const setVideoAttribute = videoPrototype.setAttribute;
  const removeVideoAttribute = videoPrototype.removeAttribute;
  videoPrototype.setAttribute = function (name, value) {
    const result = setVideoAttribute.call(this, name, value);
    if (String(name).toLowerCase() === "src") {
      videoRecord(this).src = value == null ? "" : String(value);
      loadVideo(this);
    }
    return result;
  };
  videoPrototype.removeAttribute = function (name) {
    const result = removeVideoAttribute.call(this, name);
    if (String(name).toLowerCase() === "src") {
      videoRecord(this).src = "";
      loadVideo(this);
    }
    return result;
  };
  const reflect = (name, attribute = name.toLowerCase()) => ({
    configurable: true,
    get() {
      return this.hasAttribute(attribute);
    },
    set(value) {
      if (value) this.setAttribute(attribute, "");
      else this.removeAttribute(attribute);
    },
  });
  Object.defineProperties(videoPrototype, {
    src: {
      configurable: true,
      get() {
        return this.getAttribute("src") ?? "";
      },
      set(value) {
        this.setAttribute("src", String(value));
      },
    },
    currentSrc: { configurable: true, get() { return videos.get(this)?.ready ? videos.get(this).src : ""; } },
    duration: { configurable: true, get() { return videos.get(this)?.duration ?? NaN; } },
    videoWidth: { configurable: true, get() { return videos.get(this)?.width ?? 0; } },
    videoHeight: { configurable: true, get() { return videos.get(this)?.height ?? 0; } },
    readyState: { configurable: true, get() { return videos.get(this)?.ready ?? 0; } },
    seeking: { configurable: true, get() { return videos.get(this)?.seeking ?? false; } },
    error: { configurable: true, get() { return videos.get(this)?.error ?? null; } },
    paused: { configurable: true, get() { return true; } },
    ended: { configurable: true, get() { return false; } },
    currentTime: {
      configurable: true,
      get() {
        return videos.get(this)?.time ?? 0;
      },
      set(value) {
        seekVideo(this, value);
      },
    },
    muted: {
      configurable: true,
      get() {
        return videoRecord(this).muted;
      },
      set(value) {
        videoRecord(this).muted = !!value;
      },
    },
    controls: reflect("controls"),
    loop: reflect("loop"),
    autoplay: reflect("autoplay"),
    playsInline: reflect("playsInline", "playsinline"),
    preload: {
      configurable: true,
      get() {
        return this.getAttribute("preload") ?? "auto";
      },
      set(value) {
        this.setAttribute("preload", String(value));
      },
    },
    load: {
      configurable: true,
      value() {
        loadVideo(this);
      },
    },
    // The realm reads frames; playing is the page player's (its controls).
    play: {
      configurable: true,
      value() {
        return Promise.reject(
          new DOMException("Playback starts from the player's controls.", "NotAllowedError"),
        );
      },
    },
    pause: { configurable: true, value() {} },
  });
  for (const type of MEDIA_EVENTS)
    Object.defineProperty(videoPrototype, "on" + type, {
      configurable: true,
      get() {
        return videos.get(this)?.handlers[type] ?? null;
      },
      set(fn) {
        videoRecord(this).handlers[type] = typeof fn === "function" ? fn : null;
      },
    });
  // The native surface behind a canvas an override drew: a DOM canvas or a
  // realm canvas (`RecorderCanvas`).
  owner.nativeSurfaceOf = (canvas) =>
    isCanvas(canvas) ? native(canvas).canvas.surface : canvas?.surface;
  // A realm-local source (an image's, a video's) never reaches the page as a URL.
  owner.pageAttribute = (node, name, value) =>
    name === "src" &&
    (images.get(node)?.local || (videos.has(node) && /^(blob|data):/i.test(String(value))))
      ? null
      : value;
  owner.flushCanvases = () => {
    const bindings = [];
    for (const ref of bound) {
      const node = ref.deref(),
        record = node && records.get(node);
      if (!record) {
        bound.delete(ref);
        continue;
      }
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
