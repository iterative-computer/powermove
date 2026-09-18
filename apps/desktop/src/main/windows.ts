/*
 * Editor-window registry. Powermove used to be a single window with a strip of
 * project tabs; it is now one project per window, with as many windows as the
 * user wants. This module owns the part of that model which is not Electron:
 * which window holds which project, which window the menu should talk to, and
 * the ordered list of projects to reopen at the next launch.
 *
 * A project is never open in two windows at once — two editors autosaving into
 * one storage slot would race and lose edits — so `claim` refuses a duplicate
 * and callers raise the window that already has the document instead.
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

export class EditorWindows<W extends ManagedWindow> {
  /** Most recently focused first, so the menu and `activate` agree on "the"
   *  window whenever Electron reports no focused window of its own. */
  private readonly recent: W[] = [];
  /** Window creation order, which is the order windows are restored in. */
  private readonly created: W[] = [];
  private readonly projects = new Map<W, string | null>();

  add(window: W, projectId: string | null = null): void {
    if (this.projects.has(window)) return;
    this.projects.set(window, projectId);
    this.created.push(window);
    this.recent.unshift(window);
  }

  remove(window: W): void {
    this.projects.delete(window);
    for (const list of [this.recent, this.created]) {
      const index = list.indexOf(window);
      if (index >= 0) list.splice(index, 1);
    }
  }

  /** Records a focus so `mostRecent` reflects what the user was last in. */
  touch(window: W): void {
    if (!this.projects.has(window)) return;
    const index = this.recent.indexOf(window);
    if (index > 0) this.recent.splice(index, 1);
    if (index !== 0) this.recent.unshift(window);
  }

  has(window: W): boolean {
    return this.projects.has(window);
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

  projectOf(window: W): string | null {
    return this.projects.get(window) ?? null;
  }

  windowForProject(projectId: string): W | null {
    if (!projectId) return null;
    for (const window of this.live(this.recent)) {
      if (this.projects.get(window) === projectId) return window;
    }
    return null;
  }

  /**
   * Binds a project to a window. Returns false when a different live window
   * already holds that project, which is the caller's signal to raise that
   * window rather than load a second copy.
   */
  claim(window: W, projectId: string | null): boolean {
    if (!this.projects.has(window)) return false;
    if (projectId) {
      const holder = this.windowForProject(projectId);
      if (holder && holder !== window) return false;
    }
    this.projects.set(window, projectId);
    return true;
  }

  /** Projects with a window, in window order — the launch restore list. */
  openProjectIds(): string[] {
    const ids: string[] = [];
    for (const window of this.live(this.created)) {
      const id = this.projects.get(window);
      if (id && !ids.includes(id)) ids.push(id);
    }
    return ids;
  }

  /** Raises a window the way clicking its Dock icon would. */
  reveal(window: W | null): boolean {
    if (!window || window.isDestroyed()) return false;
    if (window.isMinimized()) window.restore();
    window.show();
    window.focus();
    return true;
  }

  private live(list: W[]): W[] {
    return list.filter((window) => this.projects.has(window) && !window.isDestroyed());
  }
}

/**
 * The projects a launch should reopen, newest profile key first. Ordering and
 * de-duplication happen here so a hand-edited or migrated store cannot ask for
 * the same document in two windows.
 */
export function restorableProjects(
  openWindows: unknown,
  legacyOpenTabs: unknown,
  isKnown: (id: string) => boolean,
  limit = 12
): string[] {
  const source = Array.isArray(openWindows) && openWindows.length ? openWindows : legacyOpenTabs;
  if (!Array.isArray(source)) return [];
  const ids: string[] = [];
  for (const entry of source) {
    if (typeof entry !== 'string' || !entry || ids.includes(entry) || !isKnown(entry)) continue;
    ids.push(entry);
    if (ids.length >= limit) break;
  }
  return ids;
}
