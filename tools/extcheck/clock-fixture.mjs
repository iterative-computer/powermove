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
  const next = () => {
    random += 0x6d2b79f5;
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
  Object.defineProperty(root.performance, "now", {
    configurable: true,
    value: () => now,
  });
  Object.defineProperty(root.performance, "timeOrigin", {
    configurable: true,
    value: epoch,
  });
  if (root.crypto)
    Object.defineProperty(root.crypto, "randomUUID", {
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
