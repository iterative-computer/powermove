import {
  primitives,
  json,
  type Primitive,
  type Op,
  type Props,
  type Json,
  type SurfaceEvent,
  type ListenerFlags,
} from "./protocol";

export const EMPTY_STRING = Object.freeze({ nativeEmptyString: true });

// Virtual adapter for Preact only. No browser, layout reads, or DOM is exposed to extensions.
export class Host {
  nextId = 0;
  ops: Op[] = [];
  readonly nodes = new Map<number, Element>();
  readonly root: Element;
  poisoned: unknown = null;
  constructor() {
    this.root = new Element(this, 0, "root");
  }
  document = {
    createElement: (type: string) => this.create(type),
    createElementNS: (_ns: string, type: string) => this.create(type),
    createTextNode: (text: string) => this.create("text", text, true),
  };
  create(type: string, text = "", leaf = false): Element {
    if (!primitives.includes(type as Primitive))
      throw new TypeError(`Unknown surface primitive: ${type}`);
    const id = ++this.nextId;
    if (!Number.isSafeInteger(id))
      throw new RangeError("Element ids exhausted");
    const node = new Proxy(new Element(this, id, type, leaf), {
      has(target, key) {
        return (
          (typeof key === "string" && key.startsWith("pm:")) ||
          Reflect.has(target, key)
        );
      },
      set(target, key, value, receiver) {
        if (typeof key === "string" && key.startsWith("pm:")) {
          if (value === "") target.removeAttribute(key.slice(3));
          else
            target.setAttribute(
              key.slice(3),
              value === EMPTY_STRING ? "" : value,
            );
          return true;
        }
        return Reflect.set(target, key, value, receiver);
      },
    });
    this.nodes.set(id, node);
    this.ops.push({ op: "create", id, type: type as Primitive });
    if (text) node.data = text;
    return node;
  }
  take(): Op[] {
    const ops = this.ops;
    this.ops = [];
    return ops;
  }
}

type Handler = (event: SurfaceEvent) => void;
export class Element {
  children: Element[] = [];
  parent: Element | null = null;
  props: Props = Object.create(null);
  handlers = new Map<string, Handler>();
  private text = "";
  readonly namespaceURI = "http://www.w3.org/1999/xhtml";
  readonly nodeType: number;
  readonly localName: string;
  constructor(
    readonly host: Host,
    readonly id: number,
    readonly type: string,
    leaf = false,
  ) {
    this.nodeType = leaf ? 3 : 1;
    this.localName = type;
    host.nodes.set(id, this);
  }
  get ownerDocument() {
    return this.host.document;
  }
  get nodeName() {
    return this.type.toUpperCase();
  }
  get parentNode() {
    return this.parent;
  }
  get childNodes() {
    return this.children;
  }
  get firstChild() {
    return this.children[0] ?? null;
  }
  get nextSibling(): Element | null {
    return (
      this.parent?.children[this.parent.children.indexOf(this) + 1] ?? null
    );
  }
  get attributes() {
    return Object.keys(this.props).map((name) => ({
      name,
      value: this.props[name],
    }));
  }
  appendChild(node: Element) {
    return this.insertBefore(node, null);
  }
  insertBefore(node: Element, before: Element | null): Element {
    if (node === before) return node;
    if (before && before.parent !== this)
      throw new Error("Invalid insertion anchor");
    if (node.parent)
      node.parent.children.splice(node.parent.children.indexOf(node), 1);
    node.parent = this;
    this.children.splice(
      before ? this.children.indexOf(before) : this.children.length,
      0,
      node,
    );
    this.host.ops.push({
      op: "insertBefore",
      parent: this.id,
      id: node.id,
      before: before?.id ?? null,
    });
    return node;
  }
  removeChild(node: Element): Element {
    const index = this.children.indexOf(node);
    if (index < 0) throw new Error("Invalid removal");
    this.children.splice(index, 1);
    node.parent = null;
    const pending = [node];
    while (pending.length) {
      const next = pending.pop()!;
      this.host.nodes.delete(next.id);
      pending.push(...next.children);
    }
    this.host.ops.push({ op: "remove", id: node.id });
    return node;
  }
  remove() {
    this.parent?.removeChild(this);
  }
  get data() {
    return this.text;
  }
  set data(text: string) {
    this.text = String(text);
    this.host.ops.push({ op: "setText", id: this.id, text: this.text });
  }
  get textContent(): string {
    return this.text + this.children.map((c) => c.textContent).join("");
  }
  set textContent(text: string) {
    for (const child of [...this.children]) this.removeChild(child);
    this.data = text;
  }
  setAttribute(key: string, value: unknown) {
    if (key.startsWith("pm:")) key = key.slice(3);
    const v = json(value);
    this.props[key] = v;
    this.host.ops.push({ op: "setProps", id: this.id, patch: { [key]: v } });
    if (key === "preventDefault") this.listeners();
  }
  removeAttribute(key: string) {
    if (key.startsWith("pm:")) key = key.slice(3);
    delete this.props[key];
    this.host.ops.push({ op: "setProps", id: this.id, patch: { [key]: null } });
    if (key === "preventDefault") this.listeners();
  }
  // Object-valued style/bindings remain structured data; Preact normally serializes attributes.
  get style(): unknown {
    return this.props.style;
  }
  set style(value: unknown) {
    this.setAttribute("style", value);
  }
  get value(): Json | typeof EMPTY_STRING | undefined {
    return this.props.value === "" ? EMPTY_STRING : this.props.value;
  }
  set value(value: unknown) {
    if (value === "") this.removeAttribute("value");
    else this.setAttribute("value", value === EMPTY_STRING ? "" : value);
  }
  get checked(): Json | undefined {
    return this.props.checked;
  }
  set checked(value: unknown) {
    this.setAttribute("checked", value);
  }
  addEventListener(name: string, handler: Handler, capture: boolean) {
    this.handlers.set(`${name.toLowerCase()}:${!!capture}`, handler);
    this.listeners();
  }
  removeEventListener(name: string, _handler: Handler, capture: boolean) {
    this.handlers.delete(`${name.toLowerCase()}:${!!capture}`);
    this.listeners();
  }
  private listeners() {
    const flags: Record<string, ListenerFlags> = Object.create(null);
    const defaults = (this.props.preventDefault ?? {}) as Record<
      string,
      boolean | string[]
    >;
    for (const name of new Set(
      [...this.handlers.keys()]
        .map((k) => k.split(":")[0])
        .concat(Object.keys(defaults)),
    )) {
      const declared = defaults[name];
      flags[name] = {
        capture: this.handlers.has(`${name}:true`),
        bubble: this.handlers.has(`${name}:false`),
        preventDefault: !!declared,
        keys: Array.isArray(declared) ? declared : [],
      };
    }
    this.host.ops.push({ op: "setListeners", id: this.id, flags });
  }
}
