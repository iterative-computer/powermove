/** Small, realm-neutral request/reply transport for extension sandboxes. */
export type HandleId = number;
export interface RpcError { name: string; message: string; code?: string }
export interface RpcTransfers { readonly __sandboxTransfers: true; readonly values: Transferable[] }
export const rpcTransfers = (...values: Transferable[]): RpcTransfers => ({ __sandboxTransfers: true, values });
export interface Rpc {
  call(method: string, ...args: unknown[]): Promise<unknown>;
  notify(method: string, ...args: unknown[]): void;
  handle(fn: (...args: any[]) => unknown): HandleId;
  invokeHandle(id: HandleId, ...args: unknown[]): Promise<unknown>;
  release(id: HandleId): void;
  close(): void;
}
export class SandboxTimeoutError extends Error {
  constructor(method: string) { super(`Sandbox call timed out: ${method}`); this.name = 'SandboxTimeoutError'; }
}
export interface RpcBudget { windowStart: number; received: number; excessSince: number; reported: boolean; handles: Set<number> }
export const createRpcBudget = (): RpcBudget => ({ windowStart: Date.now(), received: 0, excessSince: 0, reported: false, handles: new Set() });
export interface RpcLimits { maxIncomingBytes?: number; maxMirrorBytes?: number; maxIncomingPerSecond?: number; maxHandles?: number; budget?: RpcBudget; onSustainedLimit?(): void; onRemoteHandleRelease?(id: number): void }
function messageBytes(value: unknown, limit: number, depth = 0, seen = new WeakSet<object>()): number {
  if (depth > 64) return Infinity;
  if (typeof value === 'string') return value.length * 2;
  if (value === null || typeof value !== 'object') return 8;
  if (seen.has(value)) return 0;
  seen.add(value);
  if (value instanceof ArrayBuffer) return value.byteLength;
  if (ArrayBuffer.isView(value)) return value.byteLength;
  if (typeof Blob !== 'undefined' && value instanceof Blob) return value.size;
  if (value instanceof Map) {
    let bytes = 0;
    for (const [key, item] of value) { bytes += messageBytes(key, limit, depth + 1, seen) + messageBytes(item, limit, depth + 1, seen); if (bytes > limit) return bytes; }
    return bytes;
  }
  if (value instanceof Set) {
    let bytes = 0;
    for (const item of value) { bytes += messageBytes(item, limit, depth + 1, seen); if (bytes > limit) return bytes; }
    return bytes;
  }
  let bytes = 0;
  for (const [key, item] of Object.entries(value)) {
    bytes += key.length * 2 + messageBytes(item, limit, depth + 1, seen);
    if (bytes > limit) return bytes;
  }
  return bytes;
}
export function serializeRpcError(error: unknown): RpcError {
  const value = error as { name?: unknown; message?: unknown; code?: unknown };
  return { name: typeof value?.name === 'string' ? value.name : 'Error', message: typeof value?.message === 'string' ? value.message : String(error), ...(typeof value?.code === 'string' ? { code: value.code } : {}) };
}
export function createRpc(port: MessagePort, handlers: Record<string, (...args: any[]) => unknown>, timeoutMs = 10_000, limits: RpcLimits = {}): Rpc {
  let sequence = 0;
  const handleBase = Math.floor(Math.random() * 1_000_000_000) * 1_000_000;
  let handleSequence = 0;
  let closed = false;
  const functions = new Map<HandleId, (...args: any[]) => unknown>();
  const budget = limits.budget ?? createRpcBudget();
  const pending = new Map<number, { resolve(value: unknown): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> }>();
  const post = (message: unknown, transfer: Transferable[] = []): void => {
    if (closed) throw new Error('Sandbox RPC is closed');
    port.postMessage(message, transfer);
  };
  const splitTransfers = (input: unknown[]): { args: unknown[]; transfers: Transferable[] } => {
    const args = [...input];
    const last = args.at(-1) as RpcTransfers | undefined;
    if (last?.__sandboxTransfers === true && Array.isArray(last.values)) { args.pop(); return { args, transfers: last.values }; }
    return { args, transfers: [] };
  };
  const request = (message: Record<string, unknown>, label: string, transfers: Transferable[] = []): Promise<unknown> => new Promise((resolve, reject) => {
    if (closed) { reject(new Error('Sandbox RPC is closed')); return; }
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new SandboxTimeoutError(label)); }, timeoutMs);
    pending.set(id, { resolve, reject, timer });
    try { post({ ...message, id }, transfers); }
    catch (error) { clearTimeout(timer); pending.delete(id); reject(error); }
  });
  const onMessage = (event: MessageEvent): void => {
    const message = event.data as Record<string, any>;
    if (!message || typeof message !== 'object' || closed) return;
    const now = Date.now();
    if (now - budget.windowStart >= 1000) { if (budget.received <= (limits.maxIncomingPerSecond ?? 200)) budget.excessSince = 0; budget.windowStart = now; budget.received = 0; }
    const byteLimit = message.t === 'notify' && message.m === 'mirror' ? (limits.maxMirrorBytes ?? limits.maxIncomingBytes ?? 1024 * 1024) : (limits.maxIncomingBytes ?? 1024 * 1024);
    const tooLarge = messageBytes(message, byteLimit) > byteLimit;
    // Replies to our own requests are already bounded by `pending`; unknown
    // replies consume the same budget as calls and notifications.
    const unsolicited = message.t !== 'reply' || !pending.has(message.id);
    const tooFast = unsolicited && ++budget.received > (limits.maxIncomingPerSecond ?? 200);
    if (tooLarge || tooFast) {
      if (tooFast) { if (!budget.excessSince) budget.excessSince = now; if (!budget.reported && now - budget.excessSince >= 3000) { budget.reported = true; limits.onSustainedLimit?.(); } }
      const error = { name: 'ResourceLimitError', message: tooLarge ? 'Sandbox RPC payload exceeds 1 MiB or nesting limit' : 'Sandbox RPC rate exceeds 200 messages per second', code: 'resource_limit' };
      if (message.t === 'reply') {
        const entry = pending.get(message.id);
        if (entry) { clearTimeout(entry.timer); pending.delete(message.id); const failure = new Error(error.message) as Error & { code: string }; failure.name = error.name; failure.code = error.code; entry.reject(failure); }
      } else if (!closed && (message.t === 'call' || message.t === 'handle-call')) post({ t: 'reply', id: message.id, ok: false, e: error });
      return;
    }
    if (message.t === 'reply') {
      const entry = pending.get(message.id);
      if (!entry) return;
      clearTimeout(entry.timer); pending.delete(message.id);
      if (message.ok) entry.resolve(message.v);
      else { const error = new Error(message.e?.message ?? 'Sandbox call failed'); error.name = message.e?.name ?? 'Error'; if (message.e?.code) (error as Error & { code: string }).code = message.e.code; entry.reject(error); }
      return;
    }
    if (message.t === 'handle-release') { functions.delete(message.h); budget.handles.delete(message.h); limits.onRemoteHandleRelease?.(message.h); return; }
    if (message.t === 'notify') { try { if (Object.hasOwn(handlers, message.m)) void handlers[message.m]?.(...(message.a ?? [])); } catch { /* notifications cannot reply */ } return; }
    if (message.t !== 'call' && message.t !== 'handle-call') return;
    const fn = message.t === 'call' && Object.hasOwn(handlers, message.m) ? handlers[message.m] : message.t === 'handle-call' ? functions.get(message.h) : undefined;
    Promise.resolve().then(() => {
      if (typeof fn !== 'function') throw new Error(`Unknown sandbox method: ${message.m ?? message.h}`);
      return fn(...(message.a ?? []));
    }).then(v => post({ t: 'reply', id: message.id, ok: true, v }), e => post({ t: 'reply', id: message.id, ok: false, e: serializeRpcError(e) }));
  };
  port.addEventListener('message', onMessage as EventListener);
  port.start();
  return {
    call: (m, ...input) => { const { args, transfers } = splitTransfers(input); return request({ t: 'call', m, a: args }, m, transfers); },
    notify: (m, ...input) => { const { args, transfers } = splitTransfers(input); post({ t: 'notify', m, a: args }, transfers); },
    handle(fn) { if (budget.handles.size >= (limits.maxHandles ?? 2000)) throw new Error('Sandbox handle limit is 2000'); const id = handleBase + ++handleSequence; functions.set(id, fn); budget.handles.add(id); return id; },
    invokeHandle: (h, ...input) => { const { args, transfers } = splitTransfers(input); return request({ t: 'handle-call', h, a: args }, `handle ${h}`, transfers); },
    release(h) { functions.delete(h); budget.handles.delete(h); post({ t: 'handle-release', h }); },
    close() { if (closed) return; closed = true; port.removeEventListener('message', onMessage as EventListener); for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(new Error('Sandbox RPC closed')); } pending.clear(); for (const id of functions.keys()) budget.handles.delete(id); functions.clear(); port.close(); }
  };
}
