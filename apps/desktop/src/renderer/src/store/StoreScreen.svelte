<script lang="ts">
  import { tick } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import {
    ALL, FEATURED, INSTALLED, KIND_LABEL, KIND_PLURAL, NEW, PICKS, coordinate,
    type StoreKind, type StoreListing
  } from './fixtures';

  export type StorePage = 'browse' | 'installed' | 'yours';

  let { PM }: { PM: Record<string, any> } = $props();

  const NAV: Array<{ id: StorePage; label: string; icon: string }> = [
    { id: 'browse', label: 'Browse', icon: 'sparkle' },
    { id: 'installed', label: 'Installed', icon: 'download' },
    { id: 'yours', label: 'Yours', icon: 'code' }
  ];

  let shown = $state(false);
  let page = $state<StorePage>('browse');
  let searchText = $state('');
  let kind = $state<StoreKind | 'all'>('all');
  let featuredIndex = $state(0);
  let detail = $state<StoreListing | null>(null);
  let rootEl = $state<HTMLElement | null>(null);
  let scrollEl = $state<HTMLElement | null>(null);
  let lastFocus: HTMLElement | null = null;

  const query = $derived(searchText.trim().toLowerCase());
  const featured = $derived(FEATURED[featuredIndex]!);
  const results = $derived(query
    ? ALL.filter((l) => `${l.name} ${l.tagline} ${l.publisher}`.toLowerCase().includes(query))
    : []);
  const filtered = $derived((list: StoreListing[]) => kind === 'all' ? list : list.filter((l) => l.kind === kind));

  export function open(target: StorePage = 'browse'): void {
    page = target;
    detail = null;
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
    if (!shown) return;
    shown = false;
    PM.bus?.emit?.('store:screen');
    const focus = lastFocus;
    lastFocus = null;
    if (focus?.isConnected) focus.focus();
  }

  export function isOpen(): boolean {
    return shown;
  }

  function show(id: StorePage): void {
    page = id;
    detail = null;
    void tick().then(() => scrollEl?.scrollTo({ top: 0, behavior: 'instant' }));
  }

  function openDetail(listing: StoreListing): void {
    detail = listing;
    void tick().then(() => scrollEl?.scrollTo({ top: 0, behavior: 'instant' }));
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key !== 'Escape') return;
    event.preventDefault();
    if (detail) detail = null;
    else close();
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
    <nav class="st-nav" aria-label="Store sections">
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
    <div class="st-actions">
      <button class="btn" type="button" onclick={close}>Done</button>
    </div>
    <div class="st-scroll" bind:this={scrollEl}>
      <div class="st-column">
        {#if detail}
          {@const l = detail}
          <button class="st-back" type="button" onclick={() => (detail = null)}>
            <Icon {PM} name="chev" /><span>Back</span>
          </button>
          <div class="st-hero is-detail" style={art(l)}></div>
          <header class="st-detail-head">
            <div class="st-detail-title">
              <h2>{l.name}</h2>
              <span class="st-tag">{l.version}</span>
            </div>
            <p class="st-byline">{byline(l)} · Updated {l.updated}</p>
            <p class="st-lede">{l.tagline}</p>
            <div class="st-detail-actions">
              {#if l.installed}
                <button class="btn" type="button">Installed</button>
              {:else}
                <button class="btn pri" type="button">Install</button>
              {/if}
              <button class="btn" type="button">Fork and edit</button>
              <button class="btn ghost" type="button">View source</button>
            </div>
          </header>
          {#if l.about}<p class="st-about">{l.about}</p>{/if}

          <section class="st-sec">
            <h3 class="st-sec-title">About</h3>
            <div class="st-card">
              <div class="st-kv"><span>Identifier</span><b>{coordinate(l)}</b></div>
              <div class="st-kv"><span>Version</span><b>{l.version}</b></div>
              {#if l.forkedFrom}<div class="st-kv"><span>Based on</span><b>{l.forkedFrom}</b></div>{/if}
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

          {#if l.versions}
            <section class="st-sec">
              <h3 class="st-sec-title">Versions</h3>
              <div class="st-card">
                {#each l.versions as v (v.version)}
                  <div class="st-kv is-version"><span>{v.version} <i>{v.date}</i></span><b>{v.note}</b></div>
                {/each}
              </div>
            </section>
          {/if}

        {:else if query}
          <header class="st-heading">
            <h2>Search</h2>
            <p>{results.length} {results.length === 1 ? 'extension' : 'extensions'} matching “{searchText.trim()}”</p>
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
            <div class="st-featured">
              <div class="st-hero" style={art(featured)}>
                <button class="st-hero-open" type="button" aria-label={featured.name} onclick={() => openDetail(featured)}></button>
                <div class="st-hero-copy">
                  <span class="st-hero-kind">{byline(featured)}</span>
                  <b>{featured.name}</b>
                  <span class="st-hero-line">{featured.tagline}</span>
                </div>
                <button class="btn pri st-hero-install" type="button">Install</button>
              </div>
              <div class="st-pager" role="tablist" aria-label="Featured">
                {#each FEATURED as f, i (f.id)}
                  <button role="tab" type="button" aria-selected={i === featuredIndex} aria-label={f.name} onclick={() => (featuredIndex = i)}></button>
                {/each}
              </div>
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
          <div class="st-grid is-one">
            {#each INSTALLED as l (coordinate(l))}
              {@render row(l, l.id === 'paper' ? 'Update' : 'Installed')}
            {/each}
          </div>

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
    </div>
  </main>
</div>

{#snippet row(l: StoreListing, trailing: string = l.installed ? 'Installed' : 'Install')}
  <div class="st-row">
    <button class="st-row-open" type="button" onclick={() => openDetail(l)}>
      <span class="st-thumb" style={art(l)}></span>
      <span class="st-row-copy">
        <b>{l.name}</b>
        <span class="st-row-line">{l.tagline}</span>
        <span class="st-row-by">{l.publisher}</span>
      </span>
    </button>
    {#if trailing === 'Install'}
      <button class="btn st-install" type="button">Install</button>
    {:else if trailing === 'Update'}
      <button class="btn st-install" type="button">Update</button>
    {:else}
      <span class="st-installed">Installed</span>
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
