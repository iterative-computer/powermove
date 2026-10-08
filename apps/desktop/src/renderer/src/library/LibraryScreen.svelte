<script lang="ts">
  import { flushSync } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import Scritto from '@scritto/svelte';
  import BackTitle from '../controls/BackTitle.svelte';
  import { pageFly, pageSettle } from '../controls/page-motion';
  import Icon from '../panels/Icon.svelte';
  import Conversation from '../panels/agent/Conversation.svelte';
  import Composer from '../panels/agent/Composer.svelte';
  import ThreadPicker from '../panels/agent/ThreadPicker.svelte';
  import { agentState } from '../panels/agent/agent-state.svelte';
  import { panelIcons } from '../panels/panel-icons';
  import { panelScope } from '../panels/agent/panel-focus';
  import { deletePanel, deletedPanelIds } from './panel-deletion';
  import { panelBelongsInLibrary, panelPreviewSize } from './panel-preview';
  import { panelFrameOf } from '../kernel/panel-frame';
  import { placePanel } from '../layout/portal';
  import { artFor, statusText, storeBridge } from '../store/data';
  import type { LibraryItemDto } from '../../../shared/store-ipc';
  import { coverSize, MAX_PREVIEW_HEIGHT, MAX_PREVIEW_WIDTH, packRows, panelSource, shelfScale, SHELF_LABEL, SHELF_ORDER, type CoverSize, type ShelfId } from './shelves';

  let { PM }: { PM: Record<string, any> } = $props();

  let shown = $state(false);
  let view = $state<'panels' | 'workspaces'>('panels');
  /* Which shelf the sidebar is showing; 'all' stacks every shelf. */
  let panelShelf = $state<'all' | ShelfId>('all');
  let workspaceShelf = $state<'all' | 'builtin' | 'custom'>('all');
  let editingId = $state<string | null>(null);
  let searchText = $state('');
  const query = $derived(searchText.trim().toLowerCase());
  /* `version` tracks workspace state (what is open where); `snapshot` and
     `edited` decide when a preview's content is stale. Keeping them apart
     means a layout change never re-clones every panel on the shelves. */
  let version = $state(0);
  let snapshot = $state(0);
  let edited = $state<Record<string, number>>({});
  function stampOf(id: string): string {
    return `${snapshot}.${edited[id] ?? 0}`;
  }
  let lastFocus: HTMLElement | null = null;
  let searchEl = $state<HTMLInputElement | null>(null);
  let rootEl = $state<HTMLElement | null>(null);
  let deletedIds = $state(new Set<string>());

  /* What the Store knows about this computer's extensions: which shelf a panel
     stands on, who made it, and the art a sandboxed panel shows (seeded like
     the Store's, from the listing's repo, so the two surfaces agree). */
  let storeItems = $state<LibraryItemDto[]>([]);
  const storeById = $derived(new Map(storeItems.map((item) => [item.localId, item])));
  async function loadStoreItems(): Promise<void> {
    try {
      storeItems = (await storeBridge()?.library()) ?? [];
    } catch {
      storeItems = [];
    }
  }

  function artSeed(item: LibraryItemDto): string {
    return item.origin?.repoId ?? `local:${item.localId}`;
  }

  export function open(target?: 'panels' | 'workspaces'): void {
    if (target) view = target;
    void loadStoreItems();
    deletedIds = deletedPanelIds(PM);
    version += 1;
    snapshot += 1;
    lastFocus = document.activeElement as HTMLElement | null;
    window.clearTimeout(closeTimer);
    closing = null;
    shown = true;
    const app = document.getElementById('app');
    if (app) app.inert = true;
    window.requestAnimationFrame(() => searchEl?.focus());
  }

  /* The overlay leaves the way it arrived. State flips at once — callers
     mutate the layout right after close() — and only the surface lingers,
     inert, for the reveal duration. A drop onto a dock skips it: the overlay
     is already out of sight. */
  const REVEAL_MS = 160;
  let closing = $state<null | 'shelves' | 'editor'>(null);
  let closeTimer = 0;

  export function close(): void {
    if (!shown) return;
    const animate = !dragActive && wantsMotion();
    const from = editingId ? 'editor' : 'shelves';
    /* Flush so the live-panel mirror returns its element to the pool before
       callers mutate the workspace layout. */
    flushSync(() => {
      leaveEditor(false);
      shown = false;
      closing = animate ? from : null;
    });
    if (animate) closeTimer = window.setTimeout(() => { closing = null; }, REVEAL_MS);
    flying?.cancel();
    endDrag();
    const app = document.getElementById('app');
    if (app) app.inert = false;
    const focus = lastFocus;
    lastFocus = null;
    if (focus?.isConnected) focus.focus();
  }

  export function isOpen(): boolean {
    return shown;
  }

  /** `content` also re-snapshots the previews (a panel definition changed). */
  export function refresh(content = false): void {
    version += 1;
    if (content) snapshot += 1;
  }

  function extensionRecord(id: string | undefined): { scope?: string; trust?: string; manifest?: { name?: string } | null } | undefined {
    if (!id) return undefined;
    return PM.Kernel?.loader?.records?.()?.find?.((record: any) => record.id === id);
  }

  function isGenerated(id: string): boolean {
    return ((PM.WS?.all ?? []) as any[]).some((workspace) => workspace.custom?.some?.((section: any) => section.id === id));
  }

  const panels = $derived.by(() => {
    void version;
    return Object.entries(PM.PANELS || {})
      .map(([id, def]: [string, any]) => {
        const ownerId = PM.Kernel?.panels?.topEntry?.(id)?.ownerId as string | undefined;
        const record = extensionRecord(ownerId);
        const item = ownerId ? storeById.get(ownerId) : undefined;
        const source = panelSource({ ownerId, record, item, generated: isGenerated(id) });
        const extensionName = item?.name ?? record?.manifest?.name;
        return { ...def, id, title: def.title || id, source, extensionName: source.shelf === 'builtin' ? undefined : extensionName };
      })
      .filter((panel) => panelBelongsInLibrary(panel) && !deletedIds.has(panel.id))
      .sort((a, b) => String(a.title).localeCompare(String(b.title)));
  });
  const icons = $derived(panelIcons(panels, PM.ICONS || {}));
  const matchedPanels = $derived(panels.filter((panel) =>
    !query || `${panel.title} ${panel.id} ${panel.extensionName ?? ''} ${panel.source.maker ?? ''}`.toLowerCase().includes(query)));

  /* Installed extensions that contribute panels but have none registered right
     now (turned off, waiting on setup or trust) still have a place on the
     shelf, so an install never silently disappears from the Library. */
  const idleExtensions = $derived.by(() => {
    const active = new Set(panels.map((panel) => panel.source.extensionId).filter(Boolean));
    return storeItems
      .filter((item) => item.group !== 'builtin' && item.contributes.includes('panels') && !active.has(item.localId))
      .filter((item) => !query || `${item.name} ${item.localId}`.toLowerCase().includes(query))
      .sort((a, b) => a.name.localeCompare(b.name));
  });

  const workspaces = $derived.by(() => {
    void version;
    return [...((PM.WS?.all ?? []) as any[])];
  });
  const currentWorkspaceId = $derived.by(() => {
    void version;
    return PM.WS?.current?.id as string | undefined;
  });
  const matchedWorkspaces = $derived(workspaces.filter((workspace) =>
    !query || String(workspace.name || workspace.id).toLowerCase().includes(query)));

  /* ── shelves ── */

  const SHELF_GAP = 18;
  const LEDGE_INSET = 0;
  const DEFAULT_COVER = { width: 360, height: 240 };
  const WORKSPACE_COVER = { width: 480, height: 300 };
  let shelfWidth = $state(0);
  function measureShelves(node: HTMLElement) {
    const observer = new ResizeObserver(([entry]) => {
      /* The shelves stay mounted behind the editor. Hiding them must not
         repack every book into a zero-width row. */
      if (entry && entry.contentRect.width > 0) shelfWidth = entry.contentRect.width;
    });
    observer.observe(node);
    return { destroy() { observer.disconnect(); } };
  }
  const scale = $derived(shelfScale(shelfWidth));
  const rowWidth = $derived(Math.max(1, shelfWidth - LEDGE_INSET * 2));
  const storeAvailable = $derived(!!PM.StoreUI?.open);

  type Book =
    | { kind: 'panel'; key: string; cover: CoverSize; panel: (typeof panels)[number] }
    | { kind: 'idle'; key: string; cover: CoverSize; item: LibraryItemDto }
    | { kind: 'store'; key: string; cover: CoverSize }
    | { kind: 'workspace'; key: string; cover: CoverSize; workspace: any };
  type Shelf = { id: string; label: string; count: number; rows: Book[][] };

  function shelve(id: string, label: string, books: Book[], count = books.length): Shelf {
    return { id, label, count, rows: packRows(books, (book) => book.cover.width, rowWidth, SHELF_GAP) };
  }

  const panelCounts = $derived.by(() => {
    const counts: Record<'all' | ShelfId, number> = { all: 0, builtin: 0, yours: 0, store: 0 };
    for (const panel of panels) { counts[panel.source.shelf as ShelfId] += 1; counts.all += 1; }
    return counts;
  });

  const panelShelves = $derived.by((): Shelf[] => {
    const wanted = panelShelf === 'all' ? SHELF_ORDER : [panelShelf];
    return wanted.flatMap((id) => {
      const books: Book[] = matchedPanels
        .filter((panel) => panel.source.shelf === id)
        .map((panel) => ({ kind: 'panel', key: panel.id, panel, cover: coverSize(panelPreviewSize(panel), scale, rowWidth) }));
      const idle: Book[] = idleExtensions
        .filter((item) => item.group === id)
        .map((item) => ({ kind: 'idle', key: `idle:${item.localId}`, item, cover: coverSize(DEFAULT_COVER, scale, rowWidth) }));
      const count = books.length + idle.length;
      /* The Store shelf ends with a way to find more — and is only that when
         nothing from the Store is installed yet. */
      const invite: Book[] = id === 'store' && storeAvailable && !query
        ? [{ kind: 'store', key: 'store', cover: coverSize(DEFAULT_COVER, scale, rowWidth) }]
        : [];
      if (!count && !invite.length) return [];
      return [shelve(id, SHELF_LABEL[id], [...books, ...idle, ...invite], count)];
    });
  });

  const workspaceCounts = $derived({
    all: workspaces.length,
    builtin: workspaces.filter((workspace) => workspace.builtin).length,
    custom: workspaces.filter((workspace) => !workspace.builtin).length
  });

  const workspaceShelves = $derived.by((): Shelf[] => {
    const groups: Array<{ id: 'builtin' | 'custom'; label: string }> = [
      { id: 'builtin', label: 'Built in' },
      { id: 'custom', label: 'Saved by you' }
    ];
    return groups
      .filter((group) => workspaceShelf === 'all' || workspaceShelf === group.id)
      .map((group) => shelve(group.id, group.label, matchedWorkspaces
        .filter((workspace) => !!workspace.builtin === (group.id === 'builtin'))
        .map((workspace) => ({ kind: 'workspace', key: workspace.id, workspace, cover: coverSize(WORKSPACE_COVER, scale, rowWidth) }))))
      .filter((shelf) => shelf.count > 0);
  });

  const shelves = $derived(view === 'panels' ? panelShelves : workspaceShelves);
  const pageTitle = $derived(view === 'panels'
    ? (panelShelf === 'all' ? 'All panels' : SHELF_LABEL[panelShelf])
    : ({ all: 'All workspaces', builtin: 'Built-in workspaces', custom: 'Saved workspaces' } as const)[workspaceShelf]);

  /* Each shelf opens at its top, like walking to a new aisle. */
  let contentEl = $state<HTMLElement | null>(null);
  $effect(() => {
    void view; void panelShelf; void workspaceShelf;
    contentEl?.scrollTo({ top: 0 });
  });

  function showPanels(next: 'all' | ShelfId): void {
    view = 'panels';
    panelShelf = next;
  }

  function showWorkspaces(next: 'all' | 'builtin' | 'custom'): void {
    view = 'workspaces';
    workspaceShelf = next;
  }

  function openStore(page: 'kind:panels' | 'library' = 'kind:panels'): void {
    PM.LibraryUI?.close?.();
    PM.StoreUI?.open?.(page);
  }

  function inWorkspace(id: string): boolean {
    void version;
    if (id === 'toolbar') return true;
    const found = PM.Layout?.findPanel?.(PM.WS?.current, id);
    return !!(found && !found.dock.hidden && !found.spec.collapsed);
  }

  const editingPanel = $derived(editingId ? panels.find((panel) => panel.id === editingId) ?? null : null);
  const stageSize = $derived(editingPanel ? panelPreviewSize(editingPanel) : { width: 360, height: 320 });
  const editingSource = $derived(editingPanel
    ? [SHELF_LABEL[editingPanel.source.shelf as ShelfId], editingPanel.extensionName !== editingPanel.title ? editingPanel.extensionName : null, editingPanel.source.maker]
      .filter(Boolean).join(' · ')
    : '');
  /* Starting points for changing one panel; each fills the draft for review. */
  const panelIdeas = $derived(editingPanel ? [
    ['Make it more compact', `Make the ${editingPanel.title} panel more compact without losing any of its controls`],
    ['Clarify the layout', `Improve the ${editingPanel.title} panel's spacing and hierarchy so its most-used controls stand out`],
    ['Add a control', `Add a control to the ${editingPanel.title} panel that `]
  ] : []);
  const editingLive = $derived.by(() => {
    void version;
    if (!editingId) return false;
    const element = PM.panelInst?.[editingId]?.el as HTMLElement | undefined;
    /* Popped out into its own window it cannot be shown; borrowed by this stage it already is. */
    return !!element && (!element.classList.contains('popped') || !!element.closest('#library-screen'));
  });

  /* The picked panel keeps its identity across the swap: it travels and grows
     from its card into the editor stage, and shrinks back onto the card on the
     way out. Measure where the box was, put it back there with a transform,
     then let it animate to where it now is (FLIP). Nothing in the library CSS
     may transform an ancestor of either box during this, or the measured
     geometry drifts under the animation. */
  /* Travel timing, shared with the CSS (--library-travel, --ease) so every
     move in the Library — card to editor, back, shelf to shelf — keeps one
     tempo and one curve. */
  const FLIP_MS = 240;
  const FLIP_EASE = 'cubic-bezier(.22,1,.36,1)';

  function wantsMotion(): boolean {
    return !window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches;
  }

  function cardBox(id: string): HTMLElement | null {
    return rootEl?.querySelector<HTMLElement>(
      `[data-panel-id="${CSS.escape(id)}"] .library-live`
    ) ?? null;
  }

  function stageBox(): HTMLElement | null {
    return rootEl?.querySelector<HTMLElement>('.library-stage-frame') ?? null;
  }

  /* Travels `node` from the `from` box to where it already sits. Transform
     only: any opacity on the moving panel — or on an ancestor fading it in —
     reads as a flash rather than as one object changing place. */
  type Flight = { cancel(): void };
  function flip(node: HTMLElement, from: DOMRect, done?: (finished: boolean) => void,
    to = node.getBoundingClientRect()): Flight | null {
    if (!from.width || !from.height || !to.width || !to.height) { done?.(true); return null; }
    const origin = node.style.transformOrigin;
    node.style.transformOrigin = '0 0';
    const shift = `translate(${from.left - to.left}px,${from.top - to.top}px)`;
    const scale = `scale(${from.width / to.width},${from.height / to.height})`;
    const animation = node.animate(
      [{ transform: `${shift} ${scale}` }, { transform: 'none' }],
      { duration: FLIP_MS, easing: FLIP_EASE }
    );
    let settled = false;
    const settle = (finished: boolean) => {
      if (settled) return;
      settled = true;
      node.style.transformOrigin = origin;
      done?.(finished);
    };
    animation.onfinish = () => settle(true);
    animation.oncancel = () => settle(false);
    /* Animation cancel events arrive later. Clean up now so an old flight
       cannot clear or complete a newer one after a quick Back or reopen. */
    return { cancel() { animation.cancel(); settle(false); } };
  }

  /* cloneNode leaves canvases blank and drops live form values; copy both so
     every panel — including future generated ones — previews as-is. */
  function copyLiveState(source: Element, target: Element): void {
    const canvases = source.querySelectorAll('canvas');
    target.querySelectorAll('canvas').forEach((node, index) => {
      const from = canvases[index];
      if (!from?.width || !from?.height) return;
      node.width = from.width;
      node.height = from.height;
      try { node.getContext('2d')?.drawImage(from, 0, 0); } catch { /* tainted or GL-only canvas */ }
    });
    const fields = source.querySelectorAll<HTMLInputElement>('input,textarea,select');
    target.querySelectorAll<HTMLInputElement>('input,textarea,select').forEach((node, index) => {
      const from = fields[index];
      if (from) node.value = from.value;
    });
  }

  function regexEscape(value: string): string {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  /* A clone cannot keep the live panel's IDs: duplicate IDs make app-level
     queries ambiguous. But removing them breaks panels such as Timeline whose
     own stylesheet deliberately targets #tl-head, #tl-canvas-wrap, and
     #tl-canvas. Give every cloned ID a preview-only name and rewrite the
     clone's embedded styles and ID references to match. */
  function isolateCloneIds(clone: HTMLElement, panelId: string): void {
    const withIds = [clone, ...clone.querySelectorAll<HTMLElement>('[id]')].filter((element) => element.id);
    const replacements = new Map<string, string>();
    withIds.forEach((element, index) => {
      const sourceId = element.id;
      const isolatedId = `library-preview-${panelId.replace(/[^a-zA-Z0-9_-]/g, '-')}-${index}`;
      replacements.set(sourceId, isolatedId);
      element.dataset.librarySourceId = sourceId;
      element.id = isolatedId;
    });

    for (const style of clone.querySelectorAll('style')) {
      let css = style.textContent || '';
      for (const [sourceId, isolatedId] of replacements) {
        css = css.replace(new RegExp(`#${regexEscape(sourceId)}(?![a-zA-Z0-9_-])`, 'g'), `#${isolatedId}`);
      }
      style.textContent = css;
    }

    const referenceAttributes = ['for', 'aria-activedescendant', 'aria-controls', 'aria-describedby', 'aria-labelledby'];
    for (const element of [clone, ...clone.querySelectorAll<HTMLElement>('*')]) {
      for (const attribute of referenceAttributes) {
        const value = element.getAttribute(attribute);
        if (!value) continue;
        element.setAttribute(attribute, value.split(/\s+/).map((id) => replacements.get(id) || id).join(' '));
      }
      const href = element.getAttribute('href');
      if (href?.startsWith('#') && replacements.has(href.slice(1))) {
        element.setAttribute('href', `#${replacements.get(href.slice(1))}`);
      }
    }
  }

  /* The cached snapshot flies above the scrolling shelves on the way back.
     Moving it avoids cloning the panel and copying its canvas bitmaps again. */
  let flying: Flight | null = null;

  function flyBack(id: string, from: DOMRect): void {
    const card = cardBox(id);
    if (!card || !rootEl) return;
    card.closest('.library-card')?.scrollIntoView({ block: 'nearest' });
    const to = card.getBoundingClientRect();
    const base = rootEl.getBoundingClientRect();
    if (!to.width || !to.height) return;
    const preview = previews.get(id);
    const frame = preview?.frame;
    const ghost = frame ? document.createElement('div') : card.cloneNode(true) as HTMLElement;
    ghost.classList.add('library-live', 'library-fly');
    ghost.classList.toggle('is-clipped', !!preview?.clipped);
    ghost.style.left = `${to.left - base.left}px`;
    ghost.style.top = `${to.top - base.top}px`;
    ghost.style.width = `${to.width}px`;
    ghost.style.height = `${to.height}px`;
    if (frame && preview) {
      frame.style.transform = `scale(${to.width / preview.width})`;
      ghost.appendChild(frame);
    }
    rootEl.appendChild(ghost);
    card.style.visibility = 'hidden';
    flying = flip(ghost, from, () => {
      flying = null;
      if (frame?.parentNode === ghost && card.isConnected) {
        card.appendChild(frame);
        card.classList.add('has-preview');
      }
      ghost.remove();
      card.style.visibility = '';
    }, to);
  }

  /* What travels is the cover's snapshot, not the live panel: a live panel
     re-lays itself out when it is moved, and one that measures itself with
     getBoundingClientRect (the Timeline's canvas) would size itself to the
     mid-flight transform. The real panel — and the conversation, which can
     be long — join once the snapshot has landed. */
  let landed = $state(false);

  function openEditor(id: string): void {
    /* A ghost still landing from a previous back would outlive its grid. */
    flying?.cancel();
    const from = wantsMotion() ? cardBox(id)?.getBoundingClientRect() ?? null : null;
    landed = !from;
    editingId = id;
    if (!from) { focusScope(id); return; }
    /* Render the editor now so the stage frame can be measured at its real
       size before the first painted frame of the animation. */
    flushSync();
    rootEl?.focus({ preventScroll: true });
    const stage = stageBox();
    const joined = () => { if (editingId === id) { landed = true; focusScope(id); } };
    if (stage) {
      const to = stage.getBoundingClientRect();
      const preview = previews.get(id);
      if (preview?.frame) preview.frame.style.transform = `scale(${to.width / preview.width})`;
      flying = flip(stage, from, (finished) => {
        flying = null;
        if (finished) joined();
      }, to);
    } else joined();
  }

  function focusScope(id: string): void {
    PM.AgentUI?.setScope?.(panelScope([id]));
    PM.AgentUI?.setDraft?.('', true);
  }

  /* `animate` is off when the whole library is closing: close() returns the
     borrowed panel inside its own flushSync, and there is no card left on
     screen to shrink back onto. */
  function leaveEditor(animate = true): void {
    if (!editingId) return;
    const id = editingId;
    const from = animate && wantsMotion() ? stageBox()?.getBoundingClientRect() ?? null : null;
    flying?.cancel();
    returnBorrowedPanel?.();
    editingId = null;
    landed = false;
    /* The agent may have changed it: re-snapshot this one cover, after it lands. */
    edited = { ...edited, [id]: (edited[id] ?? 0) + 1 };
    PM.AgentUI?.setScope?.('workspace');
    if (!from) return;
    flushSync();
    flyBack(id, from);
  }

  /* Borrows the live panel element from the layout pool, exactly like the
     panel pop-out does, so the editor previews the real panel — state,
     handlers and agent edits included — then hands it back on teardown. */
  let returnBorrowedPanel: (() => void) | null = null;
  function mirrorPanel(node: HTMLElement, id: string) {
    const element = PM.panelInst?.[id]?.el as HTMLElement | undefined;
    const originalParent = element?.parentElement;
    let borrowed: HTMLElement | null = null;
    if (element && !element.classList.contains('popped')) {
      borrowed = element;
      borrowed.classList.add('popped');
      placePanel(node, borrowed);
    }
    const restore = () => {
      if (returnBorrowedPanel === restore) returnBorrowedPanel = null;
      if (!borrowed) return;
      borrowed.classList.remove('popped');
      const host = document.querySelector<HTMLElement>(
        `#body [data-panel-slot="${CSS.escape(id)}"]`
      ) ?? document.querySelector<HTMLElement>(
        `#pm-panel-pool [data-panel-host="${CSS.escape(id)}"]`
      ) ?? (originalParent?.isConnected ? originalParent : null);
      if (host) placePanel(host, borrowed);
      borrowed = null;
    };
    /* Return before Svelte detaches the stage. A disconnected iframe cannot
       be moved in place and would reload even through placePanel. */
    returnBorrowedPanel = restore;
    return { destroy: restore };
  }

  /* Stands the cover's snapshot in the stage for the flight, scaled to fill
     it, and hands it back to the cache when the live panel takes over. */
  function stageGhost(node: HTMLElement, id: string) {
    const preview = previews.get(id);
    const frame = preview?.frame;
    if (!frame || !preview) return {};
    frame.parentNode?.removeChild(frame);
    node.appendChild(frame);
    return {
      destroy() {
        if (frame.parentNode === node) frame.remove();
      }
    };
  }

  function addToWorkspace(id: string): void {
    PM.LibraryUI?.reveal?.(id);
  }

  function confirmDeletePanel(event: MouseEvent, panel: any): void {
    event.stopPropagation();
    void PM.confirm?.({
      message: `Delete “${panel.title}” panel?`,
      detail: 'This removes the panel from the Library and every workspace. This cannot be undone.',
      confirmLabel: 'Delete Panel',
      destructive: true
    })?.then((ok: boolean) => {
      if (!ok || !deletePanel(PM, panel.id)) return;
      deletedIds = deletedPanelIds(PM);
      version += 1;
    });
  }

  /* Each book shows a snapshot clone of the live panel, laid out at the
     canonical size owned by its definition and grown to contain its content,
     then drawn at the shelf's shared scale — so panels keep their sizes
     relative to one another, the way books of different formats do. Past
     the preview bounds a cover is cropped, never squeezed. The active dock is
     only a source of panel content; its width, height, collapsed state, and
     location never shape the Library. A clone, not a borrow, keeps the
     workspace intact while a preview is dragged back onto a dock. */
  type PreviewParams = { id: string; stamp: string; scale: number; maxWidth: number; art: [string, string] | null };
  type Preview = { stamp: string; frame: HTMLElement | null; width: number; height: number; clipped: boolean };

  /* Snapshots outlive their books: switching shelves, searching, or coming
     back from the editor re-attaches the same clone instead of cloning and
     re-measuring the panel again. A stale one stays up until its
     replacement is ready, so a cover never blinks back to its placeholder. */
  const previews = new Map<string, Preview>();

  /* Observe the scroll area once. Covers just below it are prepared ahead of
     scrolling; distant shelves do not clone or measure any panel DOM. */
  const previewVisibility = new Map<Element, (nearby: boolean) => void>();
  let previewObserver: IntersectionObserver | null = null;
  function observePreview(node: HTMLElement, changed: (nearby: boolean) => void): () => void {
    if (!previewObserver) {
      previewObserver = new IntersectionObserver((entries) => {
        for (const entry of entries) previewVisibility.get(entry.target)?.(entry.isIntersecting);
      }, { root: node.closest('.library-content'), rootMargin: '200px 0px' });
    }
    previewVisibility.set(node, changed);
    previewObserver.observe(node);
    return () => {
      previewObserver?.unobserve(node);
      previewVisibility.delete(node);
      if (!previewVisibility.size) { previewObserver?.disconnect(); previewObserver = null; }
    };
  }

  /* Cloning and measuring nearby panels runs in short batches, leaving time
     between builds for scrolling, typing and navigation. A cover travelling
     back from the editor lands before any replacement is measured. */
  const buildQueue: Array<() => void> = [];
  let pumping = false;
  /* A hidden or busy window can hold animation frames back; a timer keeps
     the queue moving regardless. Whichever fires first runs the step. */
  function nextFrame(step: () => void): void {
    let ran = false;
    const run = () => {
      if (ran) return;
      ran = true;
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timer);
      step();
    };
    const frame = window.requestAnimationFrame(run);
    const timer = window.setTimeout(run, 64);
  }
  function scheduleBuild(task: () => void): void {
    buildQueue.push(task);
    if (rootEl) rootEl.dataset.previews = 'pending';
    if (!pumping) { pumping = true; nextFrame(pumpBuilds); }
  }
  function pumpBuilds(): void {
    if (!flying) {
      const start = performance.now();
      while (buildQueue.length && performance.now() - start < 4) buildQueue.shift()!();
    }
    if (buildQueue.length) { nextFrame(pumpBuilds); return; }
    pumping = false;
    if (rootEl) rootEl.dataset.previews = 'ready';
  }

  function buildPreview(host: HTMLElement, id: string, stamp: string): Preview {
    const blank: Preview = { stamp, frame: null, width: 0, height: 0, clipped: false };
    /* A sandboxed panel's body is another document: a DOM clone would load
       a second, unconnected copy of it. Its cover is the art instead. */
    if (panelFrameOf(PM.PANELS?.[id])) return blank;
    const element = PM.panelInst?.[id]?.el as HTMLElement | undefined;
    if (!element) return blank;
    const size = panelPreviewSize(PM.PANELS?.[id] ?? {});
    let fullHeight = size.height;
    const clone = element.cloneNode(true) as HTMLElement;
    clone.classList.add('library-clone');
    isolateCloneIds(clone, id);
    clone.style.width = `${size.width}px`;
    clone.style.height = `${size.height}px`;
    clone.style.flex = 'none';
    copyLiveState(element, clone);
    const frame = document.createElement('div');
    frame.className = 'library-live-frame';
    frame.style.width = `${size.width}px`;
    frame.style.height = `${size.height}px`;
    frame.appendChild(clone);
    /* Measure in place: computed styles and scroll sizes need the document. */
    host.appendChild(frame);
    const library = PM.PANELS?.[id]?.library;
    if (library && typeof library.render === 'function') {
      library.render({ source: element, clone, width: size.width, height: size.height });
    } else {
      // Snapshots cannot scroll. Unroll their scroll regions from the inside
      // out, then grow the book so the last controls and footer stay visible.
      // Canvas previews own their geometry through library.render instead.
      // A full-height inline textarea otherwise adds a baseline gap below
      // itself on every measurement, even when its content already fits.
      for (const textarea of clone.querySelectorAll('textarea')) textarea.style.display = 'block';
      /* Read every overflow style before writing anything, so the scan costs
         one style pass rather than one per element. */
      const regions = [...clone.querySelectorAll<HTMLElement>('*')].filter((region) => {
        const style = getComputedStyle(region);
        return /^(auto|scroll|hidden)$/.test(style.overflowY)
          && style.position !== 'absolute' && style.position !== 'fixed';
      }).reverse();
      for (const region of regions) {
        if (!region.clientHeight || region.querySelector('canvas')) continue;
        const height = region.scrollHeight;
        if (height <= region.clientHeight + 1) continue;
        const border = region.offsetHeight - region.clientHeight;
        region.style.boxSizing = 'border-box';
        region.style.height = `${height + border}px`;
        region.style.minHeight = `${height + border}px`;
        region.style.maxHeight = 'none';
        region.style.flexShrink = '0';
      }
      fullHeight = Math.max(size.height, clone.scrollHeight + clone.offsetHeight - clone.clientHeight);
      clone.style.height = `${fullHeight}px`;
    }
    const width = Math.min(size.width, MAX_PREVIEW_WIDTH);
    const height = Math.min(fullHeight, MAX_PREVIEW_HEIGHT);
    frame.style.width = `${width}px`;
    frame.style.height = `${height}px`;
    return { stamp, frame, width, height, clipped: width < size.width || height < fullHeight };
  }

  function panelPreview(node: HTMLElement, params: PreviewParams) {
    let current = params;
    let shown: Preview | null = null;
    let alive = true;
    let nearby = false;
    let queuedStamp: string | null = null;
    function fit(): void {
      const size = shown?.frame ? { width: shown.width, height: shown.height } : panelPreviewSize(PM.PANELS?.[current.id] ?? {});
      const cover = coverSize(size, current.scale, current.maxWidth);
      node.style.width = `${cover.width}px`;
      node.style.height = `${cover.height}px`;
      if (shown?.frame) shown.frame.style.transform = `scale(${cover.scale})`;
    }
    function paintArt(): void {
      node.classList.toggle('is-sandboxed', !!panelFrameOf(PM.PANELS?.[current.id]));
      node.classList.toggle('has-art', !!current.art);
      if (current.art) {
        node.style.setProperty('--art-a', current.art[0]);
        node.style.setProperty('--art-b', current.art[1]);
      }
    }
    /* A first snapshot fades in over its placeholder; a refreshed one swaps in place. */
    function show(preview: Preview | null, fresh = false): void {
      if (shown?.frame && shown.frame !== preview?.frame && shown.frame.parentNode === node) shown.frame.remove();
      shown = preview;
      if (fresh && preview?.frame && wantsMotion()) {
        const frame = preview.frame;
        frame.classList.add('is-fresh');
        frame.addEventListener('animationend', () => frame.classList.remove('is-fresh'), { once: true });
      }
      if (nearby && preview?.frame && preview.frame.parentNode !== node
        && !preview.frame.parentElement?.classList.contains('library-fly')) node.appendChild(preview.frame);
      node.classList.toggle('has-preview', preview?.frame?.parentNode === node);
      node.classList.toggle('is-clipped', !!preview?.clipped);
      fit();
    }
    function attach(): void {
      const cached = previews.get(current.id);
      show(cached ?? null);
      if (!nearby || cached?.stamp === current.stamp || queuedStamp === current.stamp) return;
      const id = current.id;
      const stamp = current.stamp;
      queuedStamp = stamp;
      scheduleBuild(() => {
        if (queuedStamp === stamp) queuedStamp = null;
        if (!alive || !nearby || !isOpen() || editingId || current.id !== id || current.stamp !== stamp) return;
        const previous = shown?.frame;
        const next = buildPreview(node, id, stamp);
        if (previous && previous !== next.frame) previous.remove();
        previews.set(id, next);
        show(next, !previous);
      });
    }
    paintArt();
    attach();
    const unobserve = observePreview(node, (visible) => {
      nearby = visible;
      if (nearby) attach();
    });
    return {
      update(next: PreviewParams) {
        const stale = next.id !== current.id || next.stamp !== current.stamp;
        current = next;
        paintArt();
        if (stale) attach();
        else fit();
      },
      destroy() {
        alive = false;
        unobserve();
        /* The snapshot stays cached for the next book that shows this panel. */
        if (shown?.frame?.parentNode === node) shown.frame.remove();
        node.classList.remove('has-preview', 'is-clipped');
      }
    };
  }

  function sandboxArt(id: string): [string, string] | null {
    const sandboxed = panelFrameOf(PM.PANELS?.[id]);
    const item = sandboxed ? storeById.get(sandboxed.extensionId) : undefined;
    return item ? artFor(artSeed(item)) : null;
  }

  /* Dragging a card temporarily lifts the library out of the way (and lifts
     the #app inert guard) so the card can be dropped straight onto a dock. */
  const PANEL_MIME = 'application/x-powermove-panel';
  let dragging = $state(false);
  let dragActive = false;
  let hintDock: HTMLElement | null = null;

  function setHint(dock: HTMLElement | null): void {
    if (hintDock === dock) return;
    hintDock?.classList.remove('library-drop-hint');
    hintDock = dock;
    hintDock?.classList.add('library-drop-hint');
  }

  function dockDragOver(event: DragEvent): void {
    if (!event.dataTransfer?.types.includes(PANEL_MIME)) return;
    const dock = (event.target as HTMLElement | null)?.closest?.('[data-dock]') as HTMLElement | null;
    setHint(dock);
    if (!dock) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
  }

  function dockDrop(event: DragEvent): void {
    const id = event.dataTransfer?.getData(PANEL_MIME);
    const dock = (event.target as HTMLElement | null)?.closest?.('[data-dock]') as HTMLElement | null;
    if (!id || !dock) return;
    event.preventDefault();
    const dockId = dock.dataset.dock!;
    PM.WS?.mutate?.((workspace: any) => {
      if (workspace.hiddenPanels?.some((panel: any) => panel.id === id)) PM.Layout.restorePanel(workspace, id);
      if (!PM.Layout.hasPanel(workspace, id)) PM.Layout.addPanel(workspace, id, dockId);
      else PM.Layout.movePanel?.(workspace, id, dockId);
      const found = PM.Layout.findPanel(workspace, id);
      if (found) { found.dock.hidden = false; found.spec.collapsed = false; }
    });
    endDrag();
    PM.LibraryUI?.close?.();
  }

  function endDrag(): void {
    if (!dragActive) return;
    dragActive = false;
    dragging = false;
    setHint(null);
    window.removeEventListener('dragover', dockDragOver, true);
    window.removeEventListener('drop', dockDrop, true);
    const app = document.getElementById('app');
    if (app && shown) app.inert = true;
  }

  function dragStart(event: DragEvent, id: string): void {
    if (!event.dataTransfer) return;
    event.dataTransfer.setData(PANEL_MIME, id);
    event.dataTransfer.effectAllowed = 'copy';
    dragActive = true;
    window.addEventListener('dragover', dockDragOver, true);
    window.addEventListener('drop', dockDrop, true);
    const app = document.getElementById('app');
    if (app) app.inert = false;
    /* Hiding the overlay inside dragstart would cancel the native drag, so
       reveal the editor underneath one tick later. */
    setTimeout(() => { if (dragActive) dragging = true; }, 0);
  }

  function activateWorkspace(id: string): void {
    PM.WS?.activate?.(id);
    version += 1;
  }

  function deleteWorkspace(event: MouseEvent, workspace: any): void {
    event.stopPropagation();
    void PM.confirm?.({
      message: `Delete “${workspace.name}” workspace?`,
      detail: 'This removes the saved workspace layout. Your project and its layers will not be deleted.',
      confirmLabel: 'Delete Workspace'
    })?.then((ok: boolean) => {
      if (!ok) return;
      PM.WS?.remove?.(workspace.id);
      version += 1;
    });
  }

  function workspaceDocks(workspace: any): Array<{ flex: boolean; panels: string[] }> {
    const docks = (workspace.layout?.docks ?? []).filter((dock: any) => !dock.hidden);
    return docks.map((dock: any) => ({ flex: !!dock.flex, panels: (dock.panels ?? []).map((panel: any) => panel.id) }));
  }

  function workspaceSummary(workspace: any): string {
    const count = (workspace.layout?.docks ?? []).reduce((total: number, dock: any) => total + (dock.panels?.length ?? 0), 0);
    return `${count} panel${count === 1 ? '' : 's'}`;
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      if (editingId) leaveEditor();
      else PM.LibraryUI?.close?.();
      return;
    }
    if (event.key !== 'Tab' || !rootEl) return;
    const items = [...rootEl.querySelectorAll<HTMLElement>('button,input,textarea,select')]
      .filter((item) => !item.hasAttribute('disabled') && item.offsetParent !== null);
    const first = items[0];
    const last = items.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  id="library-overlay"
  class:on={shown || !!closing}
  class:closing={!!closing}
  class:closing-editor={closing === 'editor'}
  class:dragging
  onpointerdown={(event) => { if (event.target === event.currentTarget) PM.LibraryUI?.close?.(); }}
  onkeydown={keydown}
>
  <div class="titlebar-overlay-drag" aria-hidden="true"></div>
  <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
  <section
    id="library-screen"
    class:on={shown || !!closing}
    role="dialog"
    aria-modal="true"
    aria-label="Panel library"
    data-previews="ready"
    tabindex="-1"
    bind:this={rootEl}
  >
    <header class="library-head">
      <!-- The Store's title gesture: in the editor "Library" rolls into the
           shelf you came from, a chevron grows in, and the title is Back. -->
      <BackTitle
        {PM}
        class="library-title"
        animateText={false}
        text={editingPanel ? pageTitle : 'Library'}
        back={!!editingPanel}
        label="Back to panels"
        onback={() => leaveEditor()}
      />
      {#if editingPanel}
        {#if inWorkspace(editingPanel.id)}
          <span class="library-head-note" in:fade={pageSettle()}>In workspace</span>
        {:else}
          <button class="btn pri library-head-action" type="button" in:fade={pageSettle()} onclick={() => addToWorkspace(editingPanel.id)}>
            Add to workspace
          </button>
        {/if}
      {/if}
      <button class="iconbtn" type="button" aria-label="Close Library" onclick={() => PM.LibraryUI?.close?.()}>
        <Icon {PM} name="x" />
      </button>
    </header>

    {#if editingPanel}
      <div class="library-editor">
        <!-- The book, opened on the desk: the live panel at its own
             proportions, so the swap from its cover scales evenly. -->
        <section class="library-stage" aria-label="Panel preview">
          <div class="library-stage-book">
          {#key editingId}
            <div
              class="library-stage-frame pop-mirror"
              style:--sw={stageSize.width}
              style:--sh={stageSize.height}
            >
              {#if landed}
                <div class="library-stage-live" use:mirrorPanel={editingPanel.id}>
                  {#if !editingLive}
                    <div class="library-stage-empty">
                      <Icon {PM} name={icons[editingPanel.id] || 'panel'} />
                      <b>No live preview yet</b>
                      <span>Add {editingPanel.title} to your workspace to see and edit the real panel here.</span>
                    </div>
                  {/if}
                </div>
              {:else}
                <div class="library-stage-ghost" use:stageGhost={editingPanel.id}></div>
              {/if}
            </div>
          {/key}
            <!-- Its shelf label comes with it, on the same ledge. -->
            <div class="library-stage-label" in:fade={pageSettle()}>
              <b>{editingPanel.title}</b>
              <span>{editingSource}</span>
            </div>
          </div>
        </section>
        <aside class="library-chat" class:ready={landed} aria-label="Panel conversation">
          {#if landed}
            <div class="library-chat-body" in:fly={pageFly(1)}>
              <ThreadPicker {PM} />
              <div class="library-chat-log">
                {#key agentState.threadId}<Conversation {PM} welcome={panelWelcome} />{/key}
              </div>
              <div class="library-chat-foot">
                <Composer {PM} panelId="library" placeholder={`Describe a change to ${editingPanel.title}…`} />
              </div>
            </div>
          {/if}
        </aside>
      </div>
    {/if}
      <div class="library-shell" style:display={editingPanel ? 'none' : undefined} in:fade={pageSettle()}>
        <nav class="library-sidebar" aria-label="Library sections">
          <label class="library-search">
            <Icon {PM} name="search" />
            <input
              type="search"
              placeholder="Search"
              aria-label={view === 'panels' ? 'Search panels' : 'Search workspaces'}
              bind:this={searchEl}
              bind:value={searchText}
            />
          </label>
          <div class="library-nav">
            <span class="library-nav-label">Panels</span>
            {@render navItem('panel', 'All panels', 'All panels', panelCounts.all, view === 'panels' && panelShelf === 'all', () => showPanels('all'))}
            {#each SHELF_ORDER as id (id)}
              {#if id === 'builtin' || panelCounts[id] > 0 || (id === 'store' && storeAvailable)}
                {@render navItem(
                  id === 'builtin' ? 'stack' : id === 'yours' ? 'wand' : 'basket',
                  SHELF_LABEL[id],
                  id === 'builtin' ? 'Built-in panels' : id === 'yours' ? 'Panels made by you' : 'Panels from the Store',
                  panelCounts[id],
                  view === 'panels' && panelShelf === id,
                  () => showPanels(id)
                )}
              {/if}
            {/each}
            <span class="library-nav-label">Workspaces</span>
            {@render navItem('grid', 'All workspaces', 'All workspaces', workspaceCounts.all, view === 'workspaces' && workspaceShelf === 'all', () => showWorkspaces('all'))}
            {@render navItem('stack', 'Built in', 'Built-in workspaces', workspaceCounts.builtin, view === 'workspaces' && workspaceShelf === 'builtin', () => showWorkspaces('builtin'))}
            {#if workspaceCounts.custom > 0}
              {@render navItem('userCircle', 'Saved by you', 'Saved workspaces', workspaceCounts.custom, view === 'workspaces' && workspaceShelf === 'custom', () => showWorkspaces('custom'))}
            {/if}
          </div>
          {#if storeAvailable}
            <button class="library-navbtn library-store-link" type="button" onclick={() => openStore()}>
              <Icon {PM} name="basket" />
              <span>Browse the Store</span>
            </button>
          {/if}
        </nav>
        <main class="library-main">
          <div class="library-top">
            <div class="library-page-title">
              <b><Scritto value={pageTitle} /></b>
            </div>
            <div class="library-top-action">
              {#if view === 'panels'}
                <button class="btn" type="button" onclick={() => PM.LibraryUI?.create?.()}>
                  <Icon {PM} name="plus" />
                  New panel
                </button>
              {:else}
                <button class="btn" type="button" onclick={() => PM.WS?.saveAsNew?.()}>
                  <Icon {PM} name="plus" />
                  Save current as new
                </button>
              {/if}
            </div>
          </div>
          <div class="library-content" bind:this={contentEl}>
            {#key `${view}:${view === 'panels' ? panelShelf : workspaceShelf}`}
            <!-- Another shelf arrives sideways, like another Store tab. -->
            <div
              in:fly={pageFly(0)}
              class="library-grid library-shelves {view}"
              class:is-empty={shelves.length === 0}
              style:--shelf-gap={`${SHELF_GAP}px`}
              style:--ledge-inset={`${LEDGE_INSET}px`}
              use:measureShelves
            >
              {#each shelves as shelf (shelf.id)}
                <section class="library-shelf" aria-label={shelf.label}>
                  {#if shelves.length > 1 || (view === 'panels' ? panelShelf === 'all' : workspaceShelf === 'all')}
                    <h3 class="library-shelf-title">{shelf.label}{#if shelf.count}<span class="library-count"><Scritto value={shelf.count} /></span>{/if}</h3>
                  {/if}
                  <div class="library-shelf-books" role="list" aria-label={view === 'panels' ? `${shelf.label} panels` : `${shelf.label} workspaces`}>
                    {#each shelf.rows as row, index (index)}
                      <div class="library-shelf-row" role="none">
                        {#each row as book (book.key)}
                          {#if book.kind === 'panel'}
                            {@render panelBook(book.panel)}
                          {:else if book.kind === 'workspace'}
                            {@render workspaceBook(book.workspace, book.cover)}
                          {:else if book.kind === 'idle'}
                            {@render idleBook(book.item, book.cover)}
                          {:else}
                            {@render storeBook(book.cover)}
                          {/if}
                        {/each}
                      </div>
                    {/each}
                  </div>
                </section>
              {:else}
                <div class="library-empty" role="status">
                  <Icon {PM} name={query ? 'search' : view === 'panels' ? 'panel' : 'grid'} />
                  <b>{query ? "There's nothing here." : 'Nothing on this shelf yet.'}</b>
                </div>
              {/each}
            </div>
            {/key}
          </div>
        </main>
      </div>
  </section>
</div>

{#snippet navItem(icon: string, label: string, aria: string, count: number, on: boolean, run: () => void)}
  <button class="library-navbtn" class:on type="button" aria-label={aria} aria-current={on ? 'page' : undefined} onclick={() => { run(); searchText = ''; }}>
    <Icon {PM} name={icon} />
    <span>{label}</span>
    <span class="library-count"><Scritto value={count} /></span>
  </button>
{/snippet}

{#snippet panelBook(panel: (typeof panels)[number])}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="library-card library-card-live library-book"
    role="listitem"
    data-panel-id={panel.id}
    data-shelf={panel.source.shelf}
    draggable="true"
    ondragstart={(event) => dragStart(event, panel.id)}
    ondragend={endDrag}
  >
    <div class="library-book-cover">
      <button class="library-card-hit" type="button" aria-label={`Edit ${panel.title}`} title={panel.title} onclick={() => openEditor(panel.id)}>
        <div class="library-live" use:panelPreview={{ id: panel.id, stamp: stampOf(panel.id), scale, maxWidth: rowWidth, art: sandboxArt(panel.id) }}>
          <span class="library-thumb-icon">
            <Icon {PM} name={icons[panel.id] || 'panel'} />
            <b>{panel.title}</b>
          </span>
        </div>
      </button>
      {#if panel.id !== 'viewer' && panel.id !== 'toolbar'}
        <button
          class="iconbtn library-card-delete"
          type="button"
          aria-label={`Delete ${panel.title} panel`}
          title={`Delete ${panel.title} panel`}
          onclick={(event) => confirmDeletePanel(event, panel)}
        >
          <Icon {PM} name="trash" />
        </button>
      {/if}
      <button
        class="iconbtn library-card-add"
        type="button"
        aria-label={inWorkspace(panel.id) ? `Show ${panel.title} in workspace` : `Add ${panel.title} to workspace`}
        title={inWorkspace(panel.id) ? `Show ${panel.title} in workspace` : `Add ${panel.title} to workspace`}
        onclick={() => addToWorkspace(panel.id)}
      >
        <Icon {PM} name={inWorkspace(panel.id) ? 'eye' : 'plus'} />
      </button>
    </div>
    <div class="library-book-label">
      <b>{panel.title}</b>
      {#if panel.source.shelf !== 'builtin' && (panel.source.maker || (panel.extensionName && panel.extensionName !== panel.title))}
        <span>{[panel.extensionName !== panel.title ? panel.extensionName : null, panel.source.maker].filter(Boolean).join(' · ')}</span>
      {/if}
    </div>
  </div>
{/snippet}

{#snippet idleBook(item: LibraryItemDto, cover: CoverSize)}
  {@const [a, b] = artFor(artSeed(item))}
  <div class="library-book library-book-idle" role="listitem" data-extension-id={item.localId}>
    <div class="library-book-cover">
      <button
        class="library-card-hit"
        type="button"
        aria-label={`Open ${item.name} in your Store library`}
        title={item.name}
        onclick={() => openStore('library')}
      >
        <div class="library-live is-sandboxed has-art" style:width={`${cover.width}px`} style:height={`${cover.height}px`} style:--art-a={a} style:--art-b={b}>
          <span class="library-thumb-icon">
            <Icon {PM} name="panel" />
            <b>{item.name}</b>
          </span>
        </div>
      </button>
    </div>
    <div class="library-book-label">
      <b>{item.name}</b>
      <span>{statusText(item) ?? (item.enabled ? 'Not loaded' : 'Turned off')}</span>
    </div>
  </div>
{/snippet}

{#snippet storeBook(cover: CoverSize)}
  <div class="library-book library-book-store" role="listitem">
    <div class="library-book-cover">
      <button class="library-card-hit" type="button" onclick={() => openStore()}>
        <div class="library-live" style:width={`${cover.width}px`} style:height={`${cover.height}px`}>
          <span class="library-thumb-icon">
            <Icon {PM} name="basket" />
            <b>Find panels in the Store</b>
          </span>
        </div>
      </button>
    </div>
    <div class="library-book-label"></div>
  </div>
{/snippet}

{#snippet workspaceBook(workspace: any, cover: CoverSize)}
  <div class="library-card library-book workspace-card" class:active={workspace.id === currentWorkspaceId} role="listitem" data-workspace-id={workspace.id}>
    <div class="library-book-cover">
      <button class="library-card-hit" type="button" aria-label={`Activate ${workspace.name}`} title={workspace.name} onclick={() => activateWorkspace(workspace.id)}>
        <div class="workspace-map" style:width={`${cover.width}px`} style:height={`${cover.height}px`} style:--accent={workspace.theme?.accent || undefined}>
          {#each workspaceDocks(workspace) as dock}
            <div class="workspace-map-dock" class:flex={dock.flex}>
              {#each dock.panels as panelId}
                <span class="workspace-map-panel">
                  <Icon {PM} name={icons[panelId] || 'panel'} />
                  <b>{PM.PANELS?.[panelId]?.title || panelId}</b>
                </span>
              {:else}
                <span class="workspace-map-panel empty"></span>
              {/each}
            </div>
          {/each}
        </div>
      </button>
      {#if !workspace.builtin}
        <button
          class="iconbtn library-card-add"
          type="button"
          aria-label={`Delete ${workspace.name}`}
          title={`Delete ${workspace.name} workspace`}
          onclick={(event) => deleteWorkspace(event, workspace)}
        >
          <Icon {PM} name="trash" />
        </button>
      {/if}
    </div>
    <div class="library-book-label">
      <b>{workspace.name}</b>
      <span><Scritto value={workspace.id === currentWorkspaceId ? 'Active' : workspaceSummary(workspace)} /></span>
    </div>
  </div>
{/snippet}

{#snippet panelWelcome()}
  {#if editingPanel}
    <div class="agent-welcome library-welcome">
      <div class="agent-suggestions">
        {#each panelIdeas as [label, prompt] (label)}
          <button type="button" onclick={() => PM.AgentUI?.setDraft(prompt, true)}><span>{label}</span><svg viewBox="0 0 16 16" aria-hidden="true"><path d="m6 4 4 4-4 4" /></svg></button>
        {/each}
      </div>
    </div>
  {/if}
{/snippet}
