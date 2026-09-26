/*
 * Editor-window registry. Powermove runs any number of windows, and each
 * window holds a strip of project tabs with one of them on screen. This module
 * owns the part of that model which is not Electron: which window holds which
 * projects, which window the menu should talk to, and the windows and tabs to
 * reopen at the next launch.
 *
 * A project is never open in two windows at once — two editors autosaving into
 * one storage slot would race and lose edits — so `claim` refuses a project
 * another window has as a tab, and callers raise that window instead.
 */

/** The slice of BrowserWindow this registry needs. Kept structural so the
 *  ordering and ownership rules can be tested without Electron. */
export interface ManagedWindow {
  isDestroyed(): boolean;
  isMinimized(): boolean;
  restore(): void;
  show(): void;
  focus(): void;
}

/** One window's tabs in strip order, and the one it is showing. `active` is
 *  null while the window shows Projects with nothing behind it. */
export interface WindowSession {
  tabs: string[];
  active: string | null;
}

export class EditorWindows<W extends ManagedWindow> {
  /** Most recently focused first, so the menu and `activate` agree on "the"
   *  window whenever Electron reports no focused window of its own. */
  private readonly recent: W[] = [];
  /** Window creation order, which is the order windows are restored in. */
  private readonly created: W[] = [];
  private readonly sessions = new Map<W, WindowSession>();

  /** `quiet` keeps every window where it is: the hidden background-test app
   *  must never show or focus one on the user's desktop. */
  constructor(private readonly options: { quiet?: boolean } = {}) {}

  add(window: W, projectId: string | null = null, tabs: readonly string[] = []): void {
    if (this.sessions.has(window)) return;
    const ids = [...new Set(tabs.filter((id) => typeof id === 'string' && id))];
    if (projectId && !ids.includes(projectId)) ids.push(projectId);
    this.sessions.set(window, { tabs: ids, active: projectId });
    this.created.push(window);
    this.recent.unshift(window);
  }

  remove(window: W): void {
    this.sessions.delete(window);
    for (const list of [this.recent, this.created]) {
      const index = list.indexOf(window);
      if (index >= 0) list.splice(index, 1);
    }
  }

  /** Records a focus so `mostRecent` reflects what the user was last in. */
  touch(window: W): void {
    if (!this.sessions.has(window)) return;
    const index = this.recent.indexOf(window);
    if (index > 0) this.recent.splice(index, 1);
    if (index !== 0) this.recent.unshift(window);
  }

  has(window: W): boolean {
    return this.sessions.has(window);
  }

  get size(): number {
    return this.live(this.created).length;
  }

  all(): W[] {
    return this.live(this.created);
  }

  mostRecent(): W | null {
    return this.live(this.recent)[0] ?? null;
  }

  /** Front-most first, which is the order a point on screen hits them in. */
  byRecency(): W[] {
    return this.live(this.recent);
  }

  /** The project the window is showing. */
  projectOf(window: W): string | null {
    return this.sessions.get(window)?.active ?? null;
  }

  /** The window's tabs in strip order. */
  tabsOf(window: W): string[] {
    return [...(this.sessions.get(window)?.tabs ?? [])];
  }

  windowForProject(projectId: string): W | null {
    if (!projectId) return null;
    for (const window of this.live(this.recent)) {
      if (this.sessions.get(window)?.tabs.includes(projectId)) return window;
    }
    return null;
  }

  /**
   * Shows a project in a window, adding it as the last tab when the window
   * does not have it yet. Null shows nothing and keeps every tab. Returns false
   * when a different live window already holds that project, which is the
   * caller's signal to raise that window rather than load a second copy.
   */
  claim(window: W, projectId: string | null): boolean {
    const session = this.sessions.get(window);
    if (!session) return false;
    if (projectId) {
      const holder = this.windowForProject(projectId);
      if (holder && holder !== window) return false;
      if (!session.tabs.includes(projectId)) session.tabs.push(projectId);
    }
    session.active = projectId;
    return true;
  }

  /** Closes a tab, freeing its project for any window to open. */
  release(window: W, projectId: string): boolean {
    const session = this.sessions.get(window);
    const index = session?.tabs.indexOf(projectId) ?? -1;
    if (!session || index < 0) return false;
    session.tabs.splice(index, 1);
    if (session.active === projectId) session.active = null;
    return true;
  }

  /** Puts a window's tabs in the order the user dragged them into. Ids the
   *  window does not hold are ignored and tabs left out keep their place at
   *  the end, so a stale request can reorder but never add or drop a tab. */
  reorder(window: W, order: readonly string[]): boolean {
    const session = this.sessions.get(window);
    if (!session) return false;
    const ordered = [...new Set(order)].filter((id) => session.tabs.includes(id));
    session.tabs = [...ordered, ...session.tabs.filter((id) => !ordered.includes(id))];
    return true;
  }

  /** Every project with a tab anywhere, in window then tab order. */
  openProjectIds(): string[] {
    const ids: string[] = [];
    for (const window of this.live(this.created)) {
      for (const id of this.sessions.get(window)?.tabs ?? []) if (!ids.includes(id)) ids.push(id);
    }
    return ids;
  }

  /** Windows with at least one tab, in window order — the launch restore list. */
  openSessions(): WindowSession[] {
    const sessions: WindowSession[] = [];
    for (const window of this.live(this.created)) {
      const session = this.sessions.get(window);
      if (session?.tabs.length) sessions.push({ tabs: [...session.tabs], active: session.active });
    }
    return sessions;
  }

  /** Raises a window the way clicking its Dock icon would. */
  reveal(window: W | null): boolean {
    if (!window || window.isDestroyed()) return false;
    if (this.options.quiet) return true;
    if (window.isMinimized()) window.restore();
    window.show();
    window.focus();
    return true;
  }

  private live(list: W[]): W[] {
    return list.filter((window) => this.sessions.has(window) && !window.isDestroyed());
  }
}

/**
 * The windows and tabs a launch should reopen, newest profile key first:
 * per-window tabs, then the one-project-per-window list, then the tab strip of
 * the single-window profiles, which comes back as one window of tabs.
 * Ordering and de-duplication happen here so a hand-edited or migrated store
 * cannot ask for the same document in two places.
 */
export function restorableSessions(
  windowTabs: unknown,
  openWindows: unknown,
  legacyOpenTabs: unknown,
  isKnown: (id: string) => boolean,
  { windows: windowLimit = 12, tabs: tabLimit = 40 }: { windows?: number; tabs?: number } = {}
): WindowSession[] {
  const seen = new Set<string>();
  const take = (entry: unknown): string | null => {
    if (typeof entry !== 'string' || !entry || seen.has(entry) || !isKnown(entry) || seen.size >= tabLimit) return null;
    seen.add(entry);
    return entry;
  };
  const sessions: WindowSession[] = [];
  const push = (tabs: string[], active: unknown): void => {
    if (!tabs.length || sessions.length >= windowLimit) return;
    sessions.push({ tabs, active: typeof active === 'string' && tabs.includes(active) ? active : tabs[tabs.length - 1]! });
  };

  if (Array.isArray(windowTabs) && windowTabs.length) {
    for (const entry of windowTabs) {
      const record = entry && typeof entry === 'object' ? entry as { tabs?: unknown; active?: unknown } : null;
      const tabs = Array.isArray(record?.tabs) ? record.tabs.map(take).filter((id): id is string => !!id) : [];
      push(tabs, record?.active);
    }
    if (sessions.length) return sessions;
  }
  if (Array.isArray(openWindows) && openWindows.length) {
    for (const entry of openWindows) {
      const id = take(entry);
      if (id) push([id], id);
    }
    return sessions;
  }
  if (Array.isArray(legacyOpenTabs)) {
    push(legacyOpenTabs.map(take).filter((id): id is string => !!id), null);
  }
  return sessions;
}

export interface ScreenRect { x: number; y: number; width: number; height: number }

/** How far below a window's top edge a dropped tab still lands in its strip.
 *  A little deeper than the 44px titlebar, so a drop need not be exact. */
export const TAB_STRIP_BAND = 56;

/**
 * The window whose tab strip a tab dragged out of `from` was dropped on, if
 * any. `windows` is front-most first, so where windows overlap the one on top
 * takes it, and a drop into the body of a window in front of a strip is not a
 * drop on the strip behind it.
 */
export function windowUnderTabDrop<W>(
  windows: readonly W[],
  from: W,
  point: { x: number; y: number },
  boundsOf: (window: W) => ScreenRect | null
): W | null {
  for (const window of windows) {
    const bounds = boundsOf(window);
    if (!bounds) continue;
    const inside = point.x >= bounds.x && point.x < bounds.x + bounds.width
      && point.y >= bounds.y && point.y < bounds.y + bounds.height;
    if (!inside) continue;
    if (window === from) return null;
    return point.y < bounds.y + TAB_STRIP_BAND ? window : null;
  }
  return null;
}
