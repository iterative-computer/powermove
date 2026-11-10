import {
  ROOT,
  VERSION,
  properties,
  sameSession,
  validId,
  type Batch,
  type LayoutEntry,
  type Op,
  type PageMessage,
  type Policy,
  type Session
} from './protocol';

const HTML_NS = 'http://www.w3.org/1999/xhtml';
const forbiddenTags = new Set(['script', 'iframe', 'object', 'embed', 'base']);
function validName(name: string) {
  return !!name && name.length <= 256 && !/[\s\x00-\x1f<>/"'=]/.test(name);
}
export function assertAttribute(name: string, value: string | null) {
  if (!validName(name) || /^on/i.test(name) || name.toLowerCase() === 'srcdoc')
    throw new Error('Executable/invalid attribute');
  if (
    value &&
    ['href', 'src', 'xlink:href', 'action', 'formaction'].includes(name.toLowerCase()) &&
    /^(?:javascript:|vbscript:|data:text\/html)/i.test(value.replace(/[\x00-\x20]/g, ''))
  )
    throw new Error('Executable URL');
}
type Mount = { node: Node | Window; parent: number | null; kind: string };
export type ApplierOptions = {
  document: Document;
  session: Session;
  send: (message: PageMessage) => void;
  maxOperations?: number;
  maxNodes?: number;
};
/** Run in a dedicated panel document. No extension code is evaluated here. */
export class PageApplier {
  private nodes = new Map<number, Mount>();
  private reverse = new WeakMap<object, number>();
  private subscriptions = new Map<number, Map<string, Policy>>();
  private handlers = new Map<string, EventListener>();
  /** The panel's window and any window it presents part of its DOM in (a
   * ui.modal sheet's document): events are captured in each. */
  private windows = new Set<Window>();
  private resize: ResizeObserver | null = null;
  private observed = new Set<number>();
  private seq = 0;
  private event = 0;
  private applying = false;
  private edited = new WeakMap<object, number>();
  private focused = 0;
  private selected = 0;
  private retired = false;
  private session: Session;
  readonly document: Document;
  constructor(private options: ApplierOptions) {
    this.document = options.document;
    this.session = Object.freeze({ ...options.session });
    if (!this.document.defaultView || !this.document.body || !this.document.head)
      throw new Error('Panel document must be ready');
    for (const [id, node, parent, kind] of [
      [ROOT.document, this.document, ROOT.window, 'document'],
      [ROOT.html, this.document.documentElement, ROOT.document, 'element'],
      [ROOT.head, this.document.head, ROOT.html, 'element'],
      [ROOT.body, this.document.body, ROOT.html, 'element'],
      [ROOT.window, this.document.defaultView, null, 'window']
    ] as const) {
      this.nodes.set(id, { node, parent, kind });
      this.reverse.set(node, id);
    }
    this.windows.add(this.document.defaultView);
    const RO = this.document.defaultView.ResizeObserver;
    if (RO)
      this.resize = new RO((entries) => {
        if (!this.retired)
          this.options.send({
            kind: 'layout',
            session: this.session,
            applied: this.seq,
            request: null,
            entries: entries.map((entry) => this.snapshot(this.reverse.get(entry.target)!))
          });
      });
  }
  node(id: number) {
    return this.nodes.get(id)?.node;
  }
  /** A same-origin window showing some of this panel's nodes (adopted into
   * its document): its events reach the realm as the panel's own. */
  attachWindow(view: Window) {
    if (this.retired || this.windows.has(view)) return;
    this.windows.add(view);
    for (const [type, handler] of this.handlers)
      view.addEventListener(type, handler, { capture: true, passive: false });
  }
  detachWindow(view: Window) {
    if (!this.windows.delete(view)) return;
    for (const [type, handler] of this.handlers) view.removeEventListener(type, handler, true);
  }
  private get(id: number): Mount {
    const node = this.nodes.get(id);
    if (!node) throw new Error('Unknown node');
    return node;
  }
  private snapshot(id: number, styles: string[] = []): LayoutEntry {
    const node = this.get(id).node as Element,
      rect = node.getBoundingClientRect();
    const computed = (
      node.ownerDocument.defaultView ?? this.document.defaultView!
    ).getComputedStyle(node);
    const values: Record<string, string> = {};
    // Cache declared style requests plus the complete enumerated declaration.
    for (const name of new Set([...Array.from(computed), ...styles]))
      values[name] = computed.getPropertyValue(name);
    return {
      id,
      rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
      scroll_left: node.scrollLeft,
      scroll_top: node.scrollTop,
      scroll_width: node.scrollWidth,
      scroll_height: node.scrollHeight,
      client_width: node.clientWidth,
      client_height: node.clientHeight,
      styles: values
    };
  }
  private preflight(batch: Batch) {
    if (
      batch.version !== VERSION ||
      !sameSession(batch.session, this.session) ||
      batch.seq !== this.seq + 1 ||
      !validId(batch.seq)
    )
      throw new Error('Version/session/sequence mismatch');
    if (batch.ops.length > (this.options.maxOperations ?? 16_384))
      throw new Error('Batch operation limit');
    const state = new Map<number, { parent: number | null; kind: string }>();
    let created = 0;
    const get = (id: number) => {
      const value = state.get(id) ?? this.nodes.get(id);
      if (!value) throw new Error('Unknown node');
      return value;
    };
    const element = (id: number) => {
      if (get(id).kind !== 'element') throw new Error('Expected element');
    };
    for (const op of batch.ops) {
      if (op.op === 'create') {
        if (
          !validId(op.id) ||
          op.id <= ROOT.window ||
          state.has(op.id) ||
          this.nodes.has(op.id) ||
          !['element', 'text', 'comment', 'fragment'].includes(op.kind)
        )
          throw new Error('Invalid/duplicate create');
        if (
          op.kind === 'element' &&
          (!validName(op.name) || forbiddenTags.has(op.name.split(':').pop()!.toLowerCase()))
        )
          throw new Error('Executable/invalid element');
        state.set(op.id, { parent: null, kind: op.kind });
        created++;
        continue;
      }
      if (op.op === 'range') {
        if (op.anchor !== null) get(op.anchor);
        if (op.focus !== null) get(op.focus);
        if (
          (op.anchor === null) !== (op.focus === null) ||
          !Number.isInteger(op.anchor_offset) ||
          !Number.isInteger(op.focus_offset) ||
          op.anchor_offset < 0 ||
          op.focus_offset < 0
        )
          throw new Error('Invalid range');
        continue;
      }
      if (op.op === 'measure') {
        if (!validId(op.request) || op.styles.length > 128) throw new Error('Invalid measure');
        for (const id of op.ids) element(id);
        continue;
      }
      if (!('id' in op)) throw new Error('Unknown operation');
      const node = get(op.id);
      switch (op.op) {
        case 'insert': {
          if (
            op.id <= ROOT.window ||
            node.kind === 'fragment' ||
            !['element', 'fragment'].includes(get(op.parent).kind)
          )
            throw new Error('Invalid parent/root');
          if (op.before !== null && get(op.before).parent !== op.parent)
            throw new Error('Invalid anchor');
          let ancestor: number | null = op.parent;
          while (ancestor !== null) {
            if (ancestor === op.id) throw new Error('DOM cycle');
            ancestor = get(ancestor).parent;
          }
          state.set(op.id, { ...node, parent: op.parent });
          break;
        }
        case 'remove':
          if (op.id <= ROOT.window || node.parent !== op.parent) throw new Error('Invalid remove');
          state.set(op.id, { ...node, parent: null });
          break;
        case 'attribute':
          element(op.id);
          assertAttribute(op.name, op.value);
          break;
        case 'text':
          if (!['text', 'comment'].includes(node.kind)) throw new Error('Invalid text target');
          break;
        case 'property':
          if (
            !(op.name in properties) ||
            typeof op.value !== properties[op.name] ||
            (typeof op.value === 'number' &&
              (!Number.isInteger(op.value) || op.value < -1 || op.value > 2147483647))
          )
            throw new Error('Invalid property');
          element(op.id);
          break;
        case 'listen':
          if (!op.event || op.event.length > 128 || !['observe', 'block'].includes(op.policy))
            throw new Error('Invalid listener');
          break;
        case 'focus':
        case 'blur':
        case 'observe_resize':
          element(op.id);
          break;
        case 'selection':
          element(op.id);
          if (
            !Number.isInteger(op.start) ||
            !Number.isInteger(op.end) ||
            op.start < 0 ||
            op.start > op.end ||
            !['none', 'forward', 'backward'].includes(op.direction)
          )
            throw new Error('Invalid selection');
          break;
        case 'scroll':
          element(op.id);
          if (
            !Number.isFinite(op.x) ||
            !Number.isFinite(op.y) ||
            !['auto', 'instant', 'smooth'].includes(op.behavior)
          )
            throw new Error('Invalid scroll');
          break;
        default:
          throw new Error('Unknown operation');
      }
    }
    if (this.nodes.size + created > (this.options.maxNodes ?? 100_000))
      throw new Error('Node limit');
  }
  /** Apply on arrival, without requestAnimationFrame. A fault fences this generation. */
  apply(batch: Batch): boolean {
    if (this.retired || !sameSession(batch.session, this.session)) return false;
    try {
      this.preflight(batch);
      this.applying = true;
      const measures: Extract<Op, { op: 'measure' }>[] = [];
      for (const op of batch.ops) {
        if (op.op === 'create') {
          const node =
            op.kind === 'element'
              ? op.namespace && op.namespace !== HTML_NS
                ? this.document.createElementNS(op.namespace, op.name)
                : this.document.createElement(op.name)
              : op.kind === 'text'
                ? this.document.createTextNode('')
                : op.kind === 'comment'
                  ? this.document.createComment('')
                  : this.document.createDocumentFragment();
          this.nodes.set(op.id, { node, kind: op.kind, parent: null });
          this.reverse.set(node, op.id);
          continue;
        }
        if (op.op === 'measure') {
          measures.push(op);
          continue;
        }
        if (op.op === 'range') {
          if (this.selected > (batch.caused_by ?? 0)) continue;
          const selection = this.document.getSelection()!;
          if (op.anchor === null) selection.removeAllRanges();
          else
            selection.setBaseAndExtent(
              this.get(op.anchor).node as Node,
              op.anchor_offset,
              this.get(op.focus!).node as Node,
              op.focus_offset
            );
          continue;
        }
        const mount = this.get(op.id),
          node = mount.node as HTMLElement;
        const control = node.localName === 'option' ? (node.closest('select') ?? node) : node;
        const newerInput = (this.edited.get(control) ?? 0) > (batch.caused_by ?? 0);
        switch (op.op) {
          case 'insert': {
            const parent = this.get(op.parent).node as Node;
            // A sibling shown in another document (a ui.modal sheet) is not
            // under `parent` here: the node goes last instead.
            const before = op.before === null ? null : (this.get(op.before).node as Node);
            parent.insertBefore(node, before?.parentNode === parent ? before : null);
            mount.parent = op.parent;
            break;
          }
          case 'remove':
            // Wherever it is shown: its panel parent, or a sheet's document.
            node.parentNode?.removeChild(node);
            mount.parent = null;
            break;
          case 'attribute':
            if (op.value === null) {
              if (op.namespace) node.removeAttributeNS(op.namespace, op.name.split(':').pop()!);
              else node.removeAttribute(op.name);
            } else if (op.namespace) node.setAttributeNS(op.namespace, op.name, op.value);
            else node.setAttribute(op.name, op.value);
            break;
          case 'text':
            node.nodeValue = op.value;
            break;
          case 'property':
            if (
              newerInput &&
              ['value', 'checked', 'indeterminate', 'selected', 'selectedIndex'].includes(op.name)
            )
              break;
            if ((node as any)[op.name] !== op.value) (node as any)[op.name] = op.value;
            break;
          case 'listen':
            this.listen(op.id, op.event, op.enabled, op.policy);
            break;
          case 'focus':
            if (this.focused > (batch.caused_by ?? 0)) break;
            node.focus({ preventScroll: op.prevent_scroll });
            break;
          case 'blur':
            if (this.focused > (batch.caused_by ?? 0)) break;
            node.blur();
            break;
          case 'selection':
            if (newerInput) break;
            (node as HTMLInputElement).setSelectionRange(op.start, op.end, op.direction as 'none');
            break;
          case 'scroll':
            if (typeof node.scrollTo === 'function')
              node.scrollTo({ left: op.x, top: op.y, behavior: op.behavior as ScrollBehavior });
            else {
              node.scrollLeft = op.x;
              node.scrollTop = op.y;
            }
            break;
          case 'observe_resize':
            if (op.enabled) {
              this.observed.add(op.id);
              this.resize?.observe(node);
            } else {
              this.observed.delete(op.id);
              this.resize?.unobserve(node);
            }
            break;
        }
      }
      this.seq = batch.seq;
      this.applying = false;
      this.options.send({ kind: 'applied', session: this.session, seq: batch.seq });
      for (const measure of measures)
        this.options.send({
          kind: 'layout',
          session: this.session,
          applied: this.seq,
          request: measure.request,
          entries: measure.ids.map((id) => this.snapshot(id, measure.styles))
        });
      return true;
    } catch (error) {
      this.applying = false;
      this.options.send({
        kind: 'fault',
        session: this.session,
        seq: batch.seq,
        message: error instanceof Error ? error.message : 'Page apply failure'
      });
      this.dispose();
      return false;
    }
  }
  private listen(id: number, type: string, enabled: boolean, policy: Policy) {
    let map = this.subscriptions.get(id);
    if (!map) this.subscriptions.set(id, (map = new Map()));
    if (enabled) map.set(type, policy);
    else map.delete(type);
    if (!this.handlers.has(type) && enabled) {
      const handler = ((event: Event) => this.forward(event)) as EventListener;
      for (const view of this.windows)
        view.addEventListener(type, handler, { capture: true, passive: false });
      this.handlers.set(type, handler);
    }
    if (!enabled && ![...this.subscriptions.values()].some((map) => map.has(type))) {
      const handler = this.handlers.get(type);
      if (handler) for (const view of this.windows) view.removeEventListener(type, handler, true);
      this.handlers.delete(type);
    }
  }
  private forward(event: Event) {
    if (this.retired || this.applying) return;
    const path = event.composedPath();
    let subscribed = false,
      blocked = false;
    for (const node of path) {
      const id = this.reverse.get(node!);
      const policy = id === undefined ? undefined : this.subscriptions.get(id)?.get(event.type);
      if (policy) subscribed = true;
      if (policy === 'block') blocked = true;
    }
    if (!subscribed) return;
    const target = this.reverse.get(event.target!);
    if (target === undefined) return;
    if (blocked) event.preventDefault();
    const data: Record<string, unknown> = {};
    for (const key of [
      'isTrusted',
      'key',
      'code',
      'repeat',
      'altKey',
      'ctrlKey',
      'metaKey',
      'shiftKey',
      'button',
      'buttons',
      'clientX',
      'clientY',
      'screenX',
      'screenY',
      'pointerId',
      'pointerType',
      'pressure',
      'deltaX',
      'deltaY',
      'deltaZ',
      'deltaMode',
      'inputType',
      'data',
      'isComposing',
      'detail'
    ]) {
      const value = (event as any)[key];
      if (
        value === null ||
        ['string', 'boolean'].includes(typeof value) ||
        (typeof value === 'number' && Number.isFinite(value))
      )
        data[key] = value;
    }
    const node = event.target as HTMLInputElement;
    const state: Record<string, unknown> = {};
    for (const key of [
      'value',
      'checked',
      'indeterminate',
      'selectedIndex',
      'selectionStart',
      'selectionEnd',
      'selectionDirection',
      'scrollLeft',
      'scrollTop'
    ]) {
      const value = (node as any)[key];
      if (
        value !== undefined &&
        (value === null || ['string', 'number', 'boolean'].includes(typeof value))
      )
        state[key] = value;
    }
    if (node.localName === 'select')
      state.selectedIndices = [...(node as unknown as HTMLSelectElement).options].flatMap(
        (option, i) => (option.selected ? [i] : [])
      );
    if (event.type === 'selectionchange') {
      this.selected = this.event + 1;
      // The document the selection changed in: the panel's or a sheet's.
      const target = event.target as any;
      const selection = (
        target?.getSelection ? target : (target?.ownerDocument ?? this.document)
      ).getSelection();
      data.selection = {
        anchor: selection?.anchorNode ? (this.reverse.get(selection.anchorNode) ?? null) : null,
        anchor_offset: selection?.anchorOffset ?? 0,
        focus: selection?.focusNode ? (this.reverse.get(selection.focusNode) ?? null) : null,
        focus_offset: selection?.focusOffset ?? 0
      };
    }
    if (Object.keys(state).length) {
      data.state = state;
      if ('value' in state) this.edited.set(node, this.event + 1);
    }
    if (event.type === 'focusin' || event.type === 'focusout') this.focused = this.event + 1;
    if (['focus', 'blur', 'focusin', 'focusout'].includes(event.type))
      data.relatedTarget = this.reverse.get((event as FocusEvent).relatedTarget as object) ?? null;
    this.options.send({
      kind: 'event',
      session: this.session,
      seq: ++this.event,
      applied: this.seq,
      target,
      event: event.type,
      bubbles: event.bubbles,
      cancelable: event.cancelable,
      default_prevented: event.defaultPrevented,
      data
    });
  }
  inspect() {
    return {
      session: this.session,
      applied: this.seq,
      last_event: this.event,
      nodes: this.nodes.size,
      retired: this.retired
    };
  }
  dispose() {
    this.retired = true;
    this.resize?.disconnect();
    for (const [type, handler] of this.handlers)
      for (const view of this.windows) view.removeEventListener(type, handler, true);
    this.handlers.clear();
    this.windows.clear();
    this.subscriptions.clear();
    this.nodes.clear();
    this.observed.clear();
  }
}
