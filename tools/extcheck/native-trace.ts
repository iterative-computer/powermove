// Runs inside the disposable V8 realm, mirroring reference.mjs step for step with
// the same recorder core. Native rows come from native calls; nothing is copied.
import { Recorder, type Row } from "../trace-recorder/core";
import { admitFixtureWorkspace } from "./fixture-aliases.mjs";

const root = globalThis as any;
let recorder: Recorder | null = null;
let scope = "";
const call = <T>(name: string, args: unknown[], fn: () => T): T => {
  if (!recorder) return fn();
  const previous = scope;
  scope = name;
  try {
    return recorder.call(name, args, fn);
  } finally {
    scope = previous;
  }
};
const point = (type: string, data?: unknown) => recorder?.point(type, data);
const history = () => root.__compat.realm.channel.call("history.traceState");

function instrument(realm: any) {
  const PM = realm.PM;
  const events = realm.kernel.events;
  const emit = events.emit.bind(events);
  events.emit = (event: string, payload: unknown) => {
    point("event.kernel", { event, payload });
    return emit(event, payload);
  };
  const legacy = PM.bus.emit;
  PM.bus.emit = (event: string, a: unknown, b: unknown) => {
    point("event.legacy", { event, args: [a, b] });
    return legacy(event, a, b);
  };
  const wrap = (owner: any, prefix: string, names: string[]) => {
    for (const name of names) {
      const fn = owner[name];
      owner[name] = (...args: unknown[]) =>
        call(`${prefix}.${name}`, args, () => fn(...args));
    }
  };
  const history = [
    "begin",
    "commit",
    "cancel",
    "do",
    "external",
    "selection",
    "undo",
    "redo",
  ];
  wrap(realm.history, "api.history", history);
  wrap(PM.hist, "history", [...history, "rollback", "undoIfTop", "clear"]);
  wrap(realm.edit, "edit", [
    "apply",
    "mutate",
    "begin",
    "dispatch",
    "commit",
    "cancel",
  ]);
  wrap(realm.workspace, "api.workspace", ["mutate"]);
  wrap(PM, "PM", ["touch"]);
  const uid = PM.uid;
  PM.uid = (...args: unknown[]) => {
    const value = uid(...args);
    recorder?.token(value);
    return value;
  };
}

const interactions: unknown[] = [];
let instrumented = false;
root.__extcheckDriver = {
  start(metadata: Row, clockStart: number, id: string) {
    root.__compatDriverId = id;
    const boot = root.__extcheckClock.align(clockStart);
    const realm = root.__compat.realm;
    if (!instrumented) instrument(realm);
    instrumented = true;
    const PM = realm.PM;
    recorder = new Recorder(
      () => ({
        project: PM.proj,
        selection: PM.sel,
        history: history(),
        ...(scope.includes("workspace") ? { workspace: PM.WS?.current } : {}),
      }),
      metadata,
    );
    const visit = (value: any, seen = new Set()) => {
      if (!value || typeof value !== "object" || seen.has(value)) return;
      seen.add(value);
      for (const [key, descriptor] of Object.entries(
        Object.getOwnPropertyDescriptors(value),
      )) {
        if (!("value" in descriptor)) continue;
        if (
          typeof descriptor.value === "string" &&
          (key === "id" || key === "compId")
        )
          recorder!.token(descriptor.value);
        else visit(descriptor.value, seen);
      }
    };
    visit(PM.proj);
    recorder.checkpoint("initial");
    root.__PMTrace = { recorder };
    admitFixtureWorkspace(root);
    return boot;
  },
  async step(index: number, step: any) {
    const realm = root.__compat.realm;
    const PM = realm.PM;
    const id = root.__compatDriverId;
    const loader = PM.Kernel.loader;
    point("driver.step", { index, step });
    let result: unknown = null;
    if (step.op === "reload") {
      await call("lifecycle.reload", [id], () => loader.reload(id));
      result = { active: loader.activeIds().includes(id) };
    } else if (step.op === "dispose") {
      await call("lifecycle.dispose", [id], () => loader.disable(id));
      await loader.whenIdle();
      result = { active: loader.activeIds().includes(id) };
    } else if (step.op === "seek") {
      PM.pause();
      call("PM.setTime", [step.time], () => PM.setTime(step.time));
      result = { time: PM.time };
    } else if (step.op === "panel") {
      const api = root.__compat.hostAPI();
      call("panels.open", [step.id], () => api.panels.open(step.id));
      result = { open: api.panels.isOpen(step.id) };
    } else if (step.op === "command") {
      const api = root.__compat.hostAPI();
      result = await call(
        "commands.run",
        [step.id, ...(step.args ?? [])],
        () => api.commands.run(step.id, ...(step.args ?? [])),
      );
    } else if (step.op === "advance") {
      await root.__extcheckClock.advance(step.ms);
    } else {
      throw Error("unsupported-script-step");
    }
    interactions.push({ op: step.op, result });
    point("driver.result", { index, result });
    recorder?.checkpoint("step-" + index);
    return true;
  },
  finish() {
    const trace = recorder!.finish();
    recorder = null;
    return { trace, interactions };
  },
};
