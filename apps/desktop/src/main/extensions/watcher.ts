import { watch, type FSWatcher } from 'node:fs';
import path from 'node:path';

import {
  EXTENSION_ID,
  type ExtensionsChangedEvent
} from '../../shared/extensions';

export interface ExtensionWatcherRegistry {
  refresh(ids: string[]): Promise<void> | void;
  emitChanged(event: ExtensionsChangedEvent): void;
}

export interface ExtensionWatcher {
  close(): void;
  dispose(): void;
}

type WatchListener = (eventType: string, filename: string | Buffer | null) => void;
type WatchHandle = Pick<FSWatcher, 'close'> & {
  on?(event: 'error', listener: (error: Error) => void): unknown;
};

export interface StartExtensionWatcherOptions {
  userDir: string;
  buildDir: string;
  registry: ExtensionWatcherRegistry;
  debounceMs?: number;
  onError?: (error: unknown) => void;
  /** Injectable for deterministic tests. Defaults to node:fs watch. */
  watch?: (dir: string, options: { recursive: true }, listener: WatchListener) => WatchHandle;
}

/** Watches user extension sources and batches rebuilds by immediate child id. */
export function startExtensionWatcher({
  userDir,
  buildDir,
  registry,
  debounceMs = 300,
  onError = () => undefined,
  watch: watchDirectory = defaultWatch
}: StartExtensionWatcherOptions): ExtensionWatcher {
  const sourceRoot = path.resolve(userDir);
  const buildRoot = path.resolve(buildDir);
  const pendingIds = new Set<string>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let watcher: WatchHandle | undefined;
  let closed = false;

  const reportError = (error: unknown): void => {
    try {
      onError(error);
    } catch {
      // Diagnostics must not turn a recoverable watcher failure into a crash.
    }
  };

  const flush = async (): Promise<void> => {
    timer = undefined;
    if (closed || pendingIds.size === 0) return;
    const ids = [...pendingIds].sort(compareText);
    pendingIds.clear();

    try {
      await registry.refresh(ids);
      if (!closed) registry.emitChanged({ ids, reason: 'watch' });
    } catch (error) {
      reportError(error);
    }
  };

  const schedule = (_eventType: string, filename: string | Buffer | null): void => {
    if (closed || filename === null) return;
    const changedPath = resolveChangedPath(sourceRoot, filename.toString());
    if (!changedPath || isWithin(buildRoot, changedPath)) return;

    const relative = path.relative(sourceRoot, changedPath);
    const id = relative.split(/[\\/]/, 1)[0];
    if (!id || !EXTENSION_ID.test(id)) return;

    pendingIds.add(id);
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      void flush();
    }, debounceMs);
  };

  try {
    watcher = watchDirectory(sourceRoot, { recursive: true }, schedule);
    watcher.on?.('error', reportError);
  } catch (error) {
    reportError(error);
  }

  const close = (): void => {
    if (closed) return;
    closed = true;
    pendingIds.clear();
    if (timer) clearTimeout(timer);
    timer = undefined;
    try {
      watcher?.close();
    } catch (error) {
      reportError(error);
    }
  };

  return { close, dispose: close };
}

function defaultWatch(
  dir: string,
  options: { recursive: true },
  listener: WatchListener
): FSWatcher {
  return watch(dir, options, listener);
}

function resolveChangedPath(root: string, filename: string): string | null {
  if (filename.length === 0) return null;
  const resolved = path.resolve(root, filename);
  return isWithin(root, resolved) ? resolved : null;
}

function isWithin(parent: string, candidate: string): boolean {
  const relative = path.relative(parent, candidate);
  return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative));
}

function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}
