<script lang="ts">
  import { tick } from 'svelte';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import Icon from '../panels/Icon.svelte';
  import { mountSquircles, SQUIRCLE_SELECTOR } from '../settings/squircle';
  import { mountNavGlide } from '../controls/nav-glide';
  import { openSignIn, subscribeAccount, type CloudUser } from '../cloud/account';
  import {
    ALL, BUILTINS, FEATURED, KINDS, KIND_ICON, KIND_LABEL, KIND_PLURAL, LIBRARY, NEW, PICKS,
    coordinate, hasUpdate, libraryItemFor,
    type LibraryItem, type StoreKind, type StoreListing
  } from './fixtures';

  /* Two places and six kinds. Browse is the storefront; a kind is the store
     narrowed to one shelf; Library is everything on this Mac and everything
     you have published, in one list. */
  export type StorePage = 'browse' | 'library' | `kind:${StoreKind}`;

  let { PM }: { PM: Record<string, any> } = $props();

  /* Screen motion: the whole surface glides in from the right over the home,
     and glides back out. Views inside slide the way you moved: deeper goes
     right-to-left, back goes left-to-right, a sibling page just settles. */
  const SLIDE = 24;
  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

  let shown = $state(false);
  let leaving = $state(false);
  let page = $state<StorePage>('browse');
  let searchText = $state('');
  let featuredIndex = $state(0);
  let detail = $state<StoreListing | null>(null);
  let direction = $state(0);
  let rootEl = $state<HTMLElement | null>(null);
  let scrollEl = $state<HTMLElement | null>(null);
  let lastFocus: HTMLElement | null = null;
  let leaveTimer = 0;
  let account = $state<CloudUser | null>(null);

  $effect(() => subscribeAccount((user) => { account = user; }));

  const query = $derived(searchText.trim().toLowerCase());
  const featured = $derived(FEATURED[featuredIndex]!);
  const results = $derived(query
    ? ALL.filter((l) => `${l.name} ${l.tagline} ${l.publisher}`.toLowerCase().includes(query))
    : []);
  const pageKind = $derived(page.startsWith('kind:') ? (page.slice(5) as StoreKind) : null);
  const viewKey = $derived(detail ? `detail:${coordinate(detail)}` : query ? 'search' : page);
  /* Sibling pages swap in place. Only pushing into and popping out of a
     detail slides, in the direction you moved. */
  const slide = $derived(reduced() || direction === 0 ? { duration: 0 } : { x: direction * SLIDE, duration: 220, easing: cubicOut });

  /* Library groups. Store installs come first because they are the ones that
     change under you; yours next; built-ins last and quiet. */
  const fromStore = $derived(LIBRARY.filter((i) => i.origin === 'store'));
  const yours = $derived(LIBRARY.filter((i) => i.origin === 'agent' || i.origin === 'you'));
  const attention = $derived(LIBRARY.filter((i) => i.needsSetup || hasUpdate(i)).length);

  export function open(target: StorePage = 'browse'): void {
    window.clearTimeout(leaveTimer);
    leaving = false;
    page = target;
    detail = null;
    direction = 0;
    searchText = '';
    if (!shown) {
      lastFocus = document.activeElement as HTMLElement | null;
      shown = true;
    }
    PM.bus?.emit?.('store:screen');
    void tick().then(() => {
      scrollEl?.scrollTo({ top: 0, behavior: 'instant' });
      rootEl?.focus({ preventScroll: true });
    });
  }

  export function close(): void {
    if (!shown || leaving) return;
    leaving = true;
    const finish = () => {
      shown = false;
      leaving = false;
      PM.bus?.emit?.('store:screen');
      const focus = lastFocus;
      lastFocus = null;
      if (focus?.isConnected) focus.focus();
    };
    if (reduced()) finish();
    else leaveTimer = window.setTimeout(finish, 200);
  }

  export function isOpen(): boolean {
    return shown;
  }

  function show(id: StorePage): void {
    direction = 0;
    page = id;
    detail = null;
    searchText = '';
    void tick().then(() => scrollEl?.scrollTo({ top: 0, behavior: 'instant' }));
  }

  function openDetail(listing: StoreListing): void {
    direction = 1;
    detail = listing;
    void tick().then(() => scrollEl?.scrollTo({ top: 0, behavior: 'instant' }));
  }

  /* "Forked from" leads to the origin's own page, one level deeper. */
  function openOrigin(forkedFrom: string): void {
    const coord = forkedFrom.split('@')[0];
    const origin = ALL.find((l) => coordinate(l) === coord) ?? BUILTINS.find((l) => coordinate(l) === coord);
    if (origin) openDetail(origin);
  }

  function back(): void {
    direction = -1;
    detail = null;
  }

  /* Only Escape is ours; app shortcuts such as ⌘, keep working over the Store. */
  function keydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    event.stopPropagation();
    event.preventDefault();
    if (detail) back();
    else close();
  }

  /* The sidebar highlight glides between rows. */
  function glide(node: HTMLElement) {
    const unmount = mountNavGlide(node, { row: '.st-navbtn', selected: '.on' });
    return { destroy: unmount };
  }

  /* Lisse squircles on the cards, previews and controls while the screen is up. */
  $effect(() => {
    if (!shown || !rootEl) return;
    const root = rootEl;
    const selector = `${SQUIRCLE_SELECTOR}, .st-card, .st-thumb, .st-hero, .st-navbtn, .st-search`;
    let unmount = mountSquircles(root, selector);
    const observer = new MutationObserver(() => { unmount(); unmount = mountSquircles(root, selector); });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => { observer.disconnect(); unmount(); };
  });

  function art(listing: StoreListing): string {
    const [a, b] = listing.art;
    return `--art-a:${a};--art-b:${b}`;
  }

  /* Who made it, as the store says it: "Effect by mara". Your own read as
     yours; the agent's as made with your agent. */
  function maker(l: StoreListing): string {
    const item = l as LibraryItem;
    if (item.origin === 'agent') return 'Made with your agent';
    if (item.origin === 'you') return 'By you';
    if (item.origin === 'builtin') return 'Built in';
    return `by ${l.publisher}`;
  }

  function byline(listing: StoreListing): string {
    const who = maker(listing);
    return who.startsWith('by ') ? `${KIND_LABEL[listing.kind]} ${who}` : `${KIND_LABEL[listing.kind]} · ${who}`;
  }

  /* Your own extensions read under your handle once you have one. */
  function shownCoordinate(l: StoreListing): string {
    return l.publisher === 'you' && account?.handle ? `${account.handle}/${l.id}` : coordinate(l);
  }

  /* The one trailing control a row gets, from its local state. */
  type RowAction = { label: string; primary?: boolean; quiet?: boolean };
  function action(l: StoreListing): RowAction {
    const item = (l as LibraryItem).origin ? (l as LibraryItem) : libraryItemFor(l);
    if (!item) return l.installed ? { label: 'Installed', quiet: true } : { label: l.vars?.some((v) => v.required) ? 'Install and set up' : 'Install' };
    if (item.origin === 'builtin') return { label: 'On', quiet: true };
    if (item.needsSetup) return { label: 'Set up', primary: true };
    if (hasUpdate(item)) return { label: item.modified ? 'Update…' : 'Update', primary: true };
    if (!item.installedVersion) return { label: 'Install' };
    if (item.origin === 'store') return { label: 'Installed', quiet: true };
    if (item.published) return { label: item.published.version === item.version ? 'Published' : 'Publish update', quiet: item.published.version === item.version };
    return { label: 'Publish' };
  }

  /* What a Library row says under the name: the maker, then what's up. */
  function status(item: LibraryItem): string | null {
    if (item.needsSetup) return 'Needs setup';
    if (hasUpdate(item)) return item.modified ? `${item.version} available, you changed the files` : `${item.version} available`;
    if (!item.installedVersion) return 'Not on this Mac';
    if (item.published) return `@${account?.handle ?? 'you'}/${item.id} · ${item.published.installs} installs${item.published.forks ? ` · ${item.published.forks} forks` : ''}`;
    return null;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  id="store-screen"
  class:on={shown}
  class:leaving
  role="dialog"
  aria-label="Store"
  tabindex="-1"
  bind:this={rootEl}
  onkeydown={keydown}
>
  <aside class="st-sidebar">
    <label class="st-search">
      <Icon {PM} name="search" />
      <input type="search" placeholder="Search extensions" aria-label="Search extensions" bind:value={searchText} />
    </label>
    <nav class="st-nav" aria-label="Store sections" use:glide>
      {@render navbtn('browse', 'Browse', 'sparkle')}
      <span class="st-nav-label">Kinds</span>
      {#each KINDS as k (k)}
        {@render navbtn(`kind:${k}`, KIND_PLURAL[k], KIND_ICON[k])}
      {/each}
      <span class="st-nav-label">This Mac</span>
      {@render navbtn('library', 'Library', 'stack', attention)}
    </nav>
    <button class="st-navbtn st-done" type="button" aria-label="Done" onclick={close}>
      <Icon {PM} name="chev" />
      <span>Done</span>
    </button>
  </aside>

  <main class="st-main">
    <div class="st-scroll" bind:this={scrollEl}>
      {#key viewKey}
        <div class="st-column" in:fly={slide}>
          {#if detail}
            {@const l = detail}
            {@const item = (l as LibraryItem).origin ? (l as LibraryItem) : libraryItemFor(l)}
            {@const act = action(l)}
            <button class="st-back" type="button" onclick={back}>
              <Icon {PM} name="chev" /><span>Back</span>
            </button>
            <!-- Icon, copy, then the one action at the right edge, all on one
                 line like the hero card and every row. Anything the action
                 needs to explain goes under the head as its own line. -->
            <header class="st-detail-head">
              <span class="st-thumb is-hero" style={art(l)}></span>
              <div class="st-detail-copy">
                <div class="st-detail-title">
                  <h2>{l.name}</h2>
                  <span class="st-tag">{l.version}</span>
                </div>
                <p class="st-byline">{byline(l)}</p>
                <p class="st-lede">{l.tagline}</p>
                {#if l.forkedFrom}
                  <p class="st-lineage">
                    Forked from <button class="st-link" type="button" onclick={() => openOrigin(l.forkedFrom!)}>{l.forkedFrom.split('@')[0]}</button>
                    <span class="st-lineage-sep">·</span>
                    <button class="st-link is-quiet" type="button">See what changed</button>
                  </p>
                {/if}
              </div>
              <div class="st-detail-actions">
                <button class="btn" class:pri={act.primary || (!act.quiet && act.label === 'Install')} class:is-quiet={act.quiet} type="button">{act.label}</button>
              </div>
            </header>
            {#if item && hasUpdate(item)}
              <p class="st-update-note">
                {#if item.modified}
                  You changed the files since installing {item.installedVersion}. Updating merges {item.version} with your changes; your agent resolves anything that overlaps.
                {:else}
                  You have {item.installedVersion}. Updating replaces the files with {item.version}.
                {/if}
              </p>
            {/if}

            <dl class="st-facts">
              <div><dt>Author</dt><dd>{item?.origin === 'agent' || item?.origin === 'you' ? 'You' : l.publisher}</dd></div>
              <div><dt>Kind</dt><dd>{KIND_LABEL[l.kind]}</dd></div>
              <div><dt>Updated</dt><dd>{l.updated}</dd></div>
              <div><dt>{item?.installedVersion && item.installedVersion !== l.version ? 'Installed' : 'Version'}</dt><dd>{item?.installedVersion && item.installedVersion !== l.version ? `${item.installedVersion} of ${l.version}` : l.version}</dd></div>
            </dl>

            {#if l.about}<p class="st-about">{l.about}</p>{/if}

            {#if item?.published}
              <!-- Published: the store's numbers for your own extension. -->
              <section class="st-sec">
                <h3 class="st-sec-title">On the store</h3>
                <div class="st-card">
                  <div class="st-kv"><span>Published as</span><b>@{account?.handle ?? 'you'}/{l.id}</b></div>
                  <div class="st-kv"><span>Installs</span><b>{item.published.installs}</b></div>
                  <div class="st-kv"><span>Forks</span><b>{item.published.forks || 'None yet'}</b></div>
                  {#if item.installedVersion && item.installedVersion !== item.published.version}
                    <div class="st-kv"><span>This Mac</span><b>{item.installedVersion}, ahead of the published {item.published.version}</b></div>
                  {/if}
                </div>
              </section>
            {:else if item && (item.origin === 'agent' || item.origin === 'you')}
              <section class="st-sec">
                <h3 class="st-sec-title">Publish</h3>
                <div class="st-card">
                  <div class="st-kv"><span>Will publish as</span><b>{account?.handle ? `@${account.handle}/${l.id}` : account ? 'Choose a handle to publish' : 'Sign in to choose a handle'}</b></div>
                  {#if l.forkedFrom}<div class="st-kv"><span>Forked from</span><b>{l.forkedFrom}</b></div>{/if}
                </div>
                <p class="st-note">Publishing puts the source on the store under your name. Setup values stay on this Mac.</p>
              </section>
            {/if}

            {#if l.versions?.[0]}
              {@const v = l.versions[0]}
              <section class="st-sec">
                <div class="st-sec-head">
                  <h3>What’s new <span class="st-sec-sub">{v.version}</span></h3>
                  <button class="btn ghost" type="button">Version history</button>
                </div>
                <p class="st-whatsnew">{v.note}</p>
              </section>
            {/if}

            {#if l.vars?.length}
              <!-- Variables: keys the extension reads at runtime. Values live in
                   the app profile's env files, never in the extension folder,
                   so publishing never carries them. Required ones gate
                   activation; the rest can be set here any time. -->
              <section class="st-sec">
                <h3 class="st-sec-title">Setup</h3>
                <div class="st-card">
                  {#each l.vars as v (v.key)}
                    <div class="st-var">
                      <div class="st-var-copy">
                        <b>{v.label}{#if v.required} <span class="st-var-req">Required</span>{/if}</b>
                        <span>{v.hint ?? ''} <code>{v.key}</code></span>
                      </div>
                      <input class="settings-input st-var-input" type={v.secret ? 'password' : 'text'} placeholder={v.secret ? '••••••••' : 'Not set'} aria-label={v.label} />
                    </div>
                  {/each}
                </div>
                <p class="st-note">Stays on this Mac. Never included when you publish or share this extension.</p>
              </section>
            {/if}

            <section class="st-sec">
              <h3 class="st-sec-title">Details</h3>
              <div class="st-card">
                <div class="st-kv"><span>Identifier</span><b>{shownCoordinate(l)}</b></div>
                {#if l.forkedFrom}<div class="st-kv"><span>Forked from</span><b>{l.forkedFrom}</b></div>{/if}
                <div class="st-kv"><span>Includes</span><b>{KIND_PLURAL[l.kind]}, Inspector</b></div>
                <div class="st-kv"><span>Requires</span><b>Powermove 1.0 or later</b></div>
              </div>
              <p class="st-note">Extensions run inside Powermove with the same access as the app. Read the source or install from people you know.</p>
            </section>

            {#if l.files}
              <section class="st-sec">
                <h3 class="st-sec-title">Source</h3>
                <div class="st-card">
                  {#each l.files as file (file)}
                    <button class="st-file" type="button"><span>{file}</span><Icon {PM} name="chev" /></button>
                  {/each}
                </div>
              </section>
            {/if}


          {:else if query}
            <header class="st-heading">
              <div>
                <h2>Search</h2>
                <p>{results.length} {results.length === 1 ? 'extension' : 'extensions'} matching “{searchText.trim()}”</p>
              </div>
            </header>
            <div class="st-grid">
              {#each results as l (coordinate(l))}
                {@render row(l)}
              {/each}
            </div>

          {:else if page === 'browse'}
            <header class="st-heading">
              <div>
                <h2>Store</h2>
                <p>Effects, transitions, panels and themes made by people using Powermove.</p>
              </div>
            </header>

            <!-- One extension at a time: its icon beside its copy, on a card.
                 The pager and Install share the card's last line. -->
            <div class="st-hero">
              <button class="st-hero-open" type="button" aria-label={`Open ${featured.name}`} onclick={() => openDetail(featured)}></button>
              <span class="st-thumb is-featured" style={art(featured)}></span>
              <div class="st-hero-copy">
                <span class="st-hero-kind">{byline(featured)}</span>
                <b>{featured.name}</b>
                <span class="st-hero-line">{featured.tagline}</span>
                <div class="st-pager" role="tablist" aria-label="Featured">
                  {#each FEATURED as f, i (f.id)}
                    <button role="tab" type="button" aria-selected={i === featuredIndex} aria-label={f.name} onclick={() => (featuredIndex = i)}></button>
                  {/each}
                </div>
              </div>
              <button class="btn pri st-hero-install" type="button">Install</button>
            </div>

            {@render section('Picks', PICKS)}
            {@render section('New', NEW)}
            {@render section('Effects', ALL.filter((l) => l.kind === 'effects'), 'kind:effects')}
            {@render section('Transitions', ALL.filter((l) => l.kind === 'transitions'), 'kind:transitions')}

          {:else if pageKind}
            {@const shelf = ALL.filter((l) => l.kind === pageKind)}
            <header class="st-heading">
              <div>
                <h2>{KIND_PLURAL[pageKind]}</h2>
                <p>{shelf.length} {shelf.length === 1 ? 'extension' : 'extensions'}</p>
              </div>
            </header>
            <div class="st-grid">
              {#each shelf as l (coordinate(l))}
                {@render row(l)}
              {/each}
            </div>

          {:else}
            <!-- Library: one list of everything here, grouped by where it came
                 from. Each row names its maker and says what's up with it, so
                 there is no separate Installed or Yours to keep in step. -->
            <header class="st-heading">
              <div>
                <h2>Library</h2>
                <p>{account?.handle ? `Everything on this Mac, and what you’ve published as @${account.handle}.` : account ? 'Everything on this Mac.' : 'Everything on this Mac. Sign in to publish yours.'}</p>
              </div>
              {#if !account}
                <button class="btn" type="button" onclick={() => openSignIn()}>Sign In…</button>
              {/if}
            </header>

            {@render group('From the store', fromStore, 'Extensions you install from the store show up here, with updates from their makers.')}
            {@render group('Yours', yours, 'Extensions you or your agent make show up here. Publish one to put it on the store under your name.')}
            {@render group('Built in', BUILTINS)}
          {/if}
        </div>
      {/key}
    </div>
  </main>
</div>

{#snippet navbtn(id: StorePage, label: string, icon: string, count = 0)}
  <button
    class="st-navbtn"
    class:on={id === page && !query}
    type="button"
    aria-current={id === page && !query ? 'location' : undefined}
    onclick={() => show(id)}
  >
    <Icon {PM} name={icon} />
    <span>{label}</span>
    {#if count}<span class="st-nav-count" aria-label={`${count} need attention`}>{count}</span>{/if}
  </button>
{/snippet}

{#snippet row(l: StoreListing)}
  {@const act = action(l)}
  <div class="st-row">
    <button class="st-row-open" type="button" onclick={() => openDetail(l)}>
      <span class="st-thumb" style={art(l)}></span>
      <span class="st-row-copy">
        <b>{l.name} <span class="st-row-by">{maker(l)}</span></b>
        <span class="st-row-line">{l.tagline}</span>
      </span>
    </button>
    {#if act.quiet}
      <span class="st-installed">{act.label}</span>
    {:else}
      <button class="btn st-install" class:pri={act.primary} type="button">{act.label}</button>
    {/if}
  </div>
{/snippet}

{#snippet libraryRow(item: LibraryItem)}
  {@const act = action(item)}
  {@const note = status(item)}
  <div class="st-row is-library" class:is-off={item.needsSetup}>
    <button class="st-row-open" type="button" onclick={() => openDetail(item)}>
      <span class="st-thumb" style={art(item)}></span>
      <span class="st-row-copy">
        <b>{item.name} <span class="st-row-by">{maker(item)}{#if item.forkedFrom}&nbsp;· forked from {item.forkedFrom.split('@')[0]}{/if}</span></b>
        <span class="st-row-line">{item.tagline}</span>
        {#if note}<span class="st-row-status" class:is-hot={item.needsSetup || hasUpdate(item)}>{note}</span>{/if}
      </span>
    </button>
    {#if act.quiet}
      <span class="st-installed">{act.label}</span>
    {:else}
      <button class="btn st-install" class:pri={act.primary} type="button">{act.label}</button>
    {/if}
  </div>
{/snippet}

{#snippet group(title: string, items: LibraryItem[], empty?: string)}
  <section class="st-sec">
    <div class="st-sec-head">
      <h3>{title}</h3>
      {#if title === 'Yours' && items.length}
        <button class="btn ghost" type="button">Publish…</button>
      {/if}
    </div>
    {#if items.length}
      <div class="st-list">
        {#each items as item (`${item.origin}:${coordinate(item)}`)}
          {@render libraryRow(item)}
        {/each}
      </div>
    {:else if empty}
      <p class="st-group-empty">{empty}</p>
    {/if}
  </section>
{/snippet}

{#snippet section(title: string, items: StoreListing[], more?: StorePage)}
  {#if items.length}
    <section class="st-sec">
      <div class="st-sec-head">
        <h3>{title}</h3>
        {#if more}<button class="btn ghost" type="button" onclick={() => show(more)}>See all</button>{/if}
      </div>
      <div class="st-grid">
        {#each items as l (coordinate(l))}
          {@render row(l)}
        {/each}
      </div>
    </section>
  {/if}
{/snippet}
