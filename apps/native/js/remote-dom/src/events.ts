/** Standards-oriented dispatch for the mirror. LinkeDOM's bubble-only dispatch
 * cannot preserve capture/currentTarget/passive semantics of v1–3 panels. */
export class RemoteEvent {
  static NONE = 0;
  static CAPTURING_PHASE = 1;
  static AT_TARGET = 2;
  static BUBBLING_PHASE = 3;
  NONE = 0;
  CAPTURING_PHASE = 1;
  AT_TARGET = 2;
  BUBBLING_PHASE = 3;
  type: string;
  bubbles: boolean;
  cancelable: boolean;
  composed: boolean;
  target: any = null;
  currentTarget: any = null;
  eventPhase = 0;
  defaultPrevented = false;
  cancelBubble = false;
  isTrusted = false;
  timeStamp = 0;
  immediate = false;
  passive = false;
  path: any[] = [];
  dispatching = false;
  constructor(type: string, init: EventInit & Record<string, unknown> = {}) {
    this.type = type;
    this.bubbles = !!init.bubbles;
    this.cancelable = !!init.cancelable;
    this.composed = !!init.composed;
    for (const [key, value] of Object.entries(init)) {
      if (!(key in this)) Object.defineProperty(this, key, { value, configurable: true });
    }
  }
  preventDefault() {
    if (this.cancelable && !this.passive) this.defaultPrevented = true;
  }
  stopPropagation() {
    this.cancelBubble = true;
  }
  stopImmediatePropagation() {
    this.cancelBubble = true;
    this.immediate = true;
  }
  composedPath() {
    return [...this.path];
  }
  get returnValue() {
    return !this.defaultPrevented;
  }
  set returnValue(value: boolean) {
    if (!value) this.preventDefault();
  }
}
export class RemoteCustomEvent extends RemoteEvent {
  detail: unknown;
  constructor(type: string, init: EventInit & { detail?: unknown } = {}) {
    super(type, { ...init });
    this.detail = init.detail ?? null;
  }
}
type Listener = {
  callback: any;
  capture: boolean;
  once: boolean;
  passive: boolean;
  signal?: AbortSignal;
};
const listeners = new WeakMap<object, Map<string, Listener[]>>();
export function eventTypes(target: object): string[] {
  return [...(listeners.get(target)?.keys() ?? [])];
}
export function add(target: object, type: string, callback: any, options: any = {}) {
  if (!callback || options?.signal?.aborted) return;
  const capture = typeof options === 'boolean' ? options : !!options.capture;
  let map = listeners.get(target);
  if (!map) listeners.set(target, (map = new Map()));
  let list = map.get(type);
  if (!list) map.set(type, (list = []));
  if (list.some((l) => l.callback === callback && l.capture === capture)) return;
  list.push({
    callback,
    capture,
    once: !!options.once,
    passive: !!options.passive,
    signal: options.signal
  });
  options?.signal?.addEventListener('abort', () => remove(target, type, callback, capture), {
    once: true
  });
}
export function remove(target: object, type: string, callback: any, options: any = {}) {
  const map = listeners.get(target),
    list = map?.get(type);
  const capture = typeof options === 'boolean' ? options : !!options.capture;
  if (list) {
    const remaining = list.filter((l) => l.callback !== callback || l.capture !== capture);
    if (remaining.length) map!.set(type, remaining);
    else map!.delete(type);
  }
}
export function dispatch(
  target: any,
  event: RemoteEvent,
  parent: (node: any) => any,
  report: (error: unknown) => void
) {
  if (event.dispatching || !event.type)
    throw new Error('InvalidStateError: event is already dispatching or has no type');
  event.dispatching = true;
  event.target = target;
  event.cancelBubble = event.immediate = false;
  const path = [target];
  for (let p = parent(target); p; p = parent(p)) path.push(p);
  event.path = path;
  function invoke(node: any, capture: boolean, phase: number) {
    event.currentTarget = node;
    event.eventPhase = phase;
    const list = listeners.get(node)?.get(event.type) ?? [];
    for (const listener of [...list]) {
      if (
        listener.capture !== capture ||
        listener.signal?.aborted ||
        !listeners.get(node)?.get(event.type)?.includes(listener)
      )
        continue;
      if (listener.once) remove(node, event.type, listener.callback, listener.capture);
      event.passive = listener.passive;
      try {
        if (typeof listener.callback === 'function') {
          const result = listener.callback.call(node, event);
          if (result === false && node['on' + event.type] === listener.callback)
            event.preventDefault();
        } else listener.callback.handleEvent(event);
      } catch (error) {
        report(error);
      }
      event.passive = false;
      if (event.immediate) break;
    }
  }
  try {
    for (let i = path.length - 1; i > 0 && !event.cancelBubble; i--) invoke(path[i], true, 1);
    if (!event.cancelBubble) {
      invoke(target, true, 2);
      if (!event.immediate) invoke(target, false, 2);
    }
    if (event.bubbles)
      for (let i = 1; i < path.length && !event.cancelBubble; i++) invoke(path[i], false, 3);
  } finally {
    event.currentTarget = null;
    event.eventPhase = 0;
    event.path = [];
    event.dispatching = false;
  }
  return !event.defaultPrevented;
}
