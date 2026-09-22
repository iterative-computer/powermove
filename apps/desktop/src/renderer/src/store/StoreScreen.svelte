<script lang="ts">
  import { tick } from 'svelte';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import Icon from '../panels/Icon.svelte';
  import { createExtensionSettingsControl } from '../legacy/ui/extension-settings';
  import { mountSquircles, SQUIRCLE_SELECTOR } from '../settings/squircle';
  import { mountNavGlide } from '../controls/nav-glide';
  import {
    ALL, FEATURED, KIND_LABEL, KIND_PLURAL, NEW, PICKS, coordinate,
    type StoreKind, type StoreListing
  } from './fixtures';

  export type StorePage = 'browse' | 'installed' | 'yours';

  let { PM }: { PM: Record<string, any> } = $props();

  const NAV: Array<{ id: StorePage; label: string; icon: string }> = [
    { id: 'browse', label: 'Browse', icon: 'sparkle' },
    { id: 'installed', label: 'Installed', icon: 'download' },
    { id: 'yours', label: 'Yours', icon: 'code' }
  ];

  /* Screen motion: the whole surface glides in from the right over the home,
     and glides back out. Views inside slide the way you moved: deeper goes
     right-to-left, back goes left-to-right, a sibling page just settles. */
  const SLIDE = 24;
  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

  let shown = $state(false);
  let leaving = $state(false);
  let page = $state<StorePage>('browse');
  let searchText = $state('');
  let kind = $state<StoreKind | 'all'>('all');
  let featuredIndex = $state(0);
  let detail = $state<StoreListing | null>(null);
  let direction = $state(0);
  let rootEl = $state<HTMLElement | null>(null);
  let scrollEl = $state<HTMLElement | null>(null);
  let installed = $state.raw<ReturnType<typeof createExtensionSettingsControl> | null>(null);
  let lastFocus: HTMLElement | null = null;
  let leaveTimer = 0;

  const query = $derived(searchText.trim().toLowerCase());
  const featured = $derived(FEATURED[featuredIndex]!);
  const results = $derived(query
    ? ALL.filter((l) => `${l.name} ${l.tagline} ${l.publisher}`.toLowerCase().includes(query))
    : []);
  const filtered = $derived((list: StoreListing[]) => kind === 'all' ? list : list.filter((l) => l.kind === kind));
  const viewKey = $derived(detail ? `detail:${coordinate(detail)}` : query ? 'search' : page);
  /* Sibling pages swap in place. Only pushing into and popping out of a
     detail slides, in the direction you moved. */
  const slide = $derived(reduced() || direction === 0 ? { duration: 0 } : { x: direction * SLIDE, duration: 220, easing: cubicOut });

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
    void tick().then(() => scrollEl?.scrollTo({ top: 0, behavior: 'instant' }));
  }

  function openDetail(listing: StoreListing): void {
    direction = 1;
    detail = listing;
    void tick().then(() => scrollEl?.scrollTo({ top: 0, behavior: 'instant' }));
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

  /* Installed reuses the Settings › Extensions control as is: the same rows,
     switch and in-place detail, so the two places never drift apart. */
  $effect(() => {
    if (!shown || page !== 'installed' || installed) return;
    installed = createExtensionSettingsControl(undefined, { includeBuiltin: true });
  });
  $effect(() => {
    if (shown) return;
    installed?.destroy();
    installed = null;
  });

  /* Lisse squircles on the cards, previews and controls while the screen is up. */
  $effect(() => {
    if (!shown || !rootEl) return;
    const root = rootEl;
    const selector = `${SQUIRCLE_SELECTOR}, .st-card, .st-thumb, .st-hero, .st-navbtn, .st-search, .st-kind`;
    let unmount = mountSquircles(root, selector);
    const observer = new MutationObserver(() => { unmount(); unmount = mountSquircles(root, selector); });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => { observer.disconnect(); unmount(); };
  });

  type Hosted = HTMLElement | null | undefined;
  function host(node: HTMLElement, hosted: Hosted) {
    const place = (next: Hosted) => node.replaceChildren(...(next ? [next] : []));
    place(hosted);
    return { update: place };
  }

  function art(listing: StoreListing): string {
    const [a, b] = listing.art;
    return `--art-a:${a};--art-b:${b}`;
  }

  function byline(listing: StoreListing): string {
    return `${KIND_LABEL[listing.kind]} by ${listing.publisher}`;
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
      {#each NAV as item (item.id)}
        <button
          class="st-navbtn"
          class:on={item.id === page && !query}
          type="button"
          aria-current={item.id === page && !query ? 'location' : undefined}
          onclick={() => show(item.id)}
        >
          <Icon {PM} name={item.icon} />
          <span>{item.label}</span>
        </button>
      {/each}
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
            <button class="st-back" type="button" onclick={back}>
              <Icon {PM} name="chev" /><span>Back</span>
            </button>
            <header class="st-detail-head">
              <span class="st-thumb is-hero" style={art(l)}></span>
              <div class="st-detail-copy">
                <div class="st-detail-title">
                  <h2>{l.name}</h2>
                  <span class="st-tag">{l.version}</span>
                </div>
                <p class="st-lede">{l.tagline}</p>
                {#if l.forkedFrom}
                  <p class="st-lineage">Forked from <button class="st-link" type="button">{l.forkedFrom.split('@')[0]}</button></p>
                {/if}
                <div class="st-detail-actions">
                  {#if l.installed}
                    <button class="btn" type="button">Installed</button>
                  {:else if l.vars?.some((v) => v.required)}
                    <button class="btn pri" type="button">Install and set up</button>
                  {:else}
                    <button class="btn pri" type="button">Install</button>
                  {/if}
                </div>
              </div>
            </header>

            <dl class="st-facts">
              <div><dt>Author</dt><dd>{l.publisher}</dd></div>
              <div><dt>Kind</dt><dd>{KIND_LABEL[l.kind]}</dd></div>
              <div><dt>Updated</dt><dd>{l.updated}</dd></div>
              <div><dt>Version</dt><dd>{l.version}</dd></div>
            </dl>

            {#if l.about}<p class="st-about">{l.about}</p>{/if}

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
              <!-- Variables: keys the extension reads at runtime. Values are kept
                   on this Mac beside the extension, outside the package, so
                   publishing never carries them. Required ones are asked for at
                   install; the rest can be set here any time. -->
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
                <div class="st-kv"><span>Identifier</span><b>{coordinate(l)}</b></div>
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
              <select class="st-kind" aria-label="Kind" bind:value={kind}>
                <option value="all">All kinds</option>
                {#each Object.entries(KIND_PLURAL) as [value, label] (value)}
                  <option {value}>{label}</option>
                {/each}
              </select>
            </header>

            {#if kind === 'all'}
              <!-- One extension at a time: its preview frame beside its copy, on
                   a card. The pager and Install share the card's last line. -->
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
            {/if}

            {@render section('Picks', filtered(PICKS))}
            {@render section('New', filtered(NEW))}
            {#if kind === 'all'}
              {@render section('Effects', ALL.filter((l) => l.kind === 'effects'))}
              {@render section('Transitions', ALL.filter((l) => l.kind === 'transitions'))}
            {/if}

          {:else if page === 'installed'}
            <header class="st-heading">
              <div>
                <h2>Installed</h2>
                <p>Extensions from the store, and the ones you or your agent made.</p>
              </div>
            </header>
            <div class="sg-column st-installed-host" use:host={installed?.element}></div>

          {:else}
            <header class="st-heading">
              <div>
                <h2>Yours</h2>
                <p>Extensions you have published, and their forks.</p>
              </div>
            </header>
            <div class="st-empty">
              <b>Nothing published yet</b>
              <span>Publishing puts an extension’s source on the store under your name. Forks record where they came from.</span>
              <button class="btn" type="button">Publish an extension…</button>
            </div>
          {/if}
        </div>
      {/key}
    </div>
  </main>
</div>

{#snippet row(l: StoreListing)}
  <div class="st-row">
    <button class="st-row-open" type="button" onclick={() => openDetail(l)}>
      <span class="st-thumb" style={art(l)}></span>
      <span class="st-row-copy">
        <b>{l.name}</b>
        <span class="st-row-line">{l.tagline}</span>
        <span class="st-row-by">{l.publisher}</span>
      </span>
    </button>
    {#if l.installed}
      <span class="st-installed">Installed</span>
    {:else}
      <button class="btn st-install" type="button">Install</button>
    {/if}
  </div>
{/snippet}

{#snippet section(title: string, items: StoreListing[])}
  {#if items.length}
    <section class="st-sec">
      <div class="st-sec-head">
        <h3>{title}</h3>
        <button class="btn ghost" type="button">See all</button>
      </div>
      <div class="st-grid">
        {#each items as l (coordinate(l))}
          {@render row(l)}
        {/each}
      </div>
    </section>
  {/if}
{/snippet}
