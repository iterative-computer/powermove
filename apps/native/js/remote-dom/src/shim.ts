import * as DOM from 'linkedom/worker';
import { RemoteSelection, RemoteRange } from './selection';
import { RemoteFormData } from './forms';
import { dynamicSelector, query, matches } from './selectors';
import { add, remove, dispatch, eventTypes, RemoteEvent, RemoteCustomEvent } from './events';
import {
  ROOT,
  VERSION,
  sameSession,
  properties,
  type Batch,
  type LayoutEntry,
  type Op,
  type PageMessage,
  type Policy,
  type Session
} from './protocol';

// LinkeDOM supplies tree identity, parsing, namespaces, templates and selectors.
// Only this boundary uses its structural types; public messages are strict JSON.
type N = any;
const owners = new WeakMap<object, RemoteDOM>();
function owner(node: N): RemoteDOM | undefined {
  return owners.get(node?.ownerDocument ?? node);
}
let patched = false;
function patchDOM() {
  if (patched) return;
  patched = true;
  const proto = DOM.EventTarget.prototype as N;
  proto.addEventListener = function (type: string, fn: N, options: N) {
    add(this, type, fn, options);
    owner(this)?.listenerChanged(this, type);
  };
  proto.removeEventListener = function (type: string, fn: N, options: N) {
    remove(this, type, fn, options);
    owner(this)?.listenerChanged(this, type);
  };
  proto.dispatchEvent = function (event: RemoteEvent) {
    const shim = owner(this);
    const result = dispatch(
      this,
      event,
      (node) => node.parentNode ?? (shim && node === shim.document ? shim.window : null),
      (error) => shim?.report(error)
    );
    shim?.listenersSettled(this);
    return result;
  };
  for (const name of ['firstChild', 'nextSibling']) {
    Object.defineProperty(DOM.Node.prototype, name, {
      configurable: true,
      get() {
        for (
          let p = Object.getPrototypeOf(this);
          p && p !== DOM.Node.prototype;
          p = Object.getPrototypeOf(p)
        ) {
          const descriptor = Object.getOwnPropertyDescriptor(p, name);
          if (descriptor?.get) return descriptor.get.call(this);
        }
        return null;
      }
    });
  }
  const ep = DOM.Element.prototype as N;
  const parentPrototype = Object.getPrototypeOf(ep);
  for (const name of ['querySelector', 'querySelectorAll']) {
    const original = parentPrototype[name];
    parentPrototype[name] = function (selector: string) {
      if (!dynamicSelector.test(selector)) return original.call(this, selector);
      const result = query(this, selector);
      return name === 'querySelector' ? (result[0] ?? null) : result;
    };
  }
  ep.setAttributeNS = function (namespace: string | null, name: string, value: string) {
    namespace = namespace || null;
    const local = name.split(':').pop();
    const existing = [...this.attributes].find(
      (attr: N) => (attr.namespaceURI ?? null) === namespace && attr.name.split(':').pop() === local
    );
    if (existing && existing.name !== name) this.removeAttribute(existing.name);
    this.setAttribute(name, value);
    Object.defineProperty(this.getAttributeNode(name), 'namespaceURI', {
      configurable: true,
      value: namespace
    });
  };
  ep.getAttributeNS = function (namespace: string | null, local: string) {
    return (
      [...this.attributes].find(
        (attr: N) =>
          (attr.namespaceURI ?? null) === (namespace || null) &&
          attr.name.split(':').pop() === local
      )?.value ?? null
    );
  };
  ep.hasAttributeNS = function (namespace: string | null, local: string) {
    return this.getAttributeNS(namespace, local) !== null;
  };
  ep.removeAttributeNS = function (namespace: string | null, local: string) {
    const attr = [...this.attributes].find(
      (attr: N) =>
        (attr.namespaceURI ?? null) === (namespace || null) && attr.name.split(':').pop() === local
    );
    if (attr) this.removeAttribute(attr.name);
  };
  function copyState(source: N, clone: N) {
    if (source.nodeType === 1) {
      Object.defineProperty(clone, 'namespaceURI', {
        configurable: true,
        value: source.namespaceURI
      });
      for (const attr of source.attributes)
        if (attr.namespaceURI)
          Object.defineProperty(clone.getAttributeNode(attr.name), 'namespaceURI', {
            configurable: true,
            value: attr.namespaceURI
          });
      const shim = owner(source),
        values = shim?.values.get(source);
      if (values) shim!.values.set(clone, new Map(values));
    }
    const original = [...source.childNodes],
      copied = [...clone.childNodes];
    for (let i = 0; i < copied.length; i++) copyState(original[i], copied[i]);
  }
  const clonePrototypes = new Set<N>();
  for (const start of [DOM.Element.prototype, DOM.DocumentFragment.prototype]) {
    for (
      let proto: N = start;
      proto && proto !== DOM.Node.prototype;
      proto = Object.getPrototypeOf(proto)
    ) {
      if (Object.hasOwn(proto, 'cloneNode') && !clonePrototypes.has(proto)) {
        clonePrototypes.add(proto);
        const original = proto.cloneNode;
        proto.cloneNode = function (deep: boolean) {
          const clone = original.call(this, deep);
          copyState(this, clone);
          return clone;
        };
      }
    }
  }
  const originalMatches = ep.matches;
  ep.matches = function (selector: string) {
    return dynamicSelector.test(selector)
      ? matches(this, selector)
      : originalMatches.call(this, selector);
  };
  ep.getBoundingClientRect = function () {
    return owner(this)!.rect(this);
  };
  ep.getClientRects = function () {
    const r = this.getBoundingClientRect();
    return r.width || r.height ? [r] : [];
  };
  ep.focus = function (options: FocusOptions = {}) {
    owner(this)!.focus(this, !!options.preventScroll);
  };
  ep.blur = function () {
    owner(this)!.blur(this);
  };
  (DOM.HTMLElement.prototype as N).blur = ep.blur;
  ep.scrollTo = ep.scroll = function (x: number | ScrollToOptions, y?: number) {
    const options = typeof x === 'number' ? { left: x, top: y ?? 0 } : x;
    owner(this)!.scroll(
      this,
      options.left ?? this.scrollLeft,
      options.top ?? this.scrollTop,
      options.behavior ?? 'auto'
    );
  };
  ep.scrollBy = function (x: number | ScrollToOptions, y?: number) {
    const options = typeof x === 'number' ? { left: x, top: y ?? 0 } : x;
    this.scrollTo({
      left: this.scrollLeft + (options.left ?? 0),
      top: this.scrollTop + (options.top ?? 0),
      behavior: options.behavior
    });
  };
  for (const [prop, field] of Object.entries({
    scrollLeft: 'scroll_left',
    scrollTop: 'scroll_top',
    scrollWidth: 'scroll_width',
    scrollHeight: 'scroll_height',
    clientWidth: 'client_width',
    clientHeight: 'client_height',
    offsetWidth: 'client_width',
    offsetHeight: 'client_height'
  })) {
    Object.defineProperty(ep, prop, {
      configurable: true,
      get() {
        const shim = owner(this);
        shim?.requestLayout(this);
        return (
          shim?.values.get(this)?.get(prop) ?? (shim?.layouts.get(this)?.entry as N)?.[field] ?? 0
        );
      },
      ...(prop === 'scrollLeft' || prop === 'scrollTop'
        ? {
            set(value: number) {
              const shim = owner(this)!;
              const entry = shim.layouts.get(this)?.entry;
              shim.scroll(
                this,
                prop === 'scrollLeft' ? value : (entry?.scroll_left ?? 0),
                prop === 'scrollTop' ? value : (entry?.scroll_top ?? 0),
                'auto'
              );
            }
          }
        : {})
    });
  }
  for (const C of [
    DOM.HTMLInputElement,
    DOM.HTMLTextAreaElement,
    DOM.HTMLOptionElement,
    DOM.HTMLSelectElement,
    DOM.HTMLButtonElement,
    DOM.HTMLElement
  ]) {
    for (const name of Object.keys(properties)) {
      // Restrict form-specific properties to controls; tabindex/open are generic.
      if (C === DOM.HTMLElement && !['tabIndex', 'open'].includes(name)) continue;
      const old = Object.getOwnPropertyDescriptor(C.prototype, name);
      Object.defineProperty(C.prototype, name, {
        configurable: true,
        get() {
          const shim = owner(this);
          if (!shim) return old?.get?.call(this);
          const values = shim.values.get(this);
          if (
            values?.has(name) &&
            !(this.localName === 'select' && ['value', 'selectedIndex'].includes(name))
          )
            return values.get(name);
          if (name === 'selected' && this.localName === 'option') {
            const select = this.closest('select');
            if (!select || select.multiple) return this.hasAttribute('selected');
            return (
              [...select.options].find(
                (o: N) => shim.values.get(o)?.get('selected') ?? o.hasAttribute('selected')
              ) === this ||
              (![...select.options].some(
                (o: N) => shim.values.get(o)?.has('selected') || o.hasAttribute('selected')
              ) &&
                select.options[0] === this)
            );
          }
          if (name === 'value' && this.localName === 'select')
            return [...this.options].find((o: N) => o.selected)?.value ?? '';
          if (name === 'selectedIndex' && this.localName === 'select')
            return [...this.options].findIndex((o: N) => o.selected);
          if (name === 'value' && this.localName === 'textarea') return this.textContent;
          if (name === 'value' && this.localName === 'option')
            return this.getAttribute('value') ?? this.textContent;
          if (name === 'value')
            return (
              this.getAttribute('value') ?? (['checkbox', 'radio'].includes(this.type) ? 'on' : '')
            );
          if (properties[name] === 'boolean')
            return this.hasAttribute(name === 'readOnly' ? 'readonly' : name);
          if (name === 'tabIndex')
            return Number(
              this.getAttribute('tabindex') ??
                (['input', 'button', 'select', 'textarea', 'a'].includes(this.localName) ? 0 : -1)
            );
          return old?.get?.call(this) ?? -1;
        },
        set(value: N) {
          const shim = owner(this);
          if (!shim) {
            old?.set?.call(this, value);
            return;
          }
          if (this.localName === 'select' && (name === 'value' || name === 'selectedIndex')) {
            [...this.options].forEach((o: N, i: number) => {
              o.selected = name === 'value' ? o.value === String(value) : i === Number(value);
            });
          } else {
            if (name === 'selected' && value && this.localName === 'option') {
              const select = this.closest('select');
              if (select && !select.multiple)
                for (const option of select.options)
                  if (option !== this) shim.setValue(option, 'selected', false);
            }
            shim.setValue(
              this,
              name,
              properties[name] === 'boolean'
                ? !!value
                : properties[name] === 'number'
                  ? Number(value)
                  : String(value)
            );
          }
        }
      });
    }
  }
  for (const C of [DOM.HTMLInputElement, DOM.HTMLTextAreaElement]) {
    Object.defineProperty(C.prototype, 'defaultValue', {
      configurable: true,
      get() {
        return this.localName === 'textarea'
          ? this.textContent
          : (this.getAttribute('value') ?? '');
      },
      set(value: string) {
        if (this.localName === 'textarea') this.textContent = value;
        else this.setAttribute('value', value);
      }
    });
    for (const name of ['selectionStart', 'selectionEnd', 'selectionDirection']) {
      Object.defineProperty(C.prototype, name, {
        configurable: true,
        get() {
          return (
            owner(this)?.values.get(this)?.get(name) ?? (name === 'selectionDirection' ? 'none' : 0)
          );
        },
        set(value: N) {
          owner(this)!.select(
            this,
            name === 'selectionStart' ? value : this.selectionStart,
            name === 'selectionEnd' ? value : this.selectionEnd,
            name === 'selectionDirection' ? value : this.selectionDirection
          );
        }
      });
    }
    (C.prototype as N).setSelectionRange = function (
      start: number,
      end: number,
      direction = 'none'
    ) {
      owner(this)!.select(this, start, end, direction);
    };
    (C.prototype as N).select = function () {
      this.setSelectionRange(0, this.value.length);
    };
  }
  for (const C of [
    DOM.HTMLInputElement,
    DOM.HTMLSelectElement,
    DOM.HTMLTextAreaElement,
    DOM.HTMLButtonElement
  ]) {
    Object.defineProperty(C.prototype, 'form', {
      configurable: true,
      get() {
        return this.getAttribute('form')
          ? this.ownerDocument.getElementById(this.getAttribute('form'))
          : this.closest('form');
      }
    });
  }
  Object.defineProperty(DOM.HTMLInputElement.prototype, 'defaultChecked', {
    configurable: true,
    get() {
      return this.hasAttribute('checked');
    },
    set(value: boolean) {
      this.toggleAttribute('checked', value);
    }
  });
  Object.defineProperty(DOM.HTMLOptionElement.prototype, 'defaultSelected', {
    configurable: true,
    get() {
      return this.hasAttribute('selected');
    },
    set(value: boolean) {
      this.toggleAttribute('selected', value);
    }
  });
  (DOM.HTMLElement.prototype as N).click = function () {
    if (this.disabled) return;
    const checkbox = this.localName === 'input' && this.type === 'checkbox';
    const checked = this.checked;
    if (checkbox) this.checked = !checked;
    if (
      !this.dispatchEvent(new RemoteEvent('click', { bubbles: true, cancelable: true, button: 0 }))
    ) {
      if (checkbox) this.checked = checked;
      return;
    }
    if (checkbox) {
      this.dispatchEvent(new RemoteEvent('input', { bubbles: true }));
      this.dispatchEvent(new RemoteEvent('change', { bubbles: true }));
    }
  };
  Object.defineProperty(DOM.HTMLInputElement.prototype, 'valueAsNumber', {
    configurable: true,
    get() {
      return this.value.trim() === '' ? NaN : Number(this.value);
    },
    set(value: number) {
      this.value = Number.isNaN(value) ? '' : String(value);
    }
  });
}

type Snapshot = {
  attributes: Map<string, { value: string; namespace: string | null }>;
  text: string;
  children: number[];
  props: Map<string, unknown>;
  listeners: Map<string, Policy>;
};
export type ShimOptions = {
  session: Session;
  /** Dedicated V8 global object: preserves window === globalThis and host globals. */
  realmGlobal?: Record<string, unknown>;
  /** Queue admission only: false retains an identical batch for retry. */
  send: (batch: Batch) => boolean;
  reportError?: (error: unknown) => void;
  lateCancellation?: (event: RemoteEvent) => void;
  queueMicrotask?: (callback: () => void) => void;
  requestAnimationFrame?: (callback: FrameRequestCallback) => number;
  cancelAnimationFrame?: (id: number) => void;
  maxBatchOperations?: number;
  maxBatchBytes?: number;
  maxNodes?: number;
};

export class RemoteDOM {
  readonly document: N;
  readonly window: N;
  readonly session: Session;
  readonly layouts = new WeakMap<object, { entry: LayoutEntry; applied: number }>();
  readonly values = new WeakMap<object, Map<string, N>>();
  private ids = new WeakMap<object, number>();
  private nodes = new Map<number, N>();
  private snapshots = new Map<number, Snapshot>();
  private parent = new Map<number, number>();
  private next = new Map<number, number | null>();
  private previous = new Map<number, number | null>();
  private last = new Map<number, number>();
  private dirty = new Set<N>();
  private policies = new WeakMap<object, Map<string, Policy>>();
  private observer: N;
  private queued = false;
  private nextId = 6;
  private seq = 0;
  private applied = 0;
  private lastEvent = 0;
  private causedBy: number | null = null;
  private commands: Op[] = [];
  private pending: Batch | null = null;
  private backlog: { ops: Op[]; caused_by: number | null }[] = [];
  private disposed = false;
  private request = 0;
  private layoutWanted = new Set<N>();
  private waits = new Map<
    number,
    { resolve: (entry: LayoutEntry) => void; reject: (error: Error) => void; node: N }
  >();
  private resize = new Set<RemoteResizeObserver>();
  private resizeTargets = new Map<N, Set<RemoteResizeObserver>>();
  private active: N;
  private selection: RemoteSelection;
  private diagnostics: string[] = [];
  private silent = false;
  constructor(private options: ShimOptions) {
    if (
      !options.session.panel ||
      !Number.isInteger(options.session.generation) ||
      options.session.generation < 1
    )
      throw new Error('Invalid session');
    patchDOM();
    this.session = Object.freeze({ ...options.session });
    this.document = DOM.parseHTML(
      '<!doctype html><html><head></head><body></body></html>'
    ).document;
    const Observer = this.document.defaultView.MutationObserver;
    const createElementNS = this.document.createElementNS;
    this.document.createElementNS = (namespace: string | null, name: string, options: N) => {
      const node = createElementNS.call(this.document, namespace, name, options);
      Object.defineProperty(node, 'namespaceURI', { configurable: true, value: namespace || null });
      return node;
    };
    const window: N = Object.assign(
      options.realmGlobal ??
        new Proxy(new DOM.EventTarget(), {
          get(target, key, receiver) {
            return Reflect.has(target, key)
              ? Reflect.get(target, key, receiver)
              : Reflect.get(globalThis, key);
          }
        }),
      DOM,
      {
        document: this.document,
        Event: RemoteEvent,
        CustomEvent: RemoteCustomEvent
      }
    );
    this.window = window;
    owners.set(this.document, this);
    owners.set(window, this);
    Object.defineProperty(this.document, 'defaultView', { value: window });
    Object.assign(window, {
      window,
      self: window,
      top: window,
      parent: window,
      getComputedStyle: (node: N) => this.computedStyle(node),
      requestAnimationFrame:
        options.requestAnimationFrame ??
        (() => {
          throw new Error('Host animation-frame scheduler required');
        }),
      cancelAnimationFrame: options.cancelAnimationFrame ?? (() => {}),
      MutationObserver: Observer,
      FormData: RemoteFormData,
      navigator: { userAgent: 'Powermove Native Remote DOM' },
      KeyboardEvent: RemoteEvent,
      MouseEvent: RemoteEvent,
      PointerEvent: RemoteEvent,
      InputEvent: RemoteEvent,
      FocusEvent: RemoteEvent,
      WheelEvent: RemoteEvent,
      ResizeObserver: class extends RemoteResizeObserver {
        constructor(callback: ResizeObserverCallback) {
          super(callback, owner(window)!);
        }
      }
    });
    for (const name of ['addEventListener', 'removeEventListener', 'dispatchEvent'])
      window[name] = (DOM.EventTarget.prototype as N)[name].bind(window);
    this.selection = new RemoteSelection((selection) => {
      if (this.silent || this.disposed) return;
      this.commands.push({
        op: 'range',
        anchor: selection.anchorNode ? this.id(selection.anchorNode) : null,
        anchor_offset: selection.anchorOffset,
        focus: selection.focusNode ? this.id(selection.focusNode) : null,
        focus_offset: selection.focusOffset
      });
      this.schedule();
    });
    this.document.getSelection = window.getSelection = () => this.selection;
    this.document.createRange = () => new RemoteRange(this.document);
    window.Range = RemoteRange;
    this.active = this.document.body;
    Object.defineProperty(this.document, 'activeElement', { get: () => this.active });
    for (const [node, id] of [
      [this.document, ROOT.document],
      [this.document.documentElement, ROOT.html],
      [this.document.head, ROOT.head],
      [this.document.body, ROOT.body],
      [window, ROOT.window]
    ] as const) {
      this.ids.set(node, id);
      this.nodes.set(id, node);
      this.snapshots.set(id, this.empty());
    }
    this.parent.set(ROOT.document, ROOT.window);
    this.parent.set(ROOT.html, ROOT.document);
    this.parent.set(ROOT.head, ROOT.html);
    this.parent.set(ROOT.body, ROOT.html);
    this.snapshots.get(ROOT.document)!.children = [ROOT.html];
    this.snapshots.get(ROOT.html)!.children = [ROOT.head, ROOT.body];
    this.observer = new Observer((records: N[]) => {
      this.collect(records);
      this.flush();
    });
    this.observer.observe(this.document, {
      subtree: true,
      childList: true,
      attributes: true,
      characterData: true
    });
    // Keep browser-owned form/focus/selection state fresh even without authored listeners.
    for (const type of ['input', 'change', 'focusin', 'focusout', 'selectionchange'])
      this.document.addEventListener(type, () => {});
  }
  report(error: unknown) {
    this.diagnostics.push(error instanceof Error ? error.message : String(error));
    if (this.diagnostics.length > 32) this.diagnostics.shift();
    this.options.reportError?.(error);
  }
  private empty(): Snapshot {
    return {
      attributes: new Map(),
      text: '',
      children: [],
      props: new Map(),
      listeners: new Map()
    };
  }
  id(node: N): number {
    const known = this.ids.get(node);
    if (known) return known;
    if ((node.ownerDocument ?? node) !== this.document)
      throw new Error('Cross-document nodes require importNode');
    if (this.nodes.size >= (this.options.maxNodes ?? 100_000))
      throw new Error('Remote DOM node limit');
    const id = this.nextId++;
    if (!Number.isSafeInteger(id)) throw new Error('Node ID limit');
    this.ids.set(node, id);
    this.nodes.set(id, node);
    return id;
  }
  node(id: number): N {
    return this.nodes.get(id);
  }
  private mark(node: N) {
    if (!this.silent && !this.disposed) {
      this.dirty.add(node);
      this.schedule();
    }
  }
  private schedule() {
    if (this.queued) return;
    this.queued = true;
    (this.options.queueMicrotask ?? queueMicrotask)(() => {
      this.queued = false;
      if (!this.disposed) this.flush();
    });
  }
  private collect(records: N[]) {
    for (const record of records) {
      this.dirty.add(record.target);
      for (const node of [...(record.addedNodes ?? []), ...(record.removedNodes ?? [])]) {
        this.dirty.add(node);
        if (node.parentNode) this.dirty.add(node.parentNode);
        const oldParent = this.parent.get(this.ids.get(node)!);
        if (oldParent) this.dirty.add(this.node(oldParent));
      }
    }
  }
  listenerChanged(node: N, _type: string) {
    this.mark(node);
  }
  setEventPolicy(node: N, event: string, policy: Policy) {
    let map = this.policies.get(node);
    if (!map) this.policies.set(node, (map = new Map()));
    map.set(event, policy);
    this.mark(node);
  }
  setValue(node: N, name: string, value: N) {
    let map = this.values.get(node);
    if (!map) this.values.set(node, (map = new Map()));
    map.set(name, value);
    this.mark(node);
  }
  private ensure(node: N, ops: Op[]) {
    const id = this.id(node);
    if (!this.snapshots.has(id)) {
      const kind = ({ 1: 'element', 3: 'text', 8: 'comment', 11: 'fragment' } as const)[
        node.nodeType as 1
      ];
      if (!kind) throw new Error(`Unsupported DOM node type ${node.nodeType}`);
      ops.push({
        op: 'create',
        id,
        kind,
        name: node.localName ?? '',
        namespace: node.namespaceURI ?? null
      });
      this.snapshots.set(id, this.empty());
      this.sync(node, ops, true);
    }
    return id;
  }
  private sync(node: N, ops: Op[], deep = false) {
    const id = this.ensure(node, ops),
      old = this.snapshots.get(id)!;
    if (node.nodeType === 3 || node.nodeType === 8) {
      if (node.data !== old.text) {
        old.text = node.data;
        ops.push({ op: 'text', id, value: node.data });
      }
      return;
    }
    if (node.nodeType === 1) {
      const attrs = new Map<string, { value: string; namespace: string | null }>();
      for (const attr of node.attributes)
        attrs.set(attr.name, { value: attr.value, namespace: attr.namespaceURI ?? null });
      for (const [name, attr] of attrs)
        if (
          old.attributes.get(name)?.value !== attr.value ||
          old.attributes.get(name)?.namespace !== attr.namespace
        )
          ops.push({ op: 'attribute', id, name, namespace: attr.namespace, value: attr.value });
      for (const [name, attr] of old.attributes)
        if (!attrs.has(name))
          ops.push({ op: 'attribute', id, name, namespace: attr.namespace, value: null });
      old.attributes = attrs;
      for (const [name, value] of this.values.get(node) ?? []) {
        if (name in properties && old.props.get(name) !== value) {
          ops.push({ op: 'property', id, name, value });
          old.props.set(name, value);
        }
      }
    }
    const types = new Set([
      ...eventTypes(node),
      ...Object.keys(node)
        .filter((key) => /^on[a-z]/.test(key) && typeof node[key] === 'function')
        .map((key) => key.slice(2))
    ]);
    for (const type of new Set([...types, ...old.listeners.keys()])) {
      const enabled = types.has(type),
        policy = this.policies.get(node)?.get(type) ?? 'observe';
      if ((enabled && old.listeners.get(type) !== policy) || (!enabled && old.listeners.has(type)))
        ops.push({ op: 'listen', id, event: type, enabled, policy });
      if (enabled) old.listeners.set(type, policy);
      else old.listeners.delete(type);
    }
    // The window and document roots are pre-mounted by the page host.
    if (id === ROOT.window || id === ROOT.document || id === ROOT.html) return;
    const children = [...(node.childNodes ?? [])] as N[];
    const ids = children.map((child) => this.ensure(child, ops));
    const desired = new Set(ids);
    for (const child of old.children)
      if (!desired.has(child) && this.parent.get(child) === id) {
        ops.push({ op: 'remove', parent: id, id: child });
        this.unlink(child);
      }
    if (ids.length !== old.children.length || ids.some((child, i) => child !== old.children[i])) {
      // Reverse insertion preserves anchors for keyed reorder and moves; identity
      // and all form state/listeners stay attached to the same native element.
      let before: number | null = null;
      for (let i = ids.length - 1; i >= 0; i--) {
        const child = ids[i];
        if (this.parent.get(child) !== id || (this.next.get(child) ?? null) !== before) {
          ops.push({ op: 'insert', parent: id, id: child, before });
          this.link(child, id, before);
        }
        before = child;
        this.sync(children[i], ops, true);
      }
      old.children = ids;
    } else if (deep) for (const child of children) this.sync(child, ops, true);
  }
  /** Immediate queue admission; no frame timer, await, or synchronous page call. */
  flush(): Batch | null {
    if (this.disposed) return null;
    if (!this.pending && this.backlog.length) {
      const chunk = this.backlog.shift()!;
      this.pending = {
        version: VERSION,
        session: this.session,
        seq: this.seq + 1,
        caused_by: chunk.caused_by,
        ops: chunk.ops
      };
    }
    if (this.pending) {
      if (!this.options.send(this.pending)) return null;
      this.seq = this.pending.seq;
      const sent = this.pending;
      this.pending = null;
      if (this.backlog.length || this.dirty.size || this.commands.length || this.layoutWanted.size)
        this.schedule();
      return sent;
    }
    this.collect(this.observer.takeRecords());
    const ops: Op[] = [];
    for (const node of this.dirty) this.sync(node, ops);
    this.dirty.clear();
    // Ensure IDs referenced by focus/layout/listeners exist before commands.
    for (const command of this.commands) {
      if ('id' in command) this.ensure(this.node(command.id), ops);
      if (command.op === 'range') {
        if (command.anchor) this.ensure(this.node(command.anchor), ops);
        if (command.focus) this.ensure(this.node(command.focus), ops);
      }
      if (command.op === 'measure') for (const id of command.ids) this.ensure(this.node(id), ops);
      ops.push(command);
    }
    this.commands = [];
    if (this.layoutWanted.size) {
      const ids = [...this.layoutWanted].map((node) => this.ensure(node, ops));
      this.layoutWanted.clear();
      ops.push({ op: 'measure', request: ++this.request, ids, styles: [] });
    }
    if (!ops.length) return null;
    let chunk: Op[] = [],
      bytes = 0;
    const maxOps = this.options.maxBatchOperations ?? 2048;
    const maxBytes = this.options.maxBatchBytes ?? 128 * 1024;
    for (const op of ops) {
      const size = JSON.stringify(op).length * 3; // conservative UTF-8 bound, header separate
      if (chunk.length && (chunk.length >= maxOps || bytes + size > maxBytes)) {
        this.backlog.push({ ops: chunk, caused_by: this.causedBy });
        chunk = [];
        bytes = 0;
      }
      chunk.push(op);
      bytes += size;
    }
    if (chunk.length) this.backlog.push({ ops: chunk, caused_by: this.causedBy });
    return this.flush();
  }

  focus(node: N, preventScroll: boolean) {
    if (this.active === node) return;
    const previous = this.active;
    this.active = node;
    previous.dispatchEvent(new RemoteEvent('blur'));
    previous.dispatchEvent(new RemoteEvent('focusout', { bubbles: true }));
    node.dispatchEvent(new RemoteEvent('focus'));
    node.dispatchEvent(new RemoteEvent('focusin', { bubbles: true }));
    if (!this.silent) {
      this.commands.push({ op: 'focus', id: this.id(node), prevent_scroll: preventScroll });
      this.schedule();
    }
  }
  blur(node: N) {
    if (this.active !== node) return;
    this.active = this.document.body;
    node.dispatchEvent(new RemoteEvent('blur'));
    node.dispatchEvent(new RemoteEvent('focusout', { bubbles: true }));
    if (!this.silent) {
      this.commands.push({ op: 'blur', id: this.id(node) });
      this.schedule();
    }
  }
  select(node: N, start: number, end: number, direction: string) {
    end = Math.max(0, Math.min(node.value.length, end));
    start = Math.max(0, Math.min(end, start));
    for (const [key, value] of Object.entries({
      selectionStart: start,
      selectionEnd: end,
      selectionDirection: direction
    }))
      this.setValue(node, key, value);
    if (!this.silent)
      this.commands.push({ op: 'selection', id: this.id(node), start, end, direction });
    this.schedule();
  }
  scroll(node: N, x: number, y: number, behavior: string) {
    if (!Number.isFinite(x) || !Number.isFinite(y))
      throw new TypeError('Scroll coordinates must be finite');
    this.setValue(node, 'scrollLeft', x);
    this.setValue(node, 'scrollTop', y);
    this.commands.push({ op: 'scroll', id: this.id(node), x, y, behavior });
    this.schedule();
  }
  rect(node: N): DOMRect {
    this.requestLayout(node);
    const rect = this.layouts.get(node)?.entry.rect ?? { x: 0, y: 0, width: 0, height: 0 };
    return {
      ...rect,
      top: rect.y,
      left: rect.x,
      right: rect.x + rect.width,
      bottom: rect.y + rect.height,
      toJSON: () => ({ ...rect })
    };
  }
  layoutStatus(node: N) {
    return {
      applied: this.layouts.get(node)?.applied ?? null,
      current: this.applied,
      stale: (this.layouts.get(node)?.applied ?? -1) < this.seq
    };
  }
  requestLayout(node: N) {
    if (!this.disposed) {
      this.layoutWanted.add(node);
      this.schedule();
    }
  }
  measure(node: N, styles: string[] = []): Promise<LayoutEntry> {
    if (this.disposed) return Promise.reject(new Error('Remote DOM retired'));
    const request = ++this.request;
    this.commands.push({ op: 'measure', request, ids: [this.id(node)], styles });
    this.schedule();
    return new Promise((resolve, reject) => this.waits.set(request, { resolve, reject, node }));
  }
  computedStyle(node: N) {
    this.requestLayout(node);
    const values = this.layouts.get(node)?.entry.styles ?? {};
    return new Proxy(
      {
        getPropertyValue: (name: string) => values[name] ?? '',
        item: (index: number) => Object.keys(values)[index] ?? '',
        length: Object.keys(values).length
      },
      {
        get(target, name: string) {
          return name in target
            ? target[name as keyof typeof target]
            : (values[name.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase())] ?? '');
        }
      }
    ) as unknown as CSSStyleDeclaration;
  }
  observeResize(observer: RemoteResizeObserver, node: N, enabled: boolean) {
    let observers = this.resizeTargets.get(node);
    if (!observers) this.resizeTargets.set(node, (observers = new Set()));
    const wasObserved = !!observers.size;
    if (enabled) {
      observers.add(observer);
      this.resize.add(observer);
    } else observers.delete(observer);
    if (!observers.size) this.resizeTargets.delete(node);
    if (![...this.resizeTargets.values()].some((group) => group.has(observer)))
      this.resize.delete(observer);
    if (wasObserved !== !!observers.size) {
      this.commands.push({ op: 'observe_resize', id: this.id(node), enabled });
      this.schedule();
    }
  }

  receive(message: PageMessage): boolean {
    if (this.disposed || !sameSession(message.session, this.session)) return false;
    if (message.kind === 'fault') {
      this.report(new Error(message.message));
      this.dispose();
      return true;
    }
    if (message.kind === 'applied') {
      if (message.seq < this.applied || message.seq > this.seq) return false;
      this.applied = message.seq;
      if (this.pending || this.backlog.length) this.schedule();
      return true;
    }
    if (message.applied < this.applied || message.applied > this.seq) return false;
    if (message.kind === 'layout') {
      for (const entry of message.entries) {
        const node = this.node(entry.id);
        if (!node) continue;
        const previous = this.layouts.get(node);
        if (previous && previous.applied > message.applied) continue;
        this.layouts.set(node, { entry, applied: message.applied });
        let values = this.values.get(node);
        if (!values) this.values.set(node, (values = new Map()));
        values.set('scrollLeft', entry.scroll_left);
        values.set('scrollTop', entry.scroll_top);
        for (const observer of this.resize) observer.deliver(node, entry, previous?.entry);
      }
      if (message.request !== null) {
        const wait = this.waits.get(message.request);
        const entry = message.entries.find((e) => this.node(e.id) === wait?.node);
        if (wait && entry) {
          this.waits.delete(message.request);
          wait.resolve(entry);
        }
      }
      return true;
    }
    if (message.seq !== this.lastEvent + 1) return false;
    const node = this.node(message.target);
    if (!node) return false;
    this.lastEvent = message.seq;
    this.causedBy = message.seq;
    this.silent = true;
    try {
      const selected = message.data.selection as
        | {
            anchor: number | null;
            anchor_offset: number;
            focus: number | null;
            focus_offset: number;
          }
        | undefined;
      if (selected) {
        if (selected.anchor !== null && selected.focus !== null)
          this.selection.setBaseAndExtent(
            this.node(selected.anchor),
            selected.anchor_offset,
            this.node(selected.focus),
            selected.focus_offset
          );
        else this.selection.removeAllRanges();
      }
      const state = message.data.state as Record<string, N> | undefined;
      if (state && node.localName === 'select' && Array.isArray(state.selectedIndices)) {
        [...node.options].forEach((option: N, index: number) => {
          let values = this.values.get(option);
          if (!values) this.values.set(option, (values = new Map()));
          const selected = state.selectedIndices.includes(index);
          values.set('selected', selected);
          this.snapshots.get(this.ids.get(option)!)?.props.set('selected', selected);
        });
      }
      if (state)
        for (const [key, value] of Object.entries(state)) {
          let values = this.values.get(node);
          if (!values) this.values.set(node, (values = new Map()));
          values.set(key, value);
          this.snapshots.get(message.target)?.props.set(key, value);
        }
      if (message.event === 'focus' || message.event === 'focusin') this.active = node;
      if (message.event === 'blur' || message.event === 'focusout')
        this.active = this.document.body;
    } finally {
      this.silent = false;
    }
    const event = new RemoteEvent(message.event, {
      ...message.data,
      ...('relatedTarget' in message.data
        ? { relatedTarget: this.node(message.data.relatedTarget as number) ?? null }
        : {}),
      bubbles: message.bubbles,
      cancelable: message.cancelable
    });
    event.defaultPrevented = message.default_prevented;
    event.isTrusted = message.data.isTrusted === true;
    node.dispatchEvent(event);
    if (event.defaultPrevented && !message.default_prevented) {
      if (this.options.lateCancellation) this.options.lateCancellation(event);
      else
        this.report(new Error(`Late default cancellation requires a page policy: ${event.type}`));
    }
    // Svelte's state effects flush in the current microtask checkpoint.
    this.schedule();
    return true;
  }
  private unlink(id: number) {
    const parent = this.parent.get(id),
      previous = this.previous.get(id) ?? null,
      next = this.next.get(id) ?? null;
    if (previous !== null) this.next.set(previous, next);
    if (next !== null) this.previous.set(next, previous);
    if (parent !== undefined && this.last.get(parent) === id) {
      if (previous !== null) this.last.set(parent, previous);
      else this.last.delete(parent);
    }
    this.parent.delete(id);
    this.next.delete(id);
    this.previous.delete(id);
  }
  private link(id: number, parent: number, before: number | null) {
    this.unlink(id);
    const previous =
      before === null ? (this.last.get(parent) ?? null) : (this.previous.get(before) ?? null);
    this.parent.set(id, parent);
    this.previous.set(id, previous);
    this.next.set(id, before);
    if (previous !== null) this.next.set(previous, id);
    if (before !== null) this.previous.set(before, id);
    else this.last.set(parent, id);
  }
  listenersSettled(node: N) {
    for (const target of [node, ...this.ancestors(node)]) {
      const previous = this.snapshots.get(this.ids.get(target)!)?.listeners,
        current = eventTypes(target);
      if (
        previous &&
        (previous.size !== current.length || current.some((type) => !previous.has(type)))
      )
        this.mark(target);
    }
  }
  private *ancestors(node: N): Generator<N> {
    for (let parent = node.parentNode; parent; parent = parent.parentNode) yield parent;
    if (node.isConnected || node === this.document) yield this.window;
  }
  /** Explicitly install only in a realm dedicated to this project/cluster. */
  installGlobals(target: N = this.options.realmGlobal ?? globalThis): () => void {
    const previous = new Map<string, PropertyDescriptor | undefined>();
    for (const key of [
      'window',
      'document',
      ...Object.keys(DOM),
      'Event',
      'CustomEvent',
      'KeyboardEvent',
      'MouseEvent',
      'PointerEvent',
      'InputEvent',
      'FocusEvent',
      'WheelEvent',
      'ResizeObserver',
      'MutationObserver',
      'FormData',
      'getComputedStyle',
      'requestAnimationFrame',
      'cancelAnimationFrame',
      'getSelection',
      'Range',
      'navigator',
      'addEventListener',
      'removeEventListener',
      'dispatchEvent'
    ]) {
      previous.set(key, Object.getOwnPropertyDescriptor(target, key));
      Object.defineProperty(target, key, {
        configurable: true,
        writable: true,
        value: this.window[key]
      });
    }
    return () => {
      for (const [key, descriptor] of previous) {
        if (descriptor) Object.defineProperty(target, key, descriptor);
        else delete target[key];
      }
    };
  }
  inspect() {
    return {
      session: this.session,
      sent: this.seq,
      applied: this.applied,
      nodes: this.nodes.size,
      pending: !!this.pending || !!this.backlog.length,
      retired: this.disposed,
      diagnostics: [...this.diagnostics]
    };
  }
  dispose() {
    this.disposed = true;
    this.observer.disconnect();
    for (const wait of this.waits.values()) wait.reject(new Error('Remote DOM retired'));
    this.waits.clear();
    this.nodes.clear();
    this.snapshots.clear();
    this.next.clear();
    this.previous.clear();
    this.last.clear();
    this.parent.clear();
    this.dirty.clear();
    this.commands = [];
    this.pending = null;
    this.backlog = [];
    this.resize.clear();
    this.resizeTargets.clear();
  }
}
class RemoteResizeObserver {
  private targets = new Set<N>();
  constructor(
    private callback: ResizeObserverCallback,
    private shim: RemoteDOM
  ) {}
  observe(node: N) {
    this.targets.add(node);
    this.shim.observeResize(this, node, true);
  }
  unobserve(node: N) {
    this.targets.delete(node);
    this.shim.observeResize(this, node, false);
  }
  disconnect() {
    for (const node of this.targets) this.unobserve(node);
  }
  deliver(node: N, entry: LayoutEntry, previous?: LayoutEntry) {
    if (
      !this.targets.has(node) ||
      (previous?.rect.width === entry.rect.width && previous.rect.height === entry.rect.height)
    )
      return;
    this.callback(
      [{ target: node, contentRect: this.shim.rect(node) } as ResizeObserverEntry],
      this as unknown as ResizeObserver
    );
  }
}
