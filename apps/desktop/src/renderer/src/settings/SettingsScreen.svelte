<script lang="ts">
  import { tick } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import { createChatGPTSettingsControl, createClaudeSettingsControl } from '../legacy/ui/chatgpt-settings';
  import { createCompatibleSettingsControl } from '../legacy/ui/compatible-settings';
  import { createExtensionSettingsControl } from '../legacy/ui/extension-settings';
  import { createProjectSettingsControl, type ProjectSettingsBridge } from '../legacy/ui/project-settings';
  import { mountSquircles } from './squircle';

  export type SettingsPage = 'general' | 'accounts' | 'extensions' | 'project';

  let { PM, projectBridge }: {
    PM: Record<string, any>;
    /** The live project's composition/export bridge, or null with no project open. */
    projectBridge: () => ProjectSettingsBridge | null;
  } = $props();

  type NavItem = { id: SettingsPage; label: string; icon: string; keywords: string };
  type NavGroup = { title: string; items: NavItem[] };

  const NAV: NavGroup[] = [
    {
      title: 'App',
      items: [
        { id: 'general', label: 'General', icon: 'gear', keywords: 'general appearance theme light dark system' },
        { id: 'accounts', label: 'Accounts', icon: 'link', keywords: 'accounts chatgpt claude codex subscription api key ollama lm studio openai local model provider connect sign in' },
        { id: 'extensions', label: 'Extensions', icon: 'puzzle', keywords: 'extensions plugins panels commands effects enable disable delete' }
      ]
    },
    {
      title: 'Project',
      items: [
        { id: 'project', label: 'Project', icon: 'frame', keywords: 'project composition name size resolution width height frame rate fps duration background export format quality range audio motion blur transparent' }
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
  let lastFocus: HTMLElement | null = null;

  const hasProject = $derived(!!controls?.project);
  const groups = $derived(NAV
    .map((group) => ({
      ...group,
      items: group.items.filter((item) =>
        (item.id !== 'project' || hasProject)
        && (!query || item.label.toLowerCase().includes(query) || item.keywords.includes(query)))
    }))
    .filter((group) => group.items.length > 0));
  const visibleItems = $derived(groups.flatMap((group) => group.items));
  const pages = $derived(NAV.flatMap((group) => group.items).filter((item) => item.id !== 'project' || hasProject));
  const current = $derived(pages.find((item) => item.id === page) ?? pages[0]!);

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
    /* Every entry point lands on the page it asked for; a plain open() starts
       at General, the way the old dialog did. */
    const wanted = target ?? 'general';
    page = wanted === 'project' && !controls?.project ? 'general' : wanted;
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

  function show(id: SettingsPage): void {
    page = id;
  }

  /* A new page starts at its top; the scroll position belongs to the page. */
  $effect(() => {
    page;
    scrollEl?.scrollTo({ top: 0 });
  });

  /* Rebuild the Project page when the open project changes underneath it. */
  $effect(() => {
    if (!shown) return;
    const off = PM.bus?.on?.('project', () => {
      buildProject();
      if (page === 'project' && !controls?.project) page = 'general';
    });
    return () => off?.();
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
    if (event.key === 'Escape') {
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
    page = target.id;
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
    <div class="sg-nav" role="tablist" aria-label="Settings sections" aria-orientation="vertical">
      {#each groups as group (group.title)}
        <div class="sg-nav-group">
          <span class="sg-nav-title">{group.title}</span>
          {#each group.items as item (item.id)}
            <button
              class="sg-navbtn"
              class:on={item.id === current.id}
              type="button"
              role="tab"
              id={`settings-tab-${item.id}`}
              aria-selected={item.id === current.id}
              aria-controls={`settings-page-${item.id}`}
              tabindex={item.id === current.id ? 0 : -1}
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
    </div>
  </aside>

  <main class="sg-main">
    <div class="sg-top">
      <button class="btn" type="button" onclick={close}>Done</button>
    </div>
    <div class="sg-scroll" bind:this={scrollEl}>
      <div class="sg-column">
        {#each pages as item (item.id)}
          <div
            class="sg-page"
            id={`settings-page-${item.id}`}
            role="tabpanel"
            aria-labelledby={`settings-tab-${item.id}`}
            tabindex="0"
            hidden={item.id !== current.id}
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
                      <span>Follow your system setting, or pick light or dark.</span>
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
