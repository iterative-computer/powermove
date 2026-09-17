<script lang="ts">
  import { tick } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import { createChatGPTSettingsControl, createClaudeSettingsControl } from '../legacy/ui/chatgpt-settings';
  import { createCompatibleSettingsControl } from '../legacy/ui/compatible-settings';
  import { createExtensionSettingsControl } from '../legacy/ui/extension-settings';
  import { createProjectSettingsControl, type ProjectSettingsBridge } from '../legacy/ui/project-settings';
  import { mountSquircles } from './squircle';
  import { clearSettingsSearch, searchSettings } from './search';

  export type SettingsPage = 'general' | 'accounts' | 'extensions' | 'project';

  let { PM, projectBridge }: {
    PM: Record<string, any>;
    /** The live project's composition/export bridge, or null with no project open. */
    projectBridge: () => ProjectSettingsBridge | null;
  } = $props();

  type NavItem = { id: SettingsPage; label: string; icon: string };
  type NavGroup = { title: string; items: NavItem[] };

  const NAV: NavGroup[] = [
    {
      title: 'Project',
      items: [
        { id: 'project', label: 'Project', icon: 'frame' }
      ]
    },
    {
      title: 'App',
      items: [
        { id: 'general', label: 'General', icon: 'gear' },
        { id: 'accounts', label: 'Accounts', icon: 'link' },
        { id: 'extensions', label: 'Extensions', icon: 'puzzle' }
      ]
    }
  ];

  const APPEARANCE: Array<[value: string, label: string]> = [
    ['system', 'Match system'],
    ['light', 'Light'],
    ['dark', 'Dark']
  ];

  type Controls = {
    chatgpt: ReturnType<typeof createChatGPTSettingsControl>;
    claude: ReturnType<typeof createClaudeSettingsControl>;
    compatible: ReturnType<typeof createCompatibleSettingsControl>;
    extensions: ReturnType<typeof createExtensionSettingsControl>;
    project: ReturnType<typeof createProjectSettingsControl> | null;
    offProject: (() => void) | null;
  };

  let shown = $state(false);
  let page = $state<SettingsPage>('general');
  let searchText = $state('');
  const query = $derived(searchText.trim().toLowerCase());
  let themeMode = $state<string>('system');
  let controls = $state.raw<Controls | null>(null);
  let rootEl = $state<HTMLElement | null>(null);
  let scrollEl = $state<HTMLElement | null>(null);
  let columnEl = $state<HTMLElement | null>(null);
  let matchedPages = $state<string[]>([]);
  let matchCount = $state(0);
  let lastFocus: HTMLElement | null = null;

  const hasProject = $derived(!!controls?.project);
  const groups = $derived(NAV
    .map((group) => ({
      ...group,
      items: group.items.filter((item) =>
        (item.id !== 'project' || hasProject)
        && (!query || matchedPages.includes(item.id)))
    }))
    .filter((group) => group.items.length > 0));
  const visibleItems = $derived(groups.flatMap((group) => group.items));
  const pages = $derived(NAV.flatMap((group) => group.items).filter((item) => item.id !== 'project' || hasProject));

  function buildProject(): void {
    if (!controls) return;
    controls.offProject?.();
    controls.project?.destroy();
    const bridge = projectBridge();
    const project = bridge ? createProjectSettingsControl(bridge) : null;
    controls = {
      ...controls,
      project,
      offProject: project ? PM.bus?.on?.('project', () => project.refresh()) ?? null : null
    };
  }

  function build(): void {
    if (controls) return;
    controls = {
      chatgpt: createChatGPTSettingsControl(),
      claude: createClaudeSettingsControl(),
      compatible: createCompatibleSettingsControl(),
      extensions: createExtensionSettingsControl(),
      project: null,
      offProject: null
    };
    buildProject();
  }

  function teardown(): void {
    if (!controls) return;
    controls.offProject?.();
    controls.chatgpt.destroy();
    controls.claude.destroy();
    controls.compatible.destroy();
    controls.extensions.destroy();
    controls.project?.destroy();
    controls = null;
  }

  export function open(target?: SettingsPage): void {
    build();
    themeMode = PM.theme?.mode ?? 'system';
    const wanted = target ?? (controls?.project ? 'project' : 'general');
    const destination = wanted === 'project' && !controls?.project ? 'general' : wanted;
    page = destination;
    searchText = '';
    void tick().then(() => show(destination, 'instant'));
    if (shown) {
      PM.bus?.emit?.('settings:screen');
      return;
    }
    lastFocus = document.activeElement as HTMLElement | null;
    shown = true;
    PM.bus?.emit?.('settings:screen');
    /* Focus the screen itself, not a field: Tab then reaches the search first,
       and nothing opens already lit with a focus edge. */
    void tick().then(() => rootEl?.focus({ preventScroll: true }));
  }

  export function close(): void {
    if (!shown) return;
    shown = false;
    searchText = '';
    teardown();
    PM.bus?.emit?.('settings:screen');
    const focus = lastFocus;
    lastFocus = null;
    if (focus?.isConnected && !focus.closest('[inert]')) focus.focus();
  }

  export function isOpen(): boolean {
    return shown;
  }

  function show(id: SettingsPage, behavior: ScrollBehavior = 'smooth'): void {
    page = id;
    const section = columnEl?.querySelector<HTMLElement>(`[data-settings-page="${id}"]`);
    if (!scrollEl || !section) return;
    const top = section.getBoundingClientRect().top - scrollEl.getBoundingClientRect().top
      + scrollEl.scrollTop - parseFloat(getComputedStyle(scrollEl).paddingTop);
    scrollEl.scrollTo({ top, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : behavior });
  }

  function syncSection(): void {
    if (!scrollEl || !columnEl) return;
    const sections = Array.from(columnEl.querySelectorAll<HTMLElement>('.sg-page'))
      .filter(el => !el.hasAttribute('data-settings-search-hidden'));
    if (!sections.length) return;
    const top = scrollEl.getBoundingClientRect().top + parseFloat(getComputedStyle(scrollEl).paddingTop) + 2;
    let active = sections[0]!;
    for (const section of sections) {
      if (section.getBoundingClientRect().top <= top) active = section;
    }
    if (scrollEl.scrollTop > 0 && scrollEl.scrollTop + scrollEl.clientHeight >= scrollEl.scrollHeight - 2) active = sections.at(-1)!;
    page = active.dataset.settingsPage as SettingsPage;
  }

  /* Account status and extension names can arrive after the screen opens. */
  $effect(() => {
    if (!shown || !columnEl) return;
    const column = columnEl;
    const term = query;
    const update = () => {
      const result = searchSettings(column, term);
      matchedPages = result.pages;
      matchCount = result.count;
      syncSection();
    };
    update();
    scrollEl?.scrollTo({ top: 0, behavior: 'instant' });
    let frame = 0;
    const observer = new MutationObserver(records => {
      if (!records.some(record => !(record.target instanceof Element ? record.target : record.target.parentElement)
        ?.closest('svg, [data-slot="smooth-corners-effects"]'))) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(update);
    });
    observer.observe(column, { childList: true, characterData: true, subtree: true, attributes: true, attributeFilter: ['hidden'] });
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      clearSettingsSearch(column);
    };
  });

  /* Rebuild the Project page when the open project changes underneath it, or
     when the home screen comes and goes. */
  $effect(() => {
    if (!shown) return;
    const rebuild = () => {
      buildProject();
      if (page === 'project' && !controls?.project) page = 'general';
    };
    const offs = ['project', 'projects:screen'].map((event) => PM.bus?.on?.(event, rebuild));
    return () => offs.forEach((off) => off?.());
  });

  /* Lisse squircles on every card and control while the screen is up. The
     stripped borders are painted from the tokens at attach time, so a theme
     switch re-mounts them to pick up the new ink. */
  $effect(() => {
    if (!shown || !rootEl) return;
    const root = rootEl;
    let unmount = mountSquircles(root);
    const observer = new MutationObserver(() => {
      unmount();
      unmount = mountSquircles(root);
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => {
      observer.disconnect();
      unmount();
    };
  });

  /* Hosts imperative control elements as the node's children, so the legacy
     account, provider, extension and project controls keep their own DOM. */
  type Hosted = Array<HTMLElement | null | undefined> | HTMLElement | null | undefined;
  function host(node: HTMLElement, hosted: Hosted) {
    const place = (next: Hosted) => {
      const elements = (Array.isArray(next) ? next : [next]).filter((el): el is HTMLElement => !!el);
      node.replaceChildren(...elements);
    };
    place(hosted);
    return { update: place };
  }

  function applyAppearance(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    themeMode = value;
    PM.theme?.apply?.(value);
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape' || ((event.metaKey || event.ctrlKey) && event.key === ',' && !event.shiftKey && !event.altKey)) {
      event.preventDefault();
      close();
    }
  }

  function navKeydown(event: KeyboardEvent, id: SettingsPage): void {
    const keys = ['ArrowUp', 'ArrowDown', 'Home', 'End'];
    if (!keys.includes(event.key) || visibleItems.length === 0) return;
    event.preventDefault();
    const index = visibleItems.findIndex((item) => item.id === id);
    const next = event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? visibleItems.length - 1
        : (index + (event.key === 'ArrowDown' ? 1 : -1) + visibleItems.length) % visibleItems.length;
    const target = visibleItems[next]!;
    show(target.id);
    rootEl?.querySelector<HTMLElement>(`[data-settings-tab="${target.id}"]`)?.focus();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  id="settings-screen"
  class:on={shown}
  role="dialog"
  aria-label="Settings"
  tabindex="-1"
  bind:this={rootEl}
  onkeydown={keydown}
>
  <aside class="sg-sidebar">
    <label class="sg-search">
      <Icon {PM} name="search" />
      <input
        type="search"
        placeholder="Search settings"
        aria-label="Search settings"
        bind:value={searchText}
      />
    </label>
    <nav class="sg-nav" aria-label="Settings sections">
      {#each groups as group (group.title)}
        <div class="sg-nav-group">
          <span class="sg-nav-title">{group.title}</span>
          {#each group.items as item (item.id)}
            <button
              class="sg-navbtn"
              class:on={item.id === page}
              type="button"
              id={`settings-tab-${item.id}`}
              aria-current={item.id === page ? 'location' : undefined}
              aria-controls={`settings-page-${item.id}`}
              data-settings-tab={item.id}
              onclick={() => show(item.id)}
              onkeydown={(event) => navKeydown(event, item.id)}
            >
              <Icon {PM} name={item.icon} />
              <span>{item.label}</span>
            </button>
          {/each}
        </div>
      {:else}
        <p class="sg-nav-empty">No settings found.</p>
      {/each}
    </nav>
  </aside>

  <main class="sg-main">
    <div class="sg-actions">
      <button class="sg-navbtn sg-done" type="button" aria-label="Done" onclick={close}>
        <Icon {PM} name="chev" />
        <span>Done</span>
      </button>
    </div>
    <div class="sg-scroll" bind:this={scrollEl} onscroll={syncSection}>
      {#if query}
        <p class="sg-search-results" role="status">
          {#if matchCount}{matchCount} {matchCount === 1 ? 'setting' : 'settings'} matching “{searchText.trim()}”
          {:else}No settings match “{searchText.trim()}”. Try another search.{/if}
        </p>
      {/if}
      <div class="sg-column" bind:this={columnEl}>
        {#each pages as item (item.id)}
          <div
            class="sg-page"
            id={`settings-page-${item.id}`}
            role="region"
            aria-label={item.label}
            data-settings-page={item.id}
          >
            {#if item.id === 'general'}
              <header class="sg-heading">
                <h2>General</h2>
                <p>How Powermove looks on this Mac.</p>
              </header>
              <section class="sg-section">
                <h3 class="sg-section-title">Appearance</h3>
                <div class="sg-group">
                  <div class="settings-row">
                    <div class="settings-copy">
                      <b>Appearance</b>
                      <span>Follow your system setting, or pick a light or dark theme.</span>
                    </div>
                    <select class="settings-select settings-appearance" aria-label="Appearance" value={themeMode} onchange={applyAppearance}>
                      {#each APPEARANCE as [value, label] (value)}
                        <option {value}>{label}</option>
                      {/each}
                    </select>
                  </div>
                </div>
              </section>
            {:else if item.id === 'accounts'}
              <header class="sg-heading">
                <h2>Accounts</h2>
                <p>Sign in with a subscription, or connect an API or local model.</p>
              </header>
              <section class="sg-section">
                <h3 class="sg-section-title">Subscriptions</h3>
                <div class="sg-group" use:host={[controls?.chatgpt.element, controls?.claude.element]}></div>
              </section>
              <section class="sg-section">
                <h3 class="sg-section-title">API or local model</h3>
                <div class="sg-group" use:host={[controls?.compatible.element]}></div>
              </section>
            {:else if item.id === 'extensions'}
              <header class="sg-heading">
                <h2>Extensions</h2>
                <p>Manage extensions and turn them on or off.</p>
              </header>
              <div class="sg-sections" use:host={controls?.extensions.element}></div>
            {:else if item.id === 'project'}
              <header class="sg-heading">
                <h2>Project</h2>
                <p>Composition size and timing, and the export settings this project starts from.</p>
              </header>
              <div class="sg-sections" use:host={controls?.project?.element}></div>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </main>
</div>
