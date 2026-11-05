/** Script-owned time/entropy for disposable comparison realms only. No data is
 * rewritten and no trace row or pixel is discarded. Both hosts use this code. */
export function installClockFixture(root = globalThis, seed = 0x504d5032) {
  if (root.__extcheckClock) throw Error("Comparison clock already installed");
  const NativeDate = root.Date;
  const realFrame = root.requestAnimationFrame?.bind(root);
  const realCancelFrame = root.cancelAnimationFrame?.bind(root);
  const realClearTimeout = root.clearTimeout?.bind(root);
  const realClearInterval = root.clearInterval?.bind(root);
  // Avoid collisions with native timers that were created during app boot.
  let now = 60_000,
    random = seed >>> 0,
    serial = 1_000_000_000;
  const epoch = 1_600_000_000_000;
  const timers = new Map(),
    frames = new Map();
  let draws = 0;
  const next = () => {
    draws++;
    random = (random + 0x6d2b79f5) >>> 0;
    let v = random;
    v = Math.imul(v ^ (v >>> 15), v | 1);
    v ^= v + Math.imul(v ^ (v >>> 7), v | 61);
    return ((v ^ (v >>> 14)) >>> 0) / 4294967296;
  };
  function ClockDate(...args) {
    if (!new.target) return new NativeDate(epoch + now).toString();
    return Reflect.construct(
      NativeDate,
      args.length ? args : [epoch + now],
      new.target,
    );
  }
  Object.setPrototypeOf(ClockDate, NativeDate);
  ClockDate.prototype = NativeDate.prototype;
  Object.defineProperty(ClockDate, "name", { value: "Date" });
  Object.defineProperty(ClockDate, "length", { value: 7 });
  Object.defineProperty(ClockDate.prototype, "constructor", {
    configurable: true,
    writable: true,
    value: ClockDate,
  });
  ClockDate.now = () => epoch + now;
  root.Date = ClockDate;
  root.Math.random = next;
  // V8 freezes its snapshot performance object; keep it as the prototype.
  const performance = Object.isExtensible(root.performance)
    ? root.performance
    : (root.performance = Object.create(root.performance));
  Object.defineProperty(performance, "now", {
    configurable: true,
    value: () => now,
  });
  Object.defineProperty(performance, "timeOrigin", {
    configurable: true,
    value: epoch,
  });
  const crypto = root.crypto ?? (root.crypto = {});
  Object.defineProperty(crypto, "getRandomValues", {
    configurable: true,
    value(array) {
      if (
        !ArrayBuffer.isView(array) ||
        array instanceof DataView ||
        !/^(?:Uint|Int)(?:8|16|32|64)(?:Clamped)?Array$|^Big(?:Uint|Int)64Array$/.test(
          array.constructor.name,
        )
      )
        throw new TypeError("Expected an integer typed array");
      if (array.byteLength > 65536)
        throw new RangeError("Entropy fixture quota");
      const bytes = new Uint8Array(
        array.buffer,
        array.byteOffset,
        array.byteLength,
      );
      for (let i = 0; i < bytes.length; i++)
        bytes[i] = Math.floor(next() * 256);
      return array;
    },
  });
  Object.defineProperty(crypto, "randomUUID", {
    configurable: true,
    value: () =>
      "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) =>
        (
          Number(c) ^
          ((Math.floor(next() * 16) & 15) >> (Number(c) / 4))
        ).toString(16),
      ),
  });
  const schedule = (fn, delay, args, repeat) => {
    if (typeof fn !== "function")
      throw TypeError("Comparison timers require a function");
    const id = ++serial,
      interval = Math.max(repeat ? 1 : 0, Number(delay) || 0);
    timers.set(id, {
      fn,
      args,
      due: now + interval,
      interval: repeat ? interval : 0,
    });
    return id;
  };
  root.setTimeout = (fn, delay = 0, ...args) =>
    schedule(fn, delay, args, false);
  root.setInterval = (fn, delay = 0, ...args) =>
    schedule(fn, delay, args, true);
  root.clearTimeout = (id) => {
    if (!timers.delete(id)) realClearTimeout?.(id);
  };
  root.clearInterval = (id) => {
    if (!timers.delete(id)) realClearInterval?.(id);
  };
  root.requestAnimationFrame = (fn) => {
    const id = ++serial;
    frames.set(id, fn);
    return id;
  };
  root.cancelAnimationFrame = (id) => {
    if (!frames.delete(id)) realCancelFrame?.(id);
  };
  root.__extcheckClock = {
    task: (fn) => fn(),
    snapshot: () => ({ now, draws, timers: timers.size, frames: frames.size }),
    get now() {
      return now;
    },
    async advance(ms = 16) {
      if (!Number.isFinite(ms) || ms < 0)
        throw RangeError("Invalid comparison time");
      const target = now + ms;
      let calls = 0;
      while (true) {
        const entry = [...timers]
          .filter(([, t]) => t.due <= target)
          .sort((a, b) => a[1].due - b[1].due || a[0] - b[0])[0];
        if (!entry) break;
        if (++calls > 10_000) throw Error("Comparison timer limit");
        const [id, timer] = entry;
        now = timer.due;
        if (timer.interval) timer.due += timer.interval;
        else timers.delete(id);
        this.task(() => timer.fn.apply(root, timer.args));
        await Promise.resolve();
      }
      now = target;
      const pending = [...frames];
      for (const [id, fn] of pending) {
        if (!frames.delete(id)) continue;
        this.task(() => fn.call(root, now));
        await Promise.resolve();
      }
    },
    async settle(frames = 60) {
      for (let i = 0; i < frames; i++) await this.advance(16);
    },
    nativePaint() {
      return realFrame
        ? new Promise((resolve) => realFrame(() => realFrame(resolve)))
        : Promise.resolve();
    },
  };
  return root.__extcheckClock;
}
