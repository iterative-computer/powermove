<script lang="ts">
  import { tick } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import { createChatGPTSettingsControl, createClaudeSettingsControl } from '../legacy/ui/chatgpt-settings';
  import { createCompatibleSettingsControl } from '../legacy/ui/compatible-settings';
  import { createExtensionSettingsControl } from '../legacy/ui/extension-settings';
  import { createProjectSettingsControl, type ProjectSettingsBridge } from '../legacy/ui/project-settings';
  import { mountSquircles } from './squircle';
  import { mountNavGlide } from '../controls/nav-glide';
  import { clearSettingsSearch, searchSettings } from './search';
  import Avatar from '../cloud/Avatar.svelte';
  import { applyAccount, cloudBridge, openSignIn, signOut, subscribeAccount, type CloudUser } from '../cloud/account';

  export type SettingsPage = 'general' | 'accounts' | 'extensions' | 'advanced' | 'project';

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
        { id: 'extensions', label: 'Extensions', icon: 'puzzle' },
        { id: 'advanced', label: 'Advanced', icon: 'sliders' }
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
  /* Reopening last session's windows is the default; the store only ever holds
     the opt-out, so an untouched profile needs no migration. */
  let restoreWindows = $state(true);
  let autoDownloadCloudMedia = $state(false);
  const multiWindow = $derived(!!PM.windows?.supported);
  let controls = $state.raw<Controls | null>(null);
  let rootEl = $state<HTMLElement | null>(null);
  let scrollEl = $state<HTMLElement | null>(null);
  let columnEl = $state<HTMLElement | null>(null);
  let matchedPages = $state<string[]>([]);
  let matchCount = $state(0);
  let lastFocus: HTMLElement | null = null;
  let account = $state<CloudUser | null>(null);
  let rememberInstalls = $state(true);
  let accountBusy = $state(false);
  /* Settings › Advanced › Registry URL. Shown as a host; main owns the value
     and asks with a native dialog before switching (and signing out). */
  let registryOrigin = $state<string | null>(null);
  let registryFromEnvironment = $state(false);
  let registryDraft = $state('');
  let registryBusy = $state(false);
  let registryError = $state<string | null>(null);
  const registryHost = (origin: string): string => origin.replace(/^https:\/\//, '');
  const registryChanged = $derived(registryOrigin !== null && registryDraft.trim() !== '' && registryDraft.trim() !== registryHost(registryOrigin) && registryDraft.trim() !== registryOrigin);

  $effect(() => subscribeAccount((user, me) => {
    account = user;
    rememberInstalls = me?.settings.rememberInstalls ?? true;
  }));

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

  async function loadRegistry(): Promise<void> {
    const bridge = cloudBridge();
    if (!bridge) return;
    try {
      const current = await bridge.registryUrl();
      registryOrigin = current.origin;
      registryFromEnvironment = current.fromEnvironment ?? false;
      registryDraft = registryHost(current.origin);
      registryError = null;
    } catch {
      registryOrigin = null;
    }
  }

  async function changeRegistry(): Promise<void> {
    const bridge = cloudBridge();
    const value = registryDraft.trim();
    if (!bridge || registryBusy || !value) return;
    const origin = /^[a-z][a-z0-9+.-]*:\/\//i.test(value) ? value : `https://${value}`;
    registryError = null;
    registryBusy = true;
    try {
      const result = await bridge.setRegistryUrl({ origin });
      if (!result.ok) throw new Error(result.error.error);
      registryOrigin = result.value.origin;
      registryFromEnvironment = result.value.fromEnvironment ?? false;
      registryDraft = registryHost(result.value.origin);
      if (result.value.changed) {
        applyAccount(null);
        PM.toast?.(`The Store now uses ${registryHost(result.value.origin)}. Sign in to publish there.`, 4000, { kind: 'status' });
      }
    } catch {
      registryError = 'Enter a web address, like cloud.trypowermove.com.';
    } finally {
      registryBusy = false;
    }
  }

  export function open(target?: SettingsPage): void {
    build();
    void loadRegistry();
    themeMode = PM.theme?.mode ?? 'system';
    restoreWindows = PM.store?.get?.('restoreWindows', true) !== false;
    autoDownloadCloudMedia = PM.store?.get?.('autoDownloadCloudMedia', false) === true;
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

  function glide(node: HTMLElement) {
    const unmount = mountNavGlide(node, { row: '.sg-navbtn', selected: '.on' });
    return { destroy: unmount };
  }

  function applyAppearance(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    themeMode = value;
    PM.theme?.apply?.(value);
  }

  /* Account actions run in main; failures come back as results, never throws. */
  async function toggleRememberInstalls(): Promise<void> {
    const bridge = cloudBridge();
    if (!bridge || accountBusy) return;
    const next = !rememberInstalls;
    rememberInstalls = next;
    accountBusy = true;
    try {
      const result = await bridge.setRememberInstalls({ value: next });
      if (result.ok) applyAccount(result.value);
      else throw new Error(result.error.error);
    } catch {
      rememberInstalls = !next;
      PM.toast?.('Unable to change this setting. Check your connection and try again.', 4000, { error: true });
    } finally {
      accountBusy = false;
    }
  }

  async function deleteAccount(): Promise<void> {
    const bridge = cloudBridge();
    if (!bridge || accountBusy) return;
    accountBusy = true;
    try {
      const result = await bridge.deleteAccount();
      if (!result.ok) throw new Error(result.error.error);
      if (result.value.deleted) {
        applyAccount(null);
        PM.toast?.('Your account was deleted.', 3000, { kind: 'status' });
      }
    } catch {
      PM.toast?.('Unable to delete your account. Check your connection and try again.', 5000, { error: true });
    } finally {
      accountBusy = false;
    }
  }

  function toggleRestoreWindows(): void {
    restoreWindows = !restoreWindows;
    PM.store?.set?.('restoreWindows', restoreWindows);
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
    <nav class="sg-nav" aria-label="Settings sections" use:glide>
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
    <button class="sg-navbtn sg-done" type="button" aria-label="Done" onclick={close}>
      <Icon {PM} name="chev" />
      <span>Done</span>
    </button>
  </aside>

  <main class="sg-main">
    <div class="sg-actions">
      <button class="btn" type="button" onclick={close}>Done</button>
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
              <section class="sg-section">
                <h3 class="sg-section-title">Media</h3>
                <div class="sg-group">
                  <div class="settings-row">
                    <div class="settings-copy">
                      <b>Automatically download cloud media</b>
                      <span>Download offloaded files when opening a project.</span>
                    </div>
                    <button class="toggle" class:on={autoDownloadCloudMedia} type="button" aria-pressed={autoDownloadCloudMedia} aria-label="Automatically download cloud media"
                      onclick={() => { autoDownloadCloudMedia = !autoDownloadCloudMedia; PM.store?.set?.('autoDownloadCloudMedia', autoDownloadCloudMedia); }}
                    ><i aria-hidden="true"></i></button>
                  </div>
                </div>
              </section>
              {#if multiWindow}
                <section class="sg-section">
                  <h3 class="sg-section-title">Windows</h3>
                  <div class="sg-group">
                    <div class="settings-row">
                      <div class="settings-copy">
                        <b>Reopen windows on launch</b>
                        <span>Start Powermove with the projects that had a window when you quit. Off, it opens one window.</span>
                      </div>
                      <button
                        class="toggle"
                        class:on={restoreWindows}
                        type="button"
                        aria-pressed={restoreWindows}
                        aria-label="Reopen windows on launch"
                        onclick={toggleRestoreWindows}
                      ><i aria-hidden="true"></i></button>
                    </div>
                  </div>
                </section>
              {/if}
            {:else if item.id === 'accounts'}
              <header class="sg-heading">
                <h2>Accounts</h2>
                <p>Your Powermove account, and the models your agent works with.</p>
              </header>
              <section class="sg-section">
                <h3 class="sg-section-title">Powermove</h3>
                <div class="sg-group">
                  {#if account}
                    <div class="settings-row acct-profile">
                      <Avatar user={account} size={32} />
                      <div class="settings-copy">
                        <b>{account.handle ? `@${account.handle}` : account.email}</b>
                        {#if account.handle}<span>{account.email}</span>{/if}
                      </div>
                      {#if !account.handle}
                        <button class="btn" type="button" onclick={() => openSignIn()}>Choose Handle…</button>
                      {/if}
                      <button class="btn" type="button" onclick={() => void signOut()}>Sign Out</button>
                    </div>
                    <div class="settings-row">
                      <div class="settings-copy">
                        <b>Remember installs on this account</b>
                        <span>Keep a list of the extensions you install from the Store. Turning this off deletes the list.</span>
                      </div>
                      <button
                        class="toggle"
                        class:on={rememberInstalls}
                        type="button"
                        aria-pressed={rememberInstalls}
                        aria-label="Remember installs on this account"
                        disabled={accountBusy}
                        onclick={() => void toggleRememberInstalls()}
                      ><i aria-hidden="true"></i></button>
                    </div>
                    <div class="settings-row">
                      <div class="settings-copy">
                        <b>Delete account</b>
                        <span>Your published extensions stay on the Store under your handle.</span>
                      </div>
                      <button class="btn acct-delete" type="button" disabled={accountBusy} onclick={() => void deleteAccount()}>Delete Account…</button>
                    </div>
                  {:else}
                    <div class="settings-row">
                      <div class="settings-copy">
                        <b>Powermove account</b>
                        <span>Publish and manage your extensions on the Store.</span>
                      </div>
                      <button class="btn" type="button" onclick={() => openSignIn()}>Sign In…</button>
                    </div>
                  {/if}
                </div>
              </section>
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
            {:else if item.id === 'advanced'}
              <header class="sg-heading">
                <h2>Advanced</h2>
                <p>Settings most people never need to change.</p>
              </header>
              <section class="sg-section">
                <h3 class="sg-section-title">Store</h3>
                <div class="sg-group">
                  <div class="settings-row">
                    <label class="settings-copy" for="settings-registry-url">
                      <b>Registry URL</b>
                      {#if registryFromEnvironment}<span>(from environment)</span>{/if}
                      {#if registryError}
                        <span class="settings-extension-error" role="alert">{registryError}</span>
                      {:else}
                        <span>Where the Store finds and publishes extensions. Changing it signs you out.</span>
                      {/if}
                    </label>
                    <input
                      id="settings-registry-url"
                      class="settings-input is-wide"
                      type="text"
                      inputmode="url"
                      autocomplete="off"
                      autocapitalize="off"
                      spellcheck="false"
                      maxlength="2000"
                      placeholder="cloud.trypowermove.com"
                      disabled={registryBusy || registryOrigin === null || registryFromEnvironment}
                      bind:value={registryDraft}
                      onkeydown={(event) => { if (event.key === 'Enter' && registryChanged) { event.preventDefault(); void changeRegistry(); } }}
                    />
                    <button class="btn" type="button" disabled={registryBusy || !registryChanged || registryFromEnvironment} onclick={() => void changeRegistry()}>Change…</button>
                  </div>
                </div>
              </section>
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
