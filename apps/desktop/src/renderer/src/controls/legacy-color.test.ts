// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const components = vi.hoisted(() => ({ mount: vi.fn(), unmount: vi.fn() }));
vi.mock('svelte', async (original) => ({ ...(await original<typeof import('svelte')>()), ...components }));
const NativeMutationObserver = globalThis.MutationObserver;

class LifecycleObserver {
  static instances: LifecycleObserver[] = [];
  disconnected = false;
  deliveries = 0;
  records: MutationRecord[] = [];
  observe = vi.fn();
  takeRecords = vi.fn(() => this.records.splice(0));
  disconnect = vi.fn(() => { this.disconnected = true; });
  constructor(readonly callback: MutationCallback) { LifecycleObserver.instances.push(this); }
  deliver(records: MutationRecord[]): void {
    if (this.disconnected) return;
    this.deliveries++;
    this.callback(records, this as unknown as MutationObserver);
  }
}

const change = (added: Node[] = [], removed: Node[] = []) => ({
  type: 'childList', target: document.body, addedNodes: added, removedNodes: removed,
}) as unknown as MutationRecord;
const deliver = (...records: MutationRecord[]) => {
  for (const observer of LifecycleObserver.instances) observer.deliver(records);
};

let field: typeof import('./legacy-color.svelte')['legacyColorField'];
const PM: any = {};
const create = () => field(PM, false, () => '#ff0000', () => {}, { local: true });

beforeEach(async () => {
  vi.resetModules(); vi.clearAllMocks();
  LifecycleObserver.instances = [];
  vi.stubGlobal('MutationObserver', LifecycleObserver);
  document.body.replaceChildren();
  components.mount.mockImplementation(() => ({}));
  components.unmount.mockResolvedValue(undefined);
  field = (await import('./legacy-color.svelte')).legacyColorField;
});
afterEach(() => {
  document.body.replaceChildren();
  deliver(change());
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('legacy color control lifecycle', () => {
  it('unmounts a control added and removed before mutation delivery', () => {
    const target = create();
    document.body.append(target); target.remove();
    deliver(change([target]), change([], [target]));
    expect(components.unmount).toHaveBeenCalledExactlyOnceWith(components.mount.mock.results[0]!.value);
    expect(LifecycleObserver.instances[0]!.disconnect).toHaveBeenCalledTimes(1);
    deliver(change());
    expect(components.unmount).toHaveBeenCalledTimes(1);
  });

  it('detects an ancestor added and removed in the same delivery', () => {
    const target = create(), container = document.createElement('div');
    container.append(target); document.body.append(container); container.remove();
    deliver(change([container]), change([], [container]));
    expect(components.unmount).toHaveBeenCalledTimes(1);
  });

  it('cleans up a same-turn removal through a real MutationObserver', async () => {
    vi.stubGlobal('MutationObserver', NativeMutationObserver);
    const target = create(); document.body.append(target); target.remove();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(components.unmount).toHaveBeenCalledTimes(1);
  });

  it('preserves fields moved or reattached before delivery and disposes after their final removal', () => {
    const target = create(), first = document.createElement('div'), second = document.createElement('div');
    document.body.append(first, second); first.append(target);
    deliver(change([first, second]), change([target]));
    second.append(target);
    deliver(change([], [target]), change([target]));
    target.remove(); first.append(target);
    deliver(change([], [target]), change([target]));
    expect(components.unmount).not.toHaveBeenCalled();
    first.remove();
    deliver(change([], [first]));
    expect(components.unmount).toHaveBeenCalledTimes(1);
  });

  it('waits for initially detached controls to be attached', () => {
    const target = create(), unrelated = document.createElement('div');
    document.body.append(unrelated); deliver(change([unrelated]));
    expect(components.unmount).not.toHaveBeenCalled();
    document.body.append(target); deliver(change([target]));
    target.remove(); deliver(change([], [target]));
    expect(components.unmount).toHaveBeenCalledTimes(1);
  });

  it('ignores mutations that happened before a field was created inside a detached container', async () => {
    const keeper = create(); document.body.append(keeper); deliver(change([keeper]));
    const container = document.createElement('div'); document.body.append(container); container.remove();
    LifecycleObserver.instances[0]!.records = [change([container]), change([], [container])];
    const target = create(); container.append(target);
    await Promise.resolve();
    expect(components.unmount).not.toHaveBeenCalled();
    document.body.append(container); deliver(change([container]));
    container.remove(); deliver(change([], [container]));
    expect(components.unmount).toHaveBeenCalledTimes(1);
  });

  it('preserves a never-connected field added to a container after its document removal', () => {
    const target = create(), container = document.createElement('div');
    document.body.append(container); container.remove(); container.append(target);
    // Detached subtrees remain observed until removal notification, so this
    // final addition is delivered although the control never entered the page.
    deliver(change([container]), change([], [container]), {
      ...change([target]), target: container,
    } as MutationRecord);
    expect(components.unmount).not.toHaveBeenCalled();
    document.body.append(container); deliver(change([container]));
    expect(components.unmount).not.toHaveBeenCalled();
    container.remove(); deliver(change([], [container]));
    expect(components.unmount).toHaveBeenCalledTimes(1);
  });

  it('distinguishes real ephemeral connection from later detached insertion in the same batch', () => {
    const attached = create(), detached = create(), container = document.createElement('div');
    container.append(attached); document.body.append(container); container.remove(); container.append(detached);
    deliver(change([container]), change([], [container]), {
      ...change([detached]), target: container,
    } as MutationRecord);
    expect(components.unmount).toHaveBeenCalledExactlyOnceWith(components.mount.mock.results[0]!.value);
    expect(LifecycleObserver.instances[0]!.disconnect).not.toHaveBeenCalled();
    document.body.append(container); deliver(change([container]));
    container.remove(); deliver(change([], [container]));
    expect(components.unmount).toHaveBeenCalledTimes(2);
  });

  it('shares one observer and one callback delivery across 200 fields and releases it when empty', () => {
    const targets = Array.from({ length: 200 }, create);
    document.body.append(...targets); deliver(change(targets));
    expect(LifecycleObserver.instances).toHaveLength(1);
    expect(LifecycleObserver.instances[0]!.deliveries).toBe(1);
    targets[0]!.remove(); deliver(change([], [targets[0]!]));
    expect(components.unmount).toHaveBeenCalledTimes(1);
    expect(LifecycleObserver.instances[0]!.disconnect).not.toHaveBeenCalled();
    document.body.replaceChildren(); deliver(change([], targets.slice(1)));
    expect(components.unmount).toHaveBeenCalledTimes(200);
    expect(LifecycleObserver.instances[0]!.disconnect).toHaveBeenCalledTimes(1);
    const next = create(); document.body.append(next); deliver(change([next]));
    expect(LifecycleObserver.instances).toHaveLength(2);
    next.remove(); deliver(change([], [next]));
    expect(components.unmount).toHaveBeenCalledTimes(201);
  });

  it('continues cleaning other controls when one component cleanup throws', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const targets = [create(), create()];
    document.body.append(...targets); deliver(change(targets));
    components.unmount.mockImplementationOnce(() => { throw new Error('Broken cleanup'); });
    document.body.replaceChildren(); deliver(change([], targets));
    expect(components.unmount).toHaveBeenCalledTimes(2);
    expect(LifecycleObserver.instances[0]!.disconnect).toHaveBeenCalledTimes(1);
  });

  it('preserves live color/fill reads, writes and sync updates while attached', () => {
    let color = '#ff0000';
    const write = vi.fn((value: unknown) => { color = String(value); });
    const target = field(PM, true, () => color, write, { local: true, fallback: '#000000' });
    const props = components.mount.mock.calls[0]![1].props;
    document.body.append(target); deliver(change([target]));
    expect(props.get()).toBe('#ff0000');
    props.edit.set('#00ff00');
    expect(write).toHaveBeenCalledExactlyOnceWith('#00ff00');
    expect(props.get()).toBe('#00ff00');
    color = '#0000ff'; target.sync();
    expect(props.get()).toBe('#0000ff');
    expect(props.fallback).toBe('#000000');
    expect(components.mount).toHaveBeenCalledTimes(1);
    expect(components.unmount).not.toHaveBeenCalled();
    target.remove(); deliver(change([], [target]));
    expect(components.unmount).toHaveBeenCalledTimes(1);
  });
});
