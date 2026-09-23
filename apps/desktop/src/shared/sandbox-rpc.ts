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
export function serializeRpcError(error: unknown): RpcError {
  const value = error as { name?: unknown; message?: unknown; code?: unknown };
  return { name: typeof value?.name === 'string' ? value.name : 'Error', message: typeof value?.message === 'string' ? value.message : String(error), ...(typeof value?.code === 'string' ? { code: value.code } : {}) };
}
export function createRpc(port: MessagePort, handlers: Record<string, (...args: any[]) => unknown>, timeoutMs = 10_000): Rpc {
  let sequence = 0;
  const handleBase = Math.floor(Math.random() * 1_000_000_000) * 1_000_000;
  let handleSequence = 0;
  let closed = false;
  const functions = new Map<HandleId, (...args: any[]) => unknown>();
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
    if (message.t === 'reply') {
      const entry = pending.get(message.id);
      if (!entry) return;
      clearTimeout(entry.timer); pending.delete(message.id);
      if (message.ok) entry.resolve(message.v);
      else { const error = new Error(message.e?.message ?? 'Sandbox call failed'); error.name = message.e?.name ?? 'Error'; if (message.e?.code) (error as Error & { code: string }).code = message.e.code; entry.reject(error); }
      return;
    }
    if (message.t === 'handle-release') { functions.delete(message.h); return; }
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
    handle(fn) { const id = handleBase + ++handleSequence; functions.set(id, fn); return id; },
    invokeHandle: (h, ...input) => { const { args, transfers } = splitTransfers(input); return request({ t: 'handle-call', h, a: args }, `handle ${h}`, transfers); },
    release(h) { functions.delete(h); post({ t: 'handle-release', h }); },
    close() { if (closed) return; closed = true; port.removeEventListener('message', onMessage as EventListener); for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(new Error('Sandbox RPC closed')); } pending.clear(); functions.clear(); port.close(); }
  };
}
