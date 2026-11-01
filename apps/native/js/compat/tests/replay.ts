/** Re-execute the frozen scenario program, comparing public-boundary projections.
 * The fake kernel and DOM are test-only. No golden is used as a host response.
 */
import { readFile } from "node:fs/promises";
import { isDeepStrictEqual } from "node:util";
import { Window } from "happy-dom";
import {
  runScenario,
  scenarios,
} from "../../../../../tools/trace-recorder/scenarios";
import { validateTrace } from "../../../../../tools/trace-recorder/validate";
import { CompatRealm, EditGesture, chordOfEvent } from "../src";
import { copy } from "../src/mirror";
import type { Data } from "../src/types";
import { FakeHost } from "./fake-host";

function decode(value: any, objects = new Map<string, any>()): any {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map((v) => decode(v, objects));
  if (value.$ref) return objects.get(value.$ref);
  if (value.$token) return "token:" + value.$token;
  if (value.$type === "undefined") return undefined;
  if (value.$type === "promise") return { kind: "promise" };
  if (value.$type === "json-string")
    return JSON.stringify(decode(value.value, objects));
  if (value.$type === "error") return { kind: "error", message: value.message };
  if (value.$type === "key") return { ...value, target: undefined };
  if (value.$type === "array") {
    const result: any[] = [];
    if (value.$id) objects.set(value.$id, result);
    result.push(...value.values.map((v: any) => decode(v, objects)));
    return result;
  }
  const result: Data = {};
  if (value.$id) objects.set(value.$id, result);
  for (const [k, v] of Object.entries(value.props ?? value))
    result[k] = decode(v, objects);
  return result;
}
function project(document: Data): Data {
  return {
    name: document.name,
    revision: document.revision || 0,
    layers: document.layers.map((l: Data) => ({
      id: l.id,
      name: l.name,
      type: l.type,
    })),
  };
}
function outcome(value: any): any {
  if (value instanceof Promise || value?.kind === "promise")
    return { kind: "promise" };
  if (value instanceof Error || value?.kind === "error")
    return { kind: "error", message: value.message };
  if (
    typeof value === "string" &&
    value.startsWith("{") &&
    value.includes('"layers"')
  )
    return { kind: "snapshot", project: project(JSON.parse(value)) };
  if (value?.ok !== undefined)
    return {
      ok: value.ok,
      message: value.message,
      ...(value.data
        ? {
            data: {
              ...(Object.hasOwn(value.data, "result")
                ? { result: value.data.result }
                : {}),
              ...(Object.hasOwn(value.data, "revision")
                ? { revision: value.data.revision }
                : {}),
            },
          }
        : {}),
    };
  if (value && typeof value === "object")
    return value.name !== undefined ? { workspaceName: value.name } : value;
  // Generated external history entry ids are logical tokens, not authored strings.
  if (
    typeof value === "string" &&
    (value.startsWith("history-") || value.startsWith("token:"))
  )
    return { kind: "history-id" };
  return value;
}
interface Boundary {
  name: string;
  kind: string;
  value: any;
  project: Data;
  history: Data;
}
export interface ReplayResult {
  scenario: string;
  boundaries: number;
  matched: number;
  differences: string[];
  scope: string;
}
export async function replayScenario(name: string): Promise<ReplayResult> {
  const text = await readFile(
    new URL(
      `../../../../../tools/trace-recorder/golden/${name}.jsonl`,
      import.meta.url,
    ),
    "utf8",
  );
  validateTrace(text);
  const rows = text
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  const initial = decode(rows.find((r) => r.type === "state").data).state;
  const host = new FakeHost(initial.project, initial.workspace);
  const realm = new CompatRealm({
    id: "replay",
    generation: 1,
    host,
    initial: host.publication(),
  });
  const PM = realm.PM,
    actual: Boundary[] = [];
  let recording = false,
    nesting = 0;
  const state = () => ({
    project: project(PM.proj),
    history: {
      busy: !!host.pending,
      canUndo: host.index >= 0,
      canRedo: host.index < host.entries.length - 1,
    },
  });
  const capture = <T>(method: string, run: () => T): T => {
    const root = recording && nesting === 0;
    nesting++;
    let result: T,
      error: unknown,
      threw = false;
    try {
      result = run();
      return result;
    } catch (caught) {
      error = caught;
      threw = true;
      throw caught;
    } finally {
      nesting--;
      if (root)
        actual.push({
          name: method,
          kind: threw ? "throw" : "return",
          value: outcome(threw ? error : result!),
          ...state(),
        });
    }
  };
  const wrap = (prefix: string, api: Data): Data =>
    Object.fromEntries(
      Object.entries(api).map(([method, fn]) => [
        method,
        typeof fn === "function"
          ? (...args: any[]) =>
              capture(prefix + "." + method, () => fn(...args))
          : fn,
      ]),
    );
  const apis = {
    history: wrap("api.history", realm.history),
    edit: wrap("api.edit", realm.edit),
    workspace: wrap("api.workspace", realm.workspace),
  };
  PM.hist.clear = (() => {
    const clear = PM.hist.clear;
    return () => capture("history.clear", () => clear());
  })();
  PM.touch = (() => {
    const touch = PM.touch;
    return () => capture("PM.touch", () => touch());
  })();
  // A fixture reset is host-owned, rather than an extension direct-write admission.
  PM.mkProject = () => copy(initial.project);
  PM.replaceProject = (document: Data) => {
    host.state.document = copy(document);
    realm.receive(host.publication());
  };
  PM.WS.all = [PM.WS.current];
  const points: Data[] = [];
  const trace: Data = {
    apis,
    recorder: null,
    start: () => {
      recording = true;
    },
    finish: () => {
      recording = false;
      return "projected-replay";
    },
    point: (type: string, data: any) => {
      if (!recording) return;
      if (type.startsWith("callback.")) points.push({ type, data: copy(data) });
    },
    call: (method: string, _args: any[], run: () => unknown) =>
      capture(method, run),
    checkpoint: () => {},
    gesture: class extends EditGesture {
      begin(): void {
        capture("gesture.begin", () => super.begin());
      }
      write(value: any): void {
        capture("gesture.write", () => super.write(value));
      }
      commit(): void {
        capture("gesture.commit", () => super.commit());
      }
      cancel(): void {
        capture("gesture.cancel", () => super.cancel());
      }
      once(value: any): any {
        return capture("gesture.once", () => super.once(value));
      }
    },
  };
  const window = new Window({ url: "https://synthetic.invalid" });
  const target = globalThis as Data;
  const saved = new Map<string, PropertyDescriptor | undefined>();
  for (const [key, value] of Object.entries({
    window,
    document: window.document,
    KeyboardEvent: window.KeyboardEvent,
    Element: window.Element,
  })) {
    saved.set(key, Object.getOwnPropertyDescriptor(target, key));
    Object.defineProperty(target, key, {
      configurable: true,
      writable: true,
      value,
    });
  }
  (window as any).PM = PM;
  (window as any).__PMTrace = trace;
  const keyHandler = (event: any) =>
    capture("keymap.bubble", () => {
      const chord = chordOfEvent(event);
      if (!chord) return;
      const field = ["INPUT", "TEXTAREA", "SELECT"].includes(
        event.target?.tagName,
      );
      realm.keymap.bubble({
        chord,
        field,
        repeat: event.repeat,
        defaultPrevented: event.defaultPrevented,
        preventDefault: () => event.preventDefault(),
      });
    });
  window.addEventListener("keydown", keyHandler);
  if (name === "menu-equivalents") {
    realm.keymap.commands.register("builtin", {
      id: "contextUndo",
      run: () => realm.history.undo(),
    });
    realm.keymap.commands.register("builtin", {
      id: "contextRedo",
      run: () => realm.history.redo(),
    });
    realm.keymap.bind("builtin", { key: "cmd+z", command: "contextUndo" });
  }
  try {
    runScenario({ name, metadata: {} });
    if (name === "history-async") {
      await Promise.resolve();
      await Promise.resolve();
      trace.finish();
    }
    if (name === "menu-equivalents") {
      // AppKit/Electron delivery is outside the realm. Drive the recorded command ids.
      for (const command of ["contextUndo", "contextRedo"])
        capture("command.run", () => {
          return realm.keymap.run(command);
        });
      window.dispatchEvent(
        new window.KeyboardEvent("keydown", {
          key: "z",
          code: "KeyZ",
          metaKey: true,
          bubbles: true,
          cancelable: true,
        }),
      );
      trace.finish();
    }
    const expected: Boundary[] = rows
      .filter((r) => r.type === "call" && r.parent === null)
      .map((call) => {
        const end = rows.find(
          (r) =>
            r.call === call.call && (r.type === "return" || r.type === "throw"),
        );
        const state = decode(end.state),
          h = state.history;
        return {
          name: call.name,
          kind: end.type,
          value: outcome(decode(end.type === "throw" ? end.error : end.value)),
          project: project(state.project),
          history: {
            busy: !!h.pending,
            canUndo: h.index >= 0,
            canRedo: h.index < h.entries.length - 1,
          },
        };
      });
    const differences: string[] = [];
    const length = Math.max(actual.length, expected.length);
    let matched = 0;
    for (let i = 0; i < length; i++) {
      if (isDeepStrictEqual(actual[i], expected[i])) matched++;
      else
        differences.push(
          `boundary ${i + 1}: ${JSON.stringify({ expected: expected[i], actual: actual[i] })}`,
        );
    }
    const expectedPoints = rows
      .filter((r) => r.type.startsWith("callback."))
      .map((r) => ({ type: r.type, data: decode(r.data) }));
    if (!isDeepStrictEqual(points, expectedPoints))
      differences.push(
        "explicit callback order/value mismatch: " +
          JSON.stringify({ expected: expectedPoints, actual: points }),
      );
    return {
      scenario: name,
      boundaries: expected.length,
      matched,
      differences,
      scope: name.startsWith("keymap")
        ? "resolver returns/state/callbacks; DOM delivery is a test double"
        : name === "menu-equivalents"
          ? "realm command delivery; OS menu routing excluded"
          : "public returns/errors, callback order, name/layer identity/revision, history availability",
    };
  } finally {
    for (const [key, descriptor] of saved) {
      if (descriptor) Object.defineProperty(target, key, descriptor);
      else delete target[key];
    }
    await window.happyDOM.close();
  }
}
export { scenarios };
