import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  startExtensionWatcher,
  type ExtensionWatcherRegistry,
  type StartExtensionWatcherOptions
} from './watcher';

type WatchFactory = NonNullable<StartExtensionWatcherOptions['watch']>;
type WatchListener = Parameters<WatchFactory>[2];

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function harness(overrides: Partial<ExtensionWatcherRegistry> = {}): {
  registry: ExtensionWatcherRegistry;
  watchDirectory: ReturnType<typeof vi.fn<WatchFactory>>;
  listener: () => WatchListener;
  close: ReturnType<typeof vi.fn>;
} {
  let eventListener: WatchListener | undefined;
  const close = vi.fn();
  const watchDirectory = vi.fn<WatchFactory>((_dir, _options, nextListener) => {
    eventListener = nextListener;
    return { close, on: vi.fn() };
  });
  const registry: ExtensionWatcherRegistry = {
    refresh: vi.fn().mockResolvedValue(undefined),
    emitChanged: vi.fn(),
    ...overrides
  };
  return {
    registry,
    watchDirectory,
    listener: () => {
      if (!eventListener) throw new Error('watch listener was not installed');
      return eventListener;
    },
    close
  };
}

describe('extension watcher', () => {
  it('debounces events for affected extension ids into one sorted refresh', async () => {
    vi.useFakeTimers();
    const setup = harness();
    startExtensionWatcher({
      userDir: '/tmp/extensions',
      buildDir: '/tmp/extensions-build',
      registry: setup.registry,
      watch: setup.watchDirectory
    });

    setup.listener()('change', 'zulu-ext/index.ts');
    setup.listener()('rename', 'alpha-ext/manifest.json');
    setup.listener()('change', 'zulu-ext/component.svelte');
    await vi.advanceTimersByTimeAsync(299);
    expect(setup.registry.refresh).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    expect(setup.registry.refresh).toHaveBeenCalledOnce();
    expect(setup.registry.refresh).toHaveBeenCalledWith(['alpha-ext', 'zulu-ext']);
    expect(setup.registry.emitChanged).toHaveBeenCalledWith({
      ids: ['alpha-ext', 'zulu-ext'],
      reason: 'watch'
    });
  });

  it('ignores invalid immediate ids, dot paths, null filenames, and paths outside the root', async () => {
    vi.useFakeTimers();
    const setup = harness();
    startExtensionWatcher({
      userDir: '/tmp/extensions',
      buildDir: '/tmp/extensions-build',
      registry: setup.registry,
      watch: setup.watchDirectory
    });

    setup.listener()('change', null);
    setup.listener()('change', '.hidden/index.ts');
    setup.listener()('change', 'Uppercase/index.ts');
    setup.listener()('change', 'a/index.ts');
    setup.listener()('change', '../outside/index.ts');
    await vi.advanceTimersByTimeAsync(500);

    expect(setup.registry.refresh).not.toHaveBeenCalled();
    expect(setup.registry.emitChanged).not.toHaveBeenCalled();
  });

  it('ignores generated build paths inside the watched source root', async () => {
    vi.useFakeTimers();
    const setup = harness();
    startExtensionWatcher({
      userDir: '/tmp/extensions',
      buildDir: '/tmp/extensions/extensions-build',
      registry: setup.registry,
      watch: setup.watchDirectory
    });

    setup.listener()('change', 'extensions-build/example-ext/bundle.js');
    await vi.advanceTimersByTimeAsync(500);

    expect(setup.registry.refresh).not.toHaveBeenCalled();
  });

  it('survives watch startup failures and reports them', () => {
    const error = new Error('recursive watch unsupported');
    const onError = vi.fn();
    const watchDirectory = vi.fn<WatchFactory>(() => {
      throw error;
    });
    const registry = harness().registry;

    expect(() =>
      startExtensionWatcher({
        userDir: '/missing/extensions',
        buildDir: '/tmp/extensions-build',
        registry,
        watch: watchDirectory,
        onError
      })
    ).not.toThrow();
    expect(onError).toHaveBeenCalledWith(error);
  });

  it('contains refresh failures and does not broadcast a stale change', async () => {
    vi.useFakeTimers();
    const error = new Error('refresh failed');
    const onError = vi.fn();
    const setup = harness({ refresh: vi.fn().mockRejectedValue(error) });
    startExtensionWatcher({
      userDir: '/tmp/extensions',
      buildDir: '/tmp/extensions-build',
      registry: setup.registry,
      watch: setup.watchDirectory,
      onError
    });

    setup.listener()('change', 'example-ext/index.ts');
    await vi.advanceTimersByTimeAsync(300);
    await Promise.resolve();

    expect(onError).toHaveBeenCalledWith(error);
    expect(setup.registry.emitChanged).not.toHaveBeenCalled();
  });

  it('close and dispose are idempotent and cancel pending work', async () => {
    vi.useFakeTimers();
    const setup = harness();
    const watcher = startExtensionWatcher({
      userDir: '/tmp/extensions',
      buildDir: '/tmp/extensions-build',
      registry: setup.registry,
      watch: setup.watchDirectory
    });
    setup.listener()('change', 'example-ext/index.ts');

    watcher.dispose();
    watcher.close();
    await vi.advanceTimersByTimeAsync(500);

    expect(setup.close).toHaveBeenCalledOnce();
    expect(setup.registry.refresh).not.toHaveBeenCalled();
  });
});
