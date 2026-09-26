<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import type { CompareDto, CompareStatus, ListingDto, TreeFileDto } from '@powermove/registry/wire';
  import Icon from '../panels/Icon.svelte';
  import { mountSquircles, SQUIRCLE_SELECTOR } from '../settings/squircle';
  import { openPopoverMenu, type PopoverMenuItem } from '../controls/popover-menu';
  import { openSignIn, subscribeAccount, type CloudUser } from '../cloud/account';
  import type { LibraryItemDto, StoreErrorBody, StoreResult } from '../../../shared/store-ipc';
  import {
    KINDS, KIND_LABEL, KIND_PLURAL,
    actionErrorText, artFor, coordinate, detailAction, detailFromDto, groupLibrary, includesText, isKind,
    libraryAction, libraryItemFor, listingFromDto, loadError, makerText, needsAttention, needsSetup, needsTrust, ownsListing,
    parseLineage, permissionLines, publishErrorText, requiresText, secondaryPublish, statusIsHot, statusText, storeBridge,
    type Action, type Lineage, type LoadError, type StoreDetail, type StoreKind, type StoreListing, type StorePage, type StorePM,
    type VersionEntry
  } from './data';
  import { openPublishSheet } from './publish-sheet';
  import { openSandboxCheckSheet } from './sandbox-check-sheet';
  import { bridge } from '../kernel/bridge';

  /* Two places and seven kinds. Browse is the storefront; a kind is the store
     narrowed to one shelf; Library is everything on this Mac, in one list. */
  let { PM }: { PM: StorePM } = $props();

  /* The Store is a section of the home. Its page follows Claude's Customize
     screen: a large title, Discover | Library beside search and Publish, a carousel of collections, then shelves of cards. Views
     inside slide deeper and back; sibling pages settle in place. */
  /* Motion, one vocabulary for the whole Store:
       page      Discover, Library, a kind or search settles in with a short
                 rise; a detail slides in the way you moved and back out.
       settle    content that changes in place (data after its skeleton, the
                 carousel's next collection, a status line) fades.
     Hover and press are CSS (store.css › motion). Reduced motion stills it all. */
  const SLIDE = 24;
  const RISE = 8;
  const PAGE_MS = 200;
  const SEARCH_DEBOUNCE_MS = 200;
  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  const OFFLINE: LoadError = { offline: true, message: 'Can’t reach the store' };

  type Loadable<T> = { status: 'loading' } | { status: 'ready'; value: T } | { status: 'error'; error: LoadError };
  type Section = { id: string; title: string; items: ListingDto[] };
  type Shelf = { items: ListingDto[]; nextCursor: string | null; more: boolean };
  /* A detail page: the store's page for a coordinate, a Library item's own
     page, or both when the item came from the store. */
  type DetailTarget = { coord: { handle: string; slug: string } | null; localId: string | null; preview: StoreListing | null };
  /* `lineage`: the fork against what it was forked from, where manifest.json
     always reads as changed (the uploaded copy names its origin). */
  type Compare = { base: string; head: string; lineage: boolean; state: Loadable<CompareDto> };
  type SourceFile = { path: string; state: Loadable<string> };

  let shown = $state(false);
  let page = $state<StorePage>('browse');
  let searchText = $state('');
  let featuredIndex = $state(0);
  let direction = $state(0);
  let rootEl = $state<HTMLElement | null>(null);
  let scrollEl = $state<HTMLElement | null>(null);
  let account = $state<CloudUser | null>(null);

  let library = $state<LibraryItemDto[]>([]);
  let browse = $state<Loadable<Section[]>>({ status: 'loading' });
  let shelf = $state<Loadable<Shelf>>({ status: 'loading' });
  /* Stale-while-revalidate for the surfaces you move between: a shelf or a
     detail page you have seen shows at once from cache while it refreshes.
     Skeletons only appear the first time something is loaded. */
  const shelfCache = new Map<StoreKind, Shelf>();
  const detailCache = new Map<string, StoreDetail>();
  let search = $state<Loadable<ListingDto[]>>({ status: 'loading' });
  let detail = $state<DetailTarget | null>(null);
  let remote = $state<Loadable<StoreDetail> | null>(null);
  let compare = $state<Compare | null>(null);
  let files = $state<Loadable<TreeFileDto[]> | null>(null);
  let openFile = $state<SourceFile | null>(null);
  let actionError = $state<string | null>(null);
  let busy = $state<Record<string, string>>({});
  let showAllVersions = $state(false);
  /* The version whose Withdraw is asking "are you sure?" inline. */
  let withdrawing = $state<string | null>(null);

  $effect(() => subscribeAccount((user) => {
    account = user;
    // Signing in or out changes what reads as yours.
    if (untrack(() => shown)) void loadLibrary();
  }));

  const query = $derived(searchText.trim());
  const pageKind = $derived(page.startsWith('kind:') ? kindOf(page.slice(5)) : null);
  const viewKey = $derived(detail ? `detail:${detail.coord ? `${detail.coord.handle}/${detail.coord.slug}` : detail.localId}` : query ? 'search' : page);
  const slide = $derived(reduced() ? { duration: 0 }
    : direction === 0 ? { y: RISE, duration: PAGE_MS, easing: cubicOut }
    : { x: direction * SLIDE, duration: PAGE_MS, easing: cubicOut });
  /* A status line that changes under you (an update finished) fades in. */
  const settle = $derived(reduced() ? { duration: 0 } : { duration: 180, easing: cubicOut });

  const groups = $derived(groupLibrary(library));
  /* The Library: what needs you first, then what you installed and what you
     made. Powermove's own extensions are not listed. */
  const attention = $derived([...groups.store, ...groups.yours].filter(needsAttention));
  const installedCalm = $derived(groups.store.filter((item) => !needsAttention(item)));
  const yoursCalm = $derived(groups.yours.filter((item) => !needsAttention(item)));
  const sections = $derived(browse.status === 'ready' ? browse.value.map((section) => ({ ...section, listings: section.items.map(toListing) })) : []);
  /* The carousel: the curated shelf, then each kind that has something on it.
     Explore goes to the whole collection. */
  const slides = $derived(sections
    .filter((section) => section.id === 'featured' || isKind(section.id))
    .slice(0, 4)
    .map((section) => {
      const kind = kindOf(section.id);
      return {
        id: section.id,
        eyebrow: kind ? 'Collection' : 'Curated by Powermove',
        title: kind ? KIND_PLURAL[kind] : 'Featured this week',
        blurb: kind ? KIND_BLURB[kind] : 'Extensions the Powermove team keeps coming back to.',
        listings: section.listings,
        explore: () => (kind ? show(`kind:${kind}`) : revealShelf(section.id))
      };
    }));
  const slideAt = $derived(slides[Math.min(featuredIndex, slides.length - 1)] ?? null);
  /* Icons in a collection sit at staggered sizes and heights, like a shelf of
     app icons rather than a grid. */
  const ICON_SIZES = [60, 46, 68, 42, 58, 48, 64];
  const ICON_LIFTS = [-10, 14, -2, 22, -12, 10, -4];
  const KIND_BLURB: Record<StoreKind, string> = {
    effects: 'Blur, grain, light and colour you can key to layer motion.',
    transitions: 'Cuts, wipes and morphs between layers.',
    panels: 'New places to work, docked beside the timeline.',
    themes: 'Powermove in someone else’s colours.',
    commands: 'One-step actions for the command palette.',
    layers: 'New kinds of layer: type, shapes, 3D and more.',
    tools: 'Canvas tools for drawing, measuring and arranging.'
  };

  const detailData = $derived(remote?.status === 'ready' ? remote.value : null);
  const detailItem = $derived.by(() => {
    if (!detail) return undefined;
    const repoId = detailData?.repoId ?? detail.preview?.repoId;
    const byRepo = repoId ? libraryItemFor(repoId, library, detailData ?? detail.preview ?? undefined) : undefined;
    return byRepo ?? (detail.localId ? library.find((item) => item.localId === detail?.localId) : undefined);
  });

  function kindOf(value: string): StoreKind | null {
    return isKind(value) ? value : null;
  }

  function toListing(dto: ListingDto): StoreListing {
    return listingFromDto(dto, library);
  }

  /* ── bridge calls ── */

  async function call<T>(run: () => Promise<StoreResult<T>> | undefined): Promise<StoreResult<T>> {
    try {
      const result = await run();
      return result ?? { ok: false, error: { error: 'internal', detail: 'unavailable' } };
    } catch {
      // No store service in this window (it failed to boot): offline.
      return { ok: false, error: { error: 'internal', detail: 'unavailable' } };
    }
  }

  function failed(error: StoreErrorBody): LoadError {
    return error.detail === 'unavailable' ? OFFLINE : loadError(error);
  }

  async function loadLibrary(): Promise<void> {
    try {
      library = (await storeBridge()?.library()) ?? [];
    } catch {
      // Keep what is shown; the next change reloads it.
    }
  }

  async function loadBrowse(): Promise<void> {
    const hadBrowse = browse.status === 'ready';
    if (!hadBrowse) browse = { status: 'loading' };
    const result = await call(() => storeBridge()?.browse());
    if (result.ok) {
      const sections = result.value.sections.filter((section) => section.items.length > 0);
      browse = { status: 'ready', value: sections };
      // A kind's browse section is that shelf's first page: seed it so the
      // first visit to a kind is instant rather than a skeleton.
      for (const section of sections) {
        const kind = kindOf(section.id);
        if (kind && !shelfCache.has(kind)) shelfCache.set(kind, { items: section.items, nextCursor: null, more: false });
      }
      if (!hadBrowse) featuredIndex = 0;
    } else if (!hadBrowse) {
      browse = { status: 'error', error: failed(result.error) };
    }
  }

  let shelfToken = 0;
  async function loadShelf(kind: StoreKind, cursor?: string): Promise<void> {
    const token = ++shelfToken;
    const previous = cursor && shelf.status === 'ready' ? shelf.value.items : [];
    const cached = cursor ? null : shelfCache.get(kind) ?? null;
    if (cursor && shelf.status === 'ready') shelf = { status: 'ready', value: { ...shelf.value, more: true } };
    else shelf = cached ? { status: 'ready', value: cached } : { status: 'loading' };
    const result = await call(() => storeBridge()?.extensions({ category: kind, ...(cursor ? { cursor } : {}) }));
    if (token !== shelfToken) return;
    if (result.ok) {
      const value: Shelf = { items: [...previous, ...result.value.items], nextCursor: result.value.nextCursor, more: false };
      shelfCache.set(kind, value);
      shelf = { status: 'ready', value };
    } else if (!cached) {
      shelf = { status: 'error', error: failed(result.error) };
    } else if (cursor) {
      shelf = { status: 'ready', value: { ...cached, more: false } };
    }
  }

  let searchToken = 0;
  async function runSearch(q: string): Promise<void> {
    const token = ++searchToken;
    const result = await call(() => storeBridge()?.extensions({ q }));
    if (token !== searchToken) return;
    search = result.ok ? { status: 'ready', value: result.value.items } : { status: 'error', error: failed(result.error) };
  }

  /* Search waits for a pause in typing. */
  $effect(() => {
    const q = query;
    if (!shown || !q) return;
    untrack(() => { search = { status: 'loading' }; });
    const timer = window.setTimeout(() => void runSearch(q), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  });

  $effect(() => {
    const kind = pageKind;
    if (shown && kind) untrack(() => void loadShelf(kind));
  });

  /* The Library follows the folder: installs, removals and update checks. */
  $effect(() => {
    if (!shown) return;
    const offUpdates = storeBridge()?.onUpdatesChanged(() => void loadLibrary());
    const offExtensions = bridge()?.extensions?.onChanged(() => void loadLibrary());
    const offLibrary = storeBridge()?.onLibraryChanged(() => void loadLibrary());
    return () => {
      offUpdates?.();
      offExtensions?.();
      offLibrary?.();
    };
  });

  let detailToken = 0;
  async function loadDetail(target: DetailTarget): Promise<void> {
    const token = ++detailToken;
    remote = null;
    files = null;
    if (!target.coord) return;
    /* A plain copy: `target` may be the reactive `detail` itself, and a
       state proxy can't cross the context bridge. */
    const coord = { handle: target.coord.handle, slug: target.coord.slug };
    const key = `${coord.handle}/${coord.slug}`;
    const cached = detailCache.get(key);
    remote = cached ? { status: 'ready', value: cached } : { status: 'loading' };
    const result = await call(() => storeBridge()?.detail(coord));
    if (token !== detailToken) return;
    if (!result.ok) {
      if (!cached) remote = { status: 'error', error: failed(result.error) };
      return;
    }
    const value = detailFromDto(result.value, library);
    detailCache.set(key, value);
    remote = { status: 'ready', value };
    if (value.latestReleaseId) void loadFiles(value.latestReleaseId, token);
  }

  /* Release trees are immutable, so a file list never needs revalidating. */
  const filesCache = new Map<string, TreeFileDto[]>();
  async function loadFiles(releaseId: string, token: number): Promise<void> {
    const cached = filesCache.get(releaseId);
    if (cached) { files = { status: 'ready', value: cached }; return; }
    files = { status: 'loading' };
    const result = await call(() => storeBridge()?.tree({ releaseId }));
    if (token !== detailToken) return;
    if (result.ok) filesCache.set(releaseId, result.value.files);
    files = result.ok ? { status: 'ready', value: result.value.files } : { status: 'error', error: failed(result.error) };
  }

  async function toggleFile(path: string): Promise<void> {
    if (openFile?.path === path) {
      openFile = null;
      return;
    }
    const data = detailData;
    if (!data) return;
    openFile = { path, state: { status: 'loading' } };
    const result = await call(() => storeBridge()?.file({ handle: data.publisher, slug: data.id, version: data.version, path }));
    if (openFile?.path !== path) return;
    openFile = { path, state: result.ok ? { status: 'ready', value: result.value } : { status: 'error', error: failed(result.error) } };
  }

  async function showCompare(base: string, head: string, lineage = false): Promise<void> {
    if (compare && compare.base === base && compare.head === head) {
      compare = null;
      return;
    }
    compare = { base, head, lineage, state: { status: 'loading' } };
    const result = await call(() => storeBridge()?.compare({ base, head }));
    if (compare?.base !== base || compare.head !== head) return;
    compare = { base, head, lineage, state: result.ok ? { status: 'ready', value: result.value } : { status: 'error', error: failed(result.error) } };
  }

  /* ── screen ── */

  export function open(target: StorePage = 'browse'): void {
    page = target;
    detail = null;
    direction = 0;
    searchText = '';
    if (!shown) {
      shown = true;
      void loadBrowse();
    } else if (browse.status === 'error') {
      void loadBrowse();
    }
    void loadLibrary();
    PM.bus?.emit?.('store:screen');
    void tick().then(() => {
      scrollEl?.scrollTo({ top: 0, behavior: 'instant' });
      rootEl?.focus({ preventScroll: true });
    });
  }

  export function close(): void {
    PM.ProjectsScreen?.show('recents');
  }

  export function isOpen(): boolean {
    return shown;
  }

  export function setActive(on: boolean): void {
    if (on === shown) return;
    shown = on;
    if (on) {
      void loadBrowse();
      void loadLibrary();
    }
    PM.bus?.emit?.('store:screen');
  }

  export function setSearch(text: string): void {
    if (text && !searchText) detail = null;
    searchText = text;
  }

  function stepSlide(delta: number): void {
    if (slides.length) featuredIndex = (featuredIndex + delta + slides.length) % slides.length;
  }

  function revealShelf(id: string): void {
    scrollEl?.querySelector(`#st-shelf-${CSS.escape(id)}`)?.scrollIntoView({ behavior: reduced() ? 'instant' : 'smooth', block: 'start' });
  }

  /* Publish: pick one of yours that has something to publish. Publishing
     needs an account; the sheet for that opens instead when signed out. */
  function publishMenu(event: MouseEvent): void {
    if (!(event.currentTarget instanceof HTMLElement)) return;
    if (!account) {
      openSignIn();
      return;
    }
    const ready = groups.yours.filter((item) => item.publish);
    if (!ready.length) {
      show('library');
      toast('Nothing to publish yet. Extensions you or your agent make show up under Yours.');
      return;
    }
    openPopoverMenu({
      anchor: event.currentTarget,
      label: 'Publish',
      items: ready.map((item) => ({ label: item.publish === 'update' ? `${item.name} (update)` : item.name, run: () => void publish(item) }))
    });
  }

  function scrollTop(): void {
    void tick().then(() => scrollEl?.scrollTo({ top: 0, behavior: 'instant' }));
  }

  function show(id: StorePage): void {
    direction = 0;
    page = id;
    detail = null;
    searchText = '';
    scrollTop();
  }

  function pushDetail(target: DetailTarget): void {
    direction = 1;
    detail = target;
    compare = null;
    openFile = null;
    actionError = null;
    showAllVersions = false;
    withdrawing = null;
    void loadDetail(target);
    scrollTop();
  }

  function openListing(listing: StoreListing): void {
    pushDetail({ coord: { handle: listing.publisher, slug: listing.id }, localId: null, preview: listing });
  }

  /* Your published folder opens on its own store page; an install on the
     page it came from. */
  function openItem(item: LibraryItemDto): void {
    const published = item.published?.coordinate ? splitCoordinate(item.published.coordinate) : null;
    const coord = published ?? (item.origin ? splitCoordinate(item.origin.coordinate) : null);
    pushDetail({ coord, localId: item.localId, preview: null });
  }

  /* "Forked from" leads to the origin's own page, one level deeper. */
  function openLineage(lineage: Lineage): void {
    pushDetail({ coord: { handle: lineage.handle, slug: lineage.slug }, localId: null, preview: null });
  }

  function splitCoordinate(value: string): { handle: string; slug: string } | null {
    const [handle, slug] = value.split('/');
    return handle && slug ? { handle, slug } : null;
  }

  function back(): void {
    direction = -1;
    detail = null;
    remote = null;
    compare = null;
  }

  /* Escape leaves a detail; otherwise it is the home's. App shortcuts such as
     ⌘, keep working over the Store. */
  function keydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || !detail) return;
    event.stopPropagation();
    event.preventDefault();
    back();
  }

  /* Lisse squircles on the cards, previews and controls while the screen is up. */
  $effect(() => {
    if (!shown || !rootEl) return;
    const root = rootEl;
    const selector = `${SQUIRCLE_SELECTOR}, .st-card, .st-thumb, .st-slide, .st-item, .st-item-icon, .st-detail-banner, .st-search, .st-plus`;
    let unmount = mountSquircles(root, selector);
    const observer = new MutationObserver(() => { unmount(); unmount = mountSquircles(root, selector); });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => { observer.disconnect(); unmount(); };
  });

  function art(pair: [string, string]): string {
    const [a, b] = pair;
    return `--art-a:${a};--art-b:${b}`;
  }

  function itemArt(item: LibraryItemDto): string {
    return art(artFor(item.origin?.repoId ?? `local:${item.localId}`));
  }

  /* Classify the whole package, not an individual contribution. */
  function byline(kind: StoreKind, who: string): string {
    return `${KIND_PLURAL[kind]} extension · ${who}`;
  }

  /* A published fork knows its origin from provenance (the folder's own
     manifest never names it); anything else from its manifest. */
  function storeLineageOf(item: LibraryItemDto | undefined): Lineage | undefined {
    const fork = item?.fork ? splitCoordinate(item.fork.coordinate) : null;
    if (item?.fork && fork) return { handle: fork.handle, slug: fork.slug, version: item.fork.version, releaseId: item.fork.releaseId };
    const parsed = item?.forkedFrom ? parseLineage(item.forkedFrom) : null;
    return parsed && 'store' in parsed ? parsed.store : undefined;
  }

  function builtinLineageOf(item: LibraryItemDto | undefined): { id: string; version: string } | undefined {
    const parsed = item?.forkedFrom ? parseLineage(item.forkedFrom) : null;
    return parsed && 'builtin' in parsed ? parsed.builtin : undefined;
  }

  function listingAction(listing: StoreListing): Action {
    return detailAction({ vars: listing.vars, item: libraryItemFor(listing.repoId, library, listing), repoId: listing.repoId });
  }

  /* ── actions ── */

  function toast(message: string, error = false): void {
    PM.toast?.(message, error ? 5000 : 3200, error ? { error: true } : {});
  }

  function setBusy(key: string, label: string | null): void {
    const next = { ...busy };
    if (label) next[key] = label;
    else delete next[key];
    busy = next;
  }

  async function recordFor(localId: string) {
    try {
      return (await bridge()?.extensions?.list())?.find((record) => record.id === localId) ?? null;
    } catch {
      return null;
    }
  }

  async function setUp(localId: string): Promise<void> {
    const record = await recordFor(localId);
    if (record) PM.Vars?.openSetup(record);
  }

  async function install(target: { repoId: string; releaseId: string | null; name: string }): Promise<void> {
    const releaseId = target.releaseId;
    if (!releaseId || busy[target.repoId]) return;
    actionError = null;
    setBusy(target.repoId, 'Installing…');
    const result = await call(() => storeBridge()?.install({ repoId: target.repoId, releaseId }));
    setBusy(target.repoId, null);
    await loadLibrary();
    if (!result.ok) {
      const message = actionErrorText(result.error);
      if (detail) actionError = message;
      else toast(message, true);
      return;
    }
    /* Full access: the files are here, then main asks. Declining leaves it
       installed and off, with "Needs full access" in the Library. */
    if (result.value.needsTrust) {
      const trusted = await askTrust(result.value.localId);
      toast(trusted ? result.value.warning ?? `${target.name} is installed.` : `${target.name} is installed. It stays off until you give it full access.`);
      if (trusted && result.value.needsSetup) void setUp(result.value.localId);
      return;
    }
    toast(result.value.warning ?? `${target.name} is installed.`);
    if (result.value.needsSetup) void setUp(result.value.localId);
  }

  /* The native "Give … full access to Powermove?" dialog is main's; this
     only asks main to show it. Resolves whether the user said Trust. */
  async function askTrust(localId: string): Promise<boolean> {
    setBusy(localId, 'Waiting…');
    const result = await call(() => storeBridge()?.trust({ localId }));
    setBusy(localId, null);
    await loadLibrary();
    if (!result.ok) {
      toast(actionErrorText(result.error), true);
      return false;
    }
    return result.value.trusted;
  }

  async function trust(item: LibraryItemDto): Promise<void> {
    if (busy[item.localId]) return;
    if (await askTrust(item.localId)) toast(`${item.name} has full access.`);
  }

  async function revokeTrust(item: LibraryItemDto): Promise<void> {
    if (busy[item.localId]) return;
    const result = await call(() => storeBridge()?.untrust({ localId: item.localId }));
    await loadLibrary();
    if (!result.ok) {
      toast(actionErrorText(result.error), true);
      return;
    }
    toast(`${item.name} no longer has full access.`);
  }

  async function update(item: LibraryItemDto): Promise<void> {
    if (!item.update || busy[item.localId]) return;
    if (item.update.modified) {
      const go = await bridge()?.confirm?.({
        message: `Update ${item.name} to ${item.update.version}?`,
        detail: 'You changed its files, so your folder stays as it is. The new version is saved beside it for your agent to merge.',
        confirmLabel: 'Save New Version'
      });
      if (!go) return;
    }
    actionError = null;
    setBusy(item.localId, 'Updating…');
    const result = await call(() => storeBridge()?.update({ localId: item.localId }));
    setBusy(item.localId, null);
    await loadLibrary();
    if (!result.ok) {
      const message = actionErrorText(result.error);
      if (detail) actionError = message;
      else toast(message, true);
      return;
    }
    toast(result.value.kind === 'updated'
      ? `${item.name} is updated to ${result.value.version}.`
      : 'Update available; you changed the files. The new version is beside your folder for your agent to merge.');
  }

  async function uninstall(item: LibraryItemDto): Promise<void> {
    if (busy[item.localId]) return;
    const fromStore = item.group === 'store';
    const go = await bridge()?.confirm?.({
      message: `Uninstall ${item.name}?`,
      detail: fromStore
        ? 'Its files are removed from this Mac. You can install it again from the store.'
        : 'Its folder is deleted from this Mac. This can’t be undone.',
      confirmLabel: 'Uninstall',
      destructive: true
    });
    if (!go) return;
    setBusy(item.localId, 'Uninstalling…');
    const result = await call(() => storeBridge()?.uninstall({ localId: item.localId }));
    setBusy(item.localId, null);
    await loadLibrary();
    if (!result.ok) {
      toast(actionErrorText(result.error), true);
      return;
    }
    if (detail?.localId === item.localId && !detail.coord) back();
    toast(`${item.name} is uninstalled.`);
  }

  async function toggle(item: LibraryItemDto): Promise<void> {
    try {
      await bridge()?.extensions?.setEnabled({ id: item.localId, enabled: !item.enabled });
    } catch {
      toast(`Unable to turn ${item.name} ${item.enabled ? 'off' : 'on'}. Try again.`, true);
    }
    await loadLibrary();
  }

  async function reveal(item: LibraryItemDto): Promise<void> {
    try {
      await bridge()?.extensions?.reveal({ id: item.localId });
    } catch {
      toast('Unable to show the folder in Finder.', true);
    }
  }

  async function checkForUpdates(): Promise<void> {
    const result = await call(() => storeBridge()?.checkUpdates());
    await loadLibrary();
    if (!result.ok) {
      toast(actionErrorText(result.error), true);
      return;
    }
    const count = library.filter((item) => item.update?.state === 'available').length;
    toast(count === 0 ? 'Everything is up to date.' : count === 1 ? 'One update is available.' : `${count} updates are available.`);
  }

  /* Publishing: main plans it (snapshot, scan, what is on the store), the
     sheet fills in the form, and main asks for the native confirmation. */
  async function publish(item: LibraryItemDto): Promise<void> {
    const bridge = storeBridge();
    if (!bridge || busy[item.localId]) return;
    actionError = null;
    setBusy(item.localId, 'Preparing…');
    const result = await call(() => bridge.publishPrepare({ localId: item.localId }));
    setBusy(item.localId, null);
    if (!result.ok) {
      // Signed out, or no handle yet: the account sheet asks for what is missing.
      if (result.error.error === 'unauthorized' || result.error.error === 'forbidden') {
        openSignIn();
        return;
      }
      const message = result.error.detail === 'unavailable' ? 'Can’t reach the store. Check your connection and try again.' : publishErrorText(result.error);
      if (detail) actionError = message;
      else toast(message, true);
      return;
    }
    openPublishSheet(PM, bridge, result.value, (published) => {
      void loadLibrary();
      // The page you published from now has a store page of its own.
      const current = detail;
      const coord = splitCoordinate(published.coordinate);
      if (current?.localId === item.localId && coord) {
        detail = { ...current, coord };
        void loadDetail(detail);
      }
    });
  }

  async function withdraw(repoId: string, version: VersionEntry): Promise<void> {
    const key = `yank:${version.id}`;
    if (busy[key]) return;
    actionError = null;
    setBusy(key, 'Withdrawing…');
    const result = await call(() => storeBridge()?.yank({ repoId, version: version.version }));
    setBusy(key, null);
    withdrawing = null;
    if (!result.ok) {
      actionError = publishErrorText(result.error);
      return;
    }
    toast(`Withdrew ${version.version}.`);
    await loadLibrary();
    if (detail) void loadDetail(detail);
  }

  function run(action: Action, item: LibraryItemDto | undefined, listing: { repoId: string; releaseId: string | null; name: string } | null): void {
    if (action.kind === 'install' && listing) void install(listing);
    else if (action.kind === 'update' && item) void update(item);
    else if (action.kind === 'setup' && item) void setUp(item.localId);
    else if (action.kind === 'trust' && item) void trust(item);
    else if (action.kind === 'toggle' && item) void toggle(item);
    else if (action.kind === 'publish' && item) void publish(item);
  }

  function rowMenu(event: MouseEvent, item: LibraryItemDto): void {
    if (!(event.currentTarget instanceof HTMLElement)) return;
    const items: PopoverMenuItem[] = [];
    // The trailing control is Publish: On and Off move here.
    if (libraryAction(item).kind === 'publish') items.push({ label: item.enabled ? 'Turn Off' : 'Turn On', run: () => void toggle(item) });
    if (item.group === 'store') items.push({ label: 'Check for Updates', run: () => void checkForUpdates() });
    if (item.trust === 'store') items.push({ label: 'Trust…', run: () => void trust(item) });
    else if (item.trust === 'store-trusted') items.push({ label: 'Revoke Trust', run: () => void revokeTrust(item) });
    // Made on this Mac: try it as the Store will run it for everyone else.
    if (item.trust === 'local') items.push({ label: 'Sandbox compatibility check…', run: () => openSandboxCheckSheet(PM, item.localId, item.name) });
    items.push({ label: 'Show in Finder', run: () => void reveal(item) });
    items.push('-', { label: 'Uninstall…', run: () => void uninstall(item) });
    openPopoverMenu({ anchor: event.currentTarget, label: `${item.name} actions`, items });
  }

  function compareCounts(value: CompareDto): string {
    const parts: string[] = [];
    if (value.counts.modified) parts.push(`${value.counts.modified} changed`);
    if (value.counts.added) parts.push(`${value.counts.added} added`);
    if (value.counts.removed) parts.push(`${value.counts.removed} removed`);
    return parts.length ? parts.join(' · ') : 'No file changes';
  }

  const COMPARE_LABEL: Record<CompareStatus, string> = { added: 'Added', removed: 'Removed', modified: 'Changed' };
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  id="store-screen"
  role="region"
  aria-label="Store"
  tabindex="-1"
  bind:this={rootEl}
  onkeydown={keydown}
>
  <main class="st-main">
    <div class="st-scroll" bind:this={scrollEl}>
      {#if !detail}
        <!-- Title, then one toolbar: where you are on the left; search on the
             right, and Publish beside it in the Library. -->
        <header class="st-head">
          <h1 class="st-title">Store</h1>
          <div class="st-toolbar">
            <div class="segmented" role="tablist" aria-label="Store pages">
              <button type="button" role="tab" class:on={page !== 'library'} aria-selected={page !== 'library'} onclick={() => show('browse')}>Discover</button>
              <button type="button" role="tab" class:on={page === 'library'} aria-selected={page === 'library'} onclick={() => show('library')}>Library</button>
            </div>
            <label class="st-search">
              <Icon {PM} name="search" />
              <input type="search" placeholder="Search extensions" aria-label="Search extensions" maxlength="120" bind:value={searchText} />
            </label>
            {#if page === 'library'}
              <button class="st-add" type="button" onclick={publishMenu}>
                <Icon {PM} name="plus" /><span>Publish</span>
              </button>
            {/if}
          </div>
        </header>
      {/if}
      {#key viewKey}
        <div class="st-column" in:fly={slide}>
          {#if detail}
            {@render detailView(detail)}

          {:else if query}
            {@render head(`Results for “${query}”`, search.status === 'ready' ? search.value.length : null)}
            {#if search.status === 'loading'}
              {@render skeleton(4)}
            {:else if search.status === 'error'}
              {@render failure(search.error, () => void runSearch(query))}
            {:else if search.value.length === 0}
              <div class="st-empty" in:fade={settle}>
                <b>No results for “{query}”</b>
                <span>Try a shorter name, a maker’s handle, or a kind such as “transition”.</span>
              </div>
            {:else}
              <div class="st-cards" in:fade={settle}>
                {#each search.value.map(toListing) as l (l.repoId)}
                  {@render card(l)}
                {/each}
              </div>
            {/if}

          {:else if page === 'browse'}
            {#if browse.status === 'loading'}
              <div class="st-slide is-skeleton" aria-hidden="true"><span class="st-skel-lines"><i></i><i></i></span></div>
              {@render skeleton(6)}
            {:else if browse.status === 'error'}
              {@render failure(browse.error, () => void loadBrowse())}
            {:else if sections.length === 0}
              <div class="st-empty" in:fade={settle}>
                <b>Nothing here yet</b>
                <span>Extensions people publish show up here. Make one with your agent and publish it from your Library.</span>
              </div>
            {:else}
              <div class="st-stack" in:fade={settle}>
                {#if slideAt}
                  {@const at = slideAt}
                  <section class="st-carousel" aria-roledescription="carousel" aria-label="Collections">
                    <div class="st-slide">
                      {#key at.id}
                        <span class="st-slide-bg" style={art(at.listings[0]?.art ?? artFor(at.id))} aria-hidden="true" in:fade={settle} out:fade={settle}></span>
                        <div class="st-slide-body" in:fade={settle} out:fade={settle}>
                          <span class="st-eyebrow">{at.eyebrow}</span>
                          <h2 class="st-slide-title">{at.title}</h2>
                          <p class="st-slide-blurb">{at.blurb}</p>
                          <div class="st-slide-icons">
                            {#each at.listings.slice(0, ICON_SIZES.length) as l, i (l.repoId)}
                              <button class="st-slide-icon" type="button" style={`--size:${ICON_SIZES[i]}px;--lift:${ICON_LIFTS[i]}px`} aria-label={`Open ${l.name}`} title={l.name} onclick={() => openListing(l)}>
                                <span class="st-thumb" style={art(l.art)}></span>
                              </button>
                            {/each}
                          </div>
                          <button class="st-explore" type="button" onclick={at.explore}>Explore</button>
                        </div>
                      {/key}
                      {#if slides.length > 1}
                        <button class="st-arrow is-prev" type="button" aria-label="Previous collection" onclick={() => stepSlide(-1)}><Icon {PM} name="chev" /></button>
                        <button class="st-arrow is-next" type="button" aria-label="Next collection" onclick={() => stepSlide(1)}><Icon {PM} name="chev" /></button>
                      {/if}
                    </div>
                    {#if slides.length > 1}
                      <div class="st-dots" role="tablist" aria-label="Collections">
                        {#each slides as s, i (s.id)}
                          <button role="tab" type="button" aria-selected={i === featuredIndex} aria-label={s.title} onclick={() => (featuredIndex = i)}></button>
                        {/each}
                      </div>
                    {/if}
                  </section>
                {/if}
                {#each sections as section (section.id)}
                  {@render shelfSection(section.id, section.id === 'featured' ? 'Featured' : section.title, section.listings, isKind(section.id) ? `kind:${section.id}` : undefined)}
                {/each}
              </div>
            {/if}

          {:else if pageKind}
            {@render head(KIND_PLURAL[pageKind], shelf.status === 'ready' && shelf.value.items.length ? (shelf.value.nextCursor ? `${shelf.value.items.length}+` : shelf.value.items.length) : null)}
            {#if shelf.status === 'loading'}
              {@render skeleton(6)}
            {:else if shelf.status === 'error'}
              {@render failure(shelf.error, () => { if (pageKind) void loadShelf(pageKind); })}
            {:else if shelf.value.items.length === 0}
              <div class="st-empty" in:fade={settle}>
                <b>Nothing here yet</b>
                <span>No one has published {KIND_PLURAL[pageKind].toLowerCase()} yet.</span>
              </div>
            {:else}
              {@const cursor = shelf.value.nextCursor}
              <div class="st-cards" in:fade={settle}>
                {#each shelf.value.items.map(toListing) as l (l.repoId)}
                  {@render card(l)}
                {/each}
              </div>
              {#if cursor}
                <button class="btn ghost st-more" type="button" disabled={shelf.value.more} onclick={() => { if (pageKind) void loadShelf(pageKind, cursor); }}>
                  {shelf.value.more ? 'Loading…' : 'Show More'}
                </button>
              {/if}
            {/if}

          {:else}
            {#if attention.length}
              <section class="st-sec">
                {@render head('Needs attention', attention.length)}
                <div class="st-cards">
                  {#each attention as item (item.localId)}
                    {@render libraryCard(item)}
                  {/each}
                </div>
              </section>
            {/if}
            <section class="st-sec">
              <div class="st-sec-head">
                <h3>Installed</h3>
                {#if groups.store.length}<span class="st-count">{groups.store.length}</span>{/if}
                {#if groups.store.length}<button class="st-show-all" type="button" onclick={() => void checkForUpdates()}>Check for Updates</button>{/if}
              </div>
              {#if installedCalm.length}
                <div class="st-cards">
                  {#each installedCalm as item (item.localId)}
                    {@render libraryCard(item)}
                  {/each}
                </div>
              {:else if !groups.store.length}
                {@render libraryEmpty('basket', 'Nothing installed yet', 'Extensions you install from the store live here, and update when their makers publish.', 'Discover Extensions', () => show('browse'))}
              {/if}
            </section>
            <section class="st-sec">
              {@render head('Made by you', groups.yours.length || null)}
              {#if yoursCalm.length}
                <div class="st-cards">
                  {#each yoursCalm as item (item.localId)}
                    {@render libraryCard(item)}
                  {/each}
                </div>
              {:else if !groups.yours.length}
                {@render libraryEmpty('wand', 'Nothing made yet', 'Ask your agent for an effect, a panel or a theme. What it makes shows up here, ready to publish.')}
              {/if}
            </section>
          {/if}
        </div>
      {/key}
    </div>
  </main>
</div>

{#snippet control(act: Action, item: LibraryItemDto | undefined, listing: { repoId: string; releaseId: string | null; name: string } | null, cls = 'btn st-install')}
  {@const pending = busy[item?.localId ?? ''] ?? busy[listing?.repoId ?? '']}
  {#if pending}
    <span class="st-installed" aria-live="polite">{pending}</span>
  {:else if act.kind === 'toggle' && item}
    <button class="st-toggle" type="button" aria-pressed={item.enabled} aria-label={`${item.name}: ${item.enabled ? 'on' : 'off'}`} onclick={() => void toggle(item)}>{act.label}</button>
  {:else if act.quiet}
    <span class="st-installed">{act.label}</span>
  {:else}
    <button class={cls} class:pri={act.primary} type="button" disabled={act.disabled} onclick={() => run(act, item, listing)}>{act.label}</button>
  {/if}
{/snippet}

<!-- A card per extension: its icon on a tile, name, what it does and who
     made it. The whole card opens it; the corner button is the one other
     target: + to install, a tick once it is here, or whatever it needs. -->
{#snippet card(l: StoreListing)}
  {@const act = listingAction(l)}
  {@const item = libraryItemFor(l.repoId, library, l)}
  {@const target = { repoId: l.repoId, releaseId: l.latestReleaseId, name: l.name }}
  {@const pending = busy[item?.localId ?? ''] ?? busy[l.repoId]}
  <div class="st-item">
    <button class="st-item-open" type="button" aria-label={`Open ${l.name}`} onclick={() => openListing(l)}></button>
    <span class="st-item-icon"><span class="st-thumb" style={art(l.art)}></span></span>
    <span class="st-item-copy">
      <b>{l.name}</b>
      <span class="st-item-line">{l.tagline}</span>
      <span class="st-item-by">by {l.publisher}</span>
    </span>
    <span class="st-item-action">
      {#if !pending && act.kind === 'install'}
        <button class="st-plus" type="button" aria-label={`${act.label}: ${l.name}`} title={act.label} onclick={() => run(act, item, target)}><Icon {PM} name="plus" /></button>
      {:else if !pending && act.quiet && item?.group === 'store'}
        <span class="st-plus is-done" role="img" aria-label="Installed" title="Installed">
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </span>
      {:else}
        {@render control(act, item, target, 'btn st-install')}
      {/if}
    </span>
  </div>
{/snippet}

{#snippet head(title: string, count: number | string | null)}
  <div class="st-sec-head">
    <h3>{title}</h3>
    {#if count !== null}<span class="st-count">{count}</span>{/if}
  </div>
{/snippet}

<!-- A Library card: the Discover card, plus what is up with it. The corner
     holds its one action, or its switch when it just runs; the menu has the
     rest. Turned off or waiting on you, the icon dims. -->
{#snippet libraryCard(item: LibraryItemDto)}
  {@const note = statusText(item)}
  {@const lineage = storeLineageOf(item)}
  {@const act = libraryAction(item)}
  <div class="st-item is-library" class:is-off={needsSetup(item) || needsTrust(item) || !item.enabled}>
    <button class="st-item-open" type="button" aria-label={`Open ${item.name}`} onclick={() => openItem(item)}></button>
    <span class="st-item-icon"><span class="st-thumb" style={itemArt(item)}></span></span>
    <span class="st-item-copy">
      <b>{item.name}</b>
      <span class="st-item-line">{item.description ?? KIND_LABEL[item.category]}</span>
      <span class="st-item-by">{makerText(item)}{#if lineage}&nbsp;· forked from {lineage.handle}/{lineage.slug}{/if}</span>
      {#if note}
        {#key note}
          <span class="st-item-status" class:is-hot={statusIsHot(item)} in:fade={settle}>{note}</span>
        {/key}
      {/if}
    </span>
    <span class="st-item-action">
      {#if act.kind === 'toggle' && !busy[item.localId]}
        <button class="toggle" class:on={item.enabled} type="button" role="switch" aria-checked={item.enabled} aria-label={`${item.name}: ${item.enabled ? 'on' : 'off'}`} onclick={() => void toggle(item)}><i></i></button>
      {:else}
        {@render control(act, item, null, 'btn st-install')}
      {/if}
      <button class="st-item-more" type="button" aria-label={`More actions for ${item.name}`} onclick={(event) => rowMenu(event, item)}>
        <Icon {PM} name="more" />
      </button>
    </span>
  </div>
{/snippet}

{#snippet libraryEmpty(icon: string, title: string, body: string, cta?: string, go?: () => void)}
  <div class="st-empty-card">
    <span class="st-empty-mark"><Icon {PM} name={icon} /></span>
    <span class="st-empty-copy">
      <b>{title}</b>
      <span>{body}</span>
    </span>
    {#if cta && go}<button class="btn st-empty-go" type="button" onclick={go}>{cta}</button>{/if}
  </div>
{/snippet}

{#snippet shelfSection(id: string, title: string, items: StoreListing[], more?: StorePage)}
  {#if items.length}
    <section class="st-sec" id={`st-shelf-${id}`}>
      <div class="st-sec-head">
        <h3>{title}</h3>
        {#if more}<button class="st-show-all" type="button" onclick={() => show(more)}>Show all <span aria-hidden="true">→</span></button>{/if}
      </div>
      <div class="st-cards">
        {#each items as l (l.repoId)}
          {@render card(l)}
        {/each}
      </div>
    </section>
  {/if}
{/snippet}

{#snippet skeleton(count: number)}
  <div class="st-cards" aria-busy="true" aria-label="Loading">
    {#each Array.from({ length: count }, (_, i) => i) as i (i)}
      <div class="st-item is-skeleton" aria-hidden="true">
        <span class="st-item-icon st-skel"></span>
        <span class="st-skel-lines"><i></i><i></i></span>
      </div>
    {/each}
  </div>
{/snippet}

{#snippet failure(error: LoadError, retry: () => void)}
  <p class="st-failure" role="status" in:fade={settle}>
    <span>{error.offline ? 'Can’t reach the store. Check your connection.' : error.message}</span>
    <button class="st-link" type="button" onclick={retry}>Retry</button>
  </p>
{/snippet}

{#snippet detailView(target: DetailTarget)}
  {@const data = detailData}
  {@const item = detailItem}
  {@const preview = target.preview}
  {@const name = data?.name ?? item?.name ?? preview?.name ?? ''}
  {@const kind = data?.kind ?? preview?.kind ?? item?.category ?? 'tools'}
  {@const version = data?.version || preview?.version || item?.version || ''}
  {@const who = data ? `by ${data.publisher}` : preview ? `by ${preview.publisher}` : item ? makerText(item) : ''}
  {@const lede = data?.tagline ?? preview?.tagline ?? item?.description ?? ''}
  {@const pair = data?.art ?? preview?.art ?? artFor(item?.origin?.repoId ?? `local:${item?.localId ?? ''}`)}
  {@const act = detailAction({ vars: data?.vars ?? item?.vars, item, repoId: data?.repoId })}
  {@const alsoPublish = data && item?.fork && data.repoId !== item.published?.repoId ? null : secondaryPublish(item)}
  {@const ownPage = ownsListing(data, account)}
  {@const storeLineage = data?.forkedFrom ?? storeLineageOf(item)}
  {@const builtinLineage = storeLineage ? undefined : builtinLineageOf(item)}
  <!-- A fork compares against the release it was forked from (P0 Q5), or
       the release it was last merged from once that moves on. -->
  {@const lineageBase = (ownPage ? item?.fork?.upstreamReleaseId : undefined) ?? data?.forkedFrom?.releaseId}
  {@const lineageCompare = lineageBase && data?.latestReleaseId && lineageBase !== data.latestReleaseId ? { base: lineageBase, head: data.latestReleaseId } : null}
  {@const updateCompare = item?.origin && item.update ? { base: item.origin.releaseId, head: item.update.releaseId } : null}
  {@const vars = data?.vars ?? item?.vars ?? []}
  {@const contributes = data?.contributes.length ? data.contributes : item?.contributes ?? []}
  {@const apiVersion = data?.apiVersion ?? preview?.apiVersion ?? null}
  {@const access = permissionLines(data?.permissions ?? preview?.permissions ?? item?.permissions)}
  {@const coord = data ? coordinate(data) : preview ? coordinate(preview) : item?.origin?.coordinate ?? (account?.handle && item ? `${account.handle}/${item.localId}` : item?.localId ?? '')}

  <button class="st-back" type="button" onclick={back}>
    <Icon {PM} name="chev" /><span>Back</span>
  </button>
  <div class="st-banner st-banner-art st-detail-banner" style={art(pair)} aria-hidden="true">
    <span class="st-banner-mark"><span class="st-thumb is-hero" style={art(pair)}></span></span>
  </div>
  <!-- Icon, copy, then the one action at the right edge, all on one line
       like the hero card and every row. Anything the action needs to
       explain goes under the head as its own line. -->
  <header class="st-detail-head">
    <span class="st-thumb is-hero" style={art(pair)}></span>
    <div class="st-detail-copy">
      <div class="st-detail-title">
        <h2>{name}</h2>
        {#if version}<span class="st-tag">{version}</span>{/if}
      </div>
      <p class="st-byline">{byline(kind, who)}</p>
      {#if lede}<p class="st-lede">{lede}</p>{/if}
      {#if access.length}
        <!-- What it declares it uses, beside the one action that installs it. -->
        <p class="st-access" aria-label="Access">
          {#each access as line, i (line.label)}
            {#if i > 0}<span class="st-access-sep" aria-hidden="true">·</span>{/if}
            <span class:is-warn={line.warn}>{#if line.warn}<Icon {PM} name="warning" />{/if}{line.label}</span>
          {/each}
        </p>
      {/if}
      {#if storeLineage}
        <p class="st-lineage">
          Forked from <button class="st-link" type="button" onclick={() => openLineage(storeLineage)}>{storeLineage.handle}/{storeLineage.slug}</button>
          {#if lineageCompare}
            <span class="st-lineage-sep">·</span>
            <button class="st-link is-quiet" type="button" aria-expanded={compare?.base === lineageCompare.base && compare.head === lineageCompare.head} onclick={() => void showCompare(lineageCompare.base, lineageCompare.head, true)}>See what changed</button>
          {/if}
        </p>
      {:else if builtinLineage}
        <p class="st-lineage">Forked from Powermove’s built-in {builtinLineage.id} {builtinLineage.version}</p>
      {/if}
    </div>
    <div class="st-detail-actions">
      {#if remote?.status === 'loading' && !item}
        <button class="btn" type="button" disabled>Install</button>
      {:else if remote?.status === 'error' && !item}
        <!-- Nothing to install from here without the store. -->
      {:else}
        {@render control(act, item, data ? { repoId: data.repoId, releaseId: data.latestReleaseId, name: data.name } : null, 'btn')}
      {/if}
      {#if item && alsoPublish && !busy[item.localId]}
        <button class="btn st-detail-secondary" type="button" onclick={() => void publish(item)}>{alsoPublish.label}</button>
      {/if}
      {#if item && item.group !== 'builtin'}
        <button class="btn st-detail-secondary" type="button" onclick={() => void uninstall(item)}>Uninstall…</button>
      {/if}
    </div>
  </header>

  {#if actionError}
    <p class="st-failure" role="alert"><span>{actionError}</span></p>
  {/if}

  {#if item && needsTrust(item)}
    <p class="st-update-note is-warn">It stays off until you trust it. Trusted extensions run with the same access as the app: your projects, files you open, the network, and other extensions’ values.</p>
  {:else if item?.removed}
    <p class="st-update-note">This extension is no longer on the store. It stays on this Mac, but won’t get updates.</p>
  {:else if item?.update}
    <p class="st-update-note">
      {#if item.update.state === 'staged-for-merge'}
        Update available; you changed the files. The new version is beside your folder for your agent to merge.
      {:else if item.update.modified}
        You changed the files since installing {item.origin?.version}. Updating saves {item.update.version} beside your folder for your agent to merge.
      {:else}
        You have {item.origin?.version}. Updating replaces the files with {item.update.version}.
      {/if}
      {#if updateCompare}
        <button class="st-link is-quiet" type="button" aria-expanded={compare?.base === updateCompare.base && compare.head === updateCompare.head} onclick={() => void showCompare(updateCompare.base, updateCompare.head)}>See what changed</button>
      {/if}
    </p>
  {:else if item?.trust === 'store-trusted'}
    <p class="st-update-note">Trusted · runs with full access. <button class="st-link is-quiet" type="button" onclick={() => void revokeTrust(item)}>Revoke Trust</button></p>
  {/if}

  {#if compare}
    <section class="st-sec" aria-live="polite">
      <div class="st-sec-head">
        <h3>What changed{#if compare.state.status === 'ready'}<span class="st-sec-sub">{compareCounts(compare.state.value)}</span>{/if}</h3>
        <button class="btn ghost" type="button" onclick={() => (compare = null)}>Hide</button>
      </div>
      {#if compare.state.status === 'loading'}
        {@render skeleton(2)}
      {:else if compare.state.status === 'error'}
        {@const current = compare}
        {@render failure(compare.state.error, () => { compare = null; void showCompare(current.base, current.head); })}
      {:else if compare.state.value.files.length}
        <div class="st-card">
          {#each compare.state.value.files as file (file.path)}
            <div class="st-kv is-file">
              <b>{file.path}{#if compare.lineage && file.path === 'manifest.json' && file.status === 'modified'}<i class="st-change-why">Names what it was forked from</i>{/if}</b>
              <span class="st-change is-{file.status}">{COMPARE_LABEL[file.status]}</span>
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {/if}

  {#if remote?.status === 'error'}
    {@render failure(remote.error, () => void loadDetail(target))}
  {/if}

  <dl class="st-facts">
    <div><dt>Author</dt><dd>{data ? (ownPage ? 'You' : data.publisher) : preview ? preview.publisher : item && 'builtin' in item.maker ? 'Powermove' : item && 'you' in item.maker ? 'You' : ''}</dd></div>
    <div><dt>Kind</dt><dd>{KIND_LABEL[kind]}</dd></div>
    <div><dt>Updated</dt><dd>{data?.updated ?? preview?.updated ?? '—'}</dd></div>
    <div>
      {#if item?.origin && data && item.origin.version !== data.version}
        <dt>Installed</dt><dd>{item.origin.version} of {data.version}</dd>
      {:else}
        <dt>Version</dt><dd>{version || '—'}</dd>
      {/if}
    </div>
  </dl>

  {#if data?.about}<p class="st-about">{data.about}</p>{/if}

  {#if item && item.group === 'yours' && !item.published}
    <section class="st-sec">
      <h3 class="st-sec-title">Publish</h3>
      <div class="st-card">
        <div class="st-kv"><span>Will publish as</span><b>{account?.handle ? `@${account.handle}/${item.localId}` : account ? 'Choose a handle to publish' : 'Sign in to choose a handle'}</b></div>
      </div>
      <p class="st-note">Publishing puts the source on the store under your name. Setup values stay on this Mac.</p>
    </section>
  {/if}

  {#if ownPage && data}
    <!-- Yours on the store: how it's doing, and every version with a way
         to withdraw it. Withdrawing asks inline, next to the version. -->
    <section class="st-sec">
      <h3 class="st-sec-title">On the store</h3>
      <div class="st-card">
        <div class="st-kv"><span>Installs</span><b>{data.installCount.toLocaleString('en')}</b></div>
        <div class="st-kv"><span>Forks</span><b>{data.forkCount.toLocaleString('en')}</b></div>
        <div class="st-kv"><span>Visibility</span><b>{data.visibility === 'public' ? 'Public' : 'Unlisted'}</b></div>
      </div>
    </section>
    <section class="st-sec">
      <div class="st-sec-head">
        <h3>Versions</h3>
        {#if data.versions.length > 5}
          <button class="btn ghost" type="button" onclick={() => (showAllVersions = !showAllVersions)}>{showAllVersions ? 'Show Fewer' : 'Show All'}</button>
        {/if}
      </div>
      <div class="st-card">
        {#each showAllVersions ? data.versions : data.versions.slice(0, 5) as v (v.id)}
          <div class="st-kv is-version is-own" class:is-withdrawn={v.withdrawn}>
            <span>{v.version}<i>{v.date}</i></span>
            {#if withdrawing === v.id}
              <b class="st-withdraw-ask" role="alert">Withdraw {v.version}? People who have it keep it; nobody new gets it.</b>
              <div class="st-withdraw-actions">
                <button class="btn ghost" type="button" onclick={() => (withdrawing = null)}>Cancel</button>
                <button class="btn st-withdraw" type="button" disabled={!!busy[`yank:${v.id}`]} onclick={() => void withdraw(data.repoId, v)}>{busy[`yank:${v.id}`] ? 'Withdrawing…' : 'Withdraw'}</button>
              </div>
            {:else}
              <b>{#if v.withdrawn}<span class="st-var-req">Withdrawn</span> {/if}{v.note ?? ''}</b>
              {#if !v.withdrawn}
                <button class="st-link is-quiet st-withdraw-open" type="button" aria-label={`Withdraw ${v.version}…`} onclick={() => (withdrawing = v.id)}>Withdraw…</button>
              {/if}
            {/if}
          </div>
        {/each}
      </div>
    </section>
  {:else if data?.versions.length}
    {@const latest = data.versions.find((v) => !v.withdrawn)}
    {#if latest?.note}
      <section class="st-sec">
        <div class="st-sec-head"><h3>What’s new <span class="st-sec-sub">{latest.version}</span></h3></div>
        <p class="st-whatsnew">{latest.note}</p>
      </section>
    {/if}
    <section class="st-sec">
      <div class="st-sec-head">
        <h3>Version history</h3>
        {#if data.versions.length > 5}
          <button class="btn ghost" type="button" onclick={() => (showAllVersions = !showAllVersions)}>{showAllVersions ? 'Show Fewer' : 'Show All'}</button>
        {/if}
      </div>
      <div class="st-card">
        {#each showAllVersions ? data.versions : data.versions.slice(0, 5) as v (v.id)}
          <div class="st-kv is-version" class:is-withdrawn={v.withdrawn}>
            <span>{v.version}<i>{v.date}</i></span>
            <b>{#if v.withdrawn}<span class="st-var-req">Withdrawn</span> {/if}{v.note ?? ''}</b>
          </div>
        {/each}
      </div>
    </section>
  {/if}

  {#if vars.length}
    <!-- Values the extension reads at runtime. They live in the app
         profile, never in the extension folder, so publishing never
         carries them. Users may leave any value blank. -->
    <section class="st-sec">
      <div class="st-sec-head">
        <h3>Setup</h3>
        {#if item}<button class="btn ghost" type="button" onclick={() => void setUp(item.localId)}>Set Up…</button>{/if}
      </div>
      <div class="st-card">
        {#each vars as v (v.key)}
          <div class="st-var">
            <div class="st-var-copy">
              <b>{v.label}</b>
              <span>{v.hint ?? ''} <code>{v.key}</code></span>
            </div>
          </div>
        {/each}
      </div>
      <p class="st-note">{item ? 'Stays on this Mac. Never included when you publish or share this extension.' : 'You can set these values when you install. They stay on this Mac.'}</p>
    </section>
  {/if}

  <section class="st-sec">
    <h3 class="st-sec-title">Details</h3>
    <div class="st-card">
      {#if coord}<div class="st-kv"><span>Identifier</span><b>{coord}</b></div>{/if}
      {#if storeLineage}<div class="st-kv"><span>Forked from</span><b>{storeLineage.handle}/{storeLineage.slug}@{storeLineage.version}</b></div>{/if}
      {#if contributes.length}<div class="st-kv"><span>Includes</span><b>{includesText(contributes)}</b></div>{/if}
      {#if apiVersion !== null}<div class="st-kv"><span>Compatibility</span><b>{requiresText(apiVersion)}</b></div>{/if}
    </div>
    {#if item?.group !== 'builtin' && (data || item?.group === 'store')}
      {#if (data?.permissions ?? preview?.permissions ?? item?.permissions ?? []).includes('full-access') || (apiVersion !== null && apiVersion < 3)}
        <p class="st-note">This extension needs full access to Powermove. Only trust code from people you trust.</p>
      {:else if apiVersion === 3}
        <p class="st-note">This extension runs in a sandbox with the permissions listed above.</p>
      {/if}
    {/if}
  </section>

  {#if files?.status === 'ready' && files.value.length}
    <section class="st-sec">
      <h3 class="st-sec-title">Source</h3>
      <div class="st-card">
        {#each files.value as file (file.path)}
          <button class="st-file" class:is-open={openFile?.path === file.path} type="button" aria-expanded={openFile?.path === file.path} onclick={() => void toggleFile(file.path)}>
            <span>{file.path}</span><Icon {PM} name="chev" />
          </button>
          {#if openFile?.path === file.path}
            {#if openFile.state.status === 'loading'}
              <pre class="st-source" aria-busy="true"></pre>
            {:else if openFile.state.status === 'error'}
              <p class="st-failure st-source-error"><span>{openFile.state.error.message}</span></p>
            {:else}
              <pre class="st-source">{openFile.state.value}</pre>
            {/if}
          {/if}
        {/each}
      </div>
    </section>
  {/if}
{/snippet}
