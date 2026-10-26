<script lang="ts">
  import Icon from '../panels/Icon.svelte';
  import { readProjectLoading } from '../overlays/project-ghost';
  import AgentLauncher from './AgentLauncher.svelte';
  import TaskPill from './TaskPill.svelte';
  import ToolbarMount from './ToolbarMount.svelte';

  /* After Effects' layout: Home and the tools on the left, the document's
     name centred the way a macOS window titles itself, and what you do with
     the project on the right. Other open projects are one click away in the
     title's menu instead of a strip of tabs. */
  let { PM }: { PM: Record<string, any> } = $props();
  let refreshToken = $state(0);
  let metadataToken = $state(0);
  let renaming = $state(false);
  let renameValue = $state('');
  let titleButton = $state<HTMLButtonElement>();

  const openIds = $derived.by(() => {
    metadataToken;
    return [...(PM.Tabs?.list?.() ?? (PM.proj?.id && !PM.isHomeProject?.() ? [PM.proj.id] : []))] as string[];
  });
  const names = $derived.by(() => {
    metadataToken;
    return new Map<string, string>((PM.Projects?.list?.() ?? []).map((meta: { id: string; name?: string }) => [meta.id, meta.name || 'Untitled']));
  });
  /* A project that is still opening is titled at once; the loaded document
     (name, dirty state) stays with its own title until it lands. */
  const pending = $derived.by(() => {
    refreshToken;
    return readProjectLoading(PM);
  });
  const documentId = $derived.by(() => {
    refreshToken;
    return PM.proj?.id as string | undefined;
  });
  const activeProjectId = $derived(pending ? (pending.id ?? undefined) : documentId);
  const activeName = $derived.by(() => {
    metadataToken;
    refreshToken;
    return (PM.proj?.name as string | undefined) || 'Untitled';
  });
  const homeOpen = $derived.by(() => {
    refreshToken;
    return !!PM.ProjectsScreen?.isOpen;
  });
  const settingsOpen = $derived.by(() => {
    refreshToken;
    return !!PM.SettingsUI?.isOpen;
  });
  const appDirty = $derived.by(() => {
    refreshToken;
    return !!PM.app?.dirty;
  });
  // File metadata is read lazily, so the lookup — not its result — is what
  // re-derives when a save, rename or dirty change bumps the token.
  const fileStateFor = $derived.by(() => {
    refreshToken;
    return (id: string): { path?: string; dirty?: boolean } | null => {
      const state = PM.projectFileState?.(id, { initialize: false });
      return state ? { path: state.path, dirty: state.dirty } : null;
    };
  });
  const multiWindow = $derived(!!PM.windows?.supported);
  /* The project on screen, or the one Projects will return to. Nothing on a
     cold boot until the first document lands. */
  const titleId = $derived(activeProjectId && (openIds.includes(activeProjectId) || pending?.id === activeProjectId) ? activeProjectId : null);
  const opening = $derived(!!pending && (!pending.id || pending.id === titleId));
  const titleName = $derived(titleId ? nameFor(titleId) : pending ? (pending.name || 'Untitled') : '');
  const titleFile = $derived(titleId && !opening ? fileStateFor(titleId) : null);
  const titleDirty = $derived(!!titleId && !opening && (titleId === documentId ? (titleFile?.dirty ?? appDirty) : !!titleFile?.dirty));

  function nameFor(id: string): string {
    if (pending?.id === id && pending.name) return pending.name;
    return id === documentId ? activeName : names.get(id) || 'Untitled';
  }

  function showProjects(): void {
    PM.ProjectsScreen?.show?.();
  }

  function activate(id: string): void {
    if (id === activeProjectId) {
      // The title is also the way back out of Projects to the composition.
      if (PM.ProjectsScreen?.isOpen) PM.ProjectsScreen.hide?.();
      return;
    }
    void PM.Tabs?.activate?.(id);
  }

  function close(id: string): void {
    void PM.Tabs?.close?.(id);
  }

  function beginRename(): void {
    if (renaming || !titleId || opening) return;
    renaming = true;
    renameValue = nameFor(titleId);
  }

  function finishRename(commit: boolean): void {
    if (!renaming || !titleId) return;
    if (commit) {
      try { PM.Projects?.rename?.(titleId, renameValue); }
      catch (error) { PM.toast?.(`Could not rename project: ${error instanceof Error ? error.message : String(error)}`); return; }
    }
    renaming = false;
    PM.bus?.emit?.('projects:open');
    if (commit) PM.bus?.emit?.('project');
    titleButton?.focus();
  }

  function renameKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Enter') {
      event.preventDefault();
      finishRename(true);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      finishRename(false);
    }
  }

  function focusAndSelect(node: HTMLInputElement): { destroy(): void } {
    const frame = window.requestAnimationFrame(() => {
      node.focus();
      node.select();
    });
    return { destroy: () => window.cancelAnimationFrame(frame) };
  }

  /* Open projects, then what you can do with the one on screen. */
  function projectMenu(event?: MouseEvent): void {
    event?.preventDefault();
    event?.stopPropagation();
    if (!titleButton || !titleId) return;
    const items: Array<Record<string, unknown> | string> = [
      ...openIds.map((id) => ({ label: nameFor(id), on: id === titleId && !homeOpen, run: () => activate(id) })),
      '-',
      { label: 'Rename Project…', kb: 'F2', disabled: opening, run: beginRename },
      { label: 'Close Project', kb: '⌘W', run: () => close(titleId) },
      { label: 'Close Other Projects', disabled: openIds.length < 2, run: () => void PM.Tabs?.closeOthers?.(titleId) }
    ];
    if (multiWindow) items.push({ label: 'Move to New Window', disabled: openIds.length < 2, run: () => void PM.Tabs?.moveToNewWindow?.(titleId) });
    items.push(
      '-',
      { label: 'New Project', kb: '⌥⌘N', run: () => PM.newProject?.() },
      { label: 'All Projects…', kb: '⌘P', run: showProjects }
    );
    if (event?.type === 'contextmenu') PM.menu(titleButton, items, { x: event.clientX, y: event.clientY });
    else PM.menu(titleButton, items);
  }

  function titleKeydown(event: KeyboardEvent): void {
    if (event.key === 'F2') {
      event.preventDefault();
      event.stopPropagation();
      beginRename();
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      projectMenu();
    }
  }

  /* ── native window behaviour ────────────────────────────── */
  function startWindowDrag(event: PointerEvent): void {
    if (event.button !== 0) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('button, #toolbar-strip, input, a, .tb-right')) return;
    const bridge = (window as any).webkit?.messageHandlers?.windowDrag;
    if (!bridge) return;
    event.preventDefault();
    document.body.style.cursor = 'default';
    (document.activeElement as HTMLElement | null)?.blur?.();
    const pin = () => { document.body.style.cursor = 'default'; };
    let released = false;
    const release = () => {
      if (released) return;
      released = true;
      window.removeEventListener('pointermove', pin, true);
      window.removeEventListener('pointerup', release, true);
      window.removeEventListener('pointercancel', release, true);
      document.body.style.cursor = '';
    };
    window.addEventListener('pointermove', pin, true);
    window.addEventListener('pointerup', release, true);
    window.addEventListener('pointercancel', release, true);
    bridge.postMessage({ x: event.clientX, y: event.clientY });
  }

  /* After Effects' workspace bar: the main workspaces one click away, the
     rest (and the panel library) in the overflow menu. */
  const BAR_WORKSPACES = ['design', 'animate', 'effects'];
  const workspaces = $derived.by(() => {
    refreshToken;
    const all = (PM.WS?.list?.() ?? []) as Array<{ id: string; name: string }>;
    return { bar: BAR_WORKSPACES.map((id) => all.find((item) => item.id === id)).filter((item) => !!item), all, current: PM.WS?.current?.id as string | undefined };
  });
  function moreWorkspaces(event: MouseEvent): void {
    const anchor = event.currentTarget as HTMLElement;
    PM.menu(anchor, [
      { header: 'Workspaces' },
      ...workspaces.all.map((item) => ({ label: item.name, on: item.id === workspaces.current, run: () => PM.WS.activate(item.id) })),
      '-',
      { label: 'Reset “' + (workspaces.all.find((item) => item.id === workspaces.current)?.name ?? 'Workspace') + '” to Saved Layout', disabled: !PM.WS?.get?.(workspaces.current)?.builtin, run: () => PM.WS.resetBuiltin(workspaces.current) },
      '-',
      { label: 'Panel Library…', icon: 'grid', run: () => PM.LibraryUI?.open?.() },
    ]);
  }

  function zoomWindow(event: MouseEvent): void {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('button, #toolbar-strip, input, a, .tb-right')) return;
    (window as any).webkit?.messageHandlers?.windowZoom?.postMessage({});
  }

  $effect(() => {
    const events = ['projects:open', 'projects:screen', 'settings:screen', 'project', 'project:loading', 'history', 'workspaces'];
    const offs = events.map((event) => PM.bus?.on?.(event, () => {
      if (event === 'projects:open' || event === 'project' || event === 'project:loading') metadataToken++;
      refreshToken++;
    }));
    return () => offs.forEach((off) => off?.());
  });

  // Keep Electron's CSS drag region stable. Interactive controls explicitly
  // opt out; toggling the entire titlebar on pointer entry can swallow drags.
  $effect(() => {
    const titlebar = document.getElementById('titlebar');
    titlebar?.addEventListener('pointerdown', startWindowDrag as EventListener);
    titlebar?.addEventListener('dblclick', zoomWindow as EventListener);
    return () => {
      titlebar?.removeEventListener('pointerdown', startWindowDrag as EventListener);
      titlebar?.removeEventListener('dblclick', zoomWindow as EventListener);
    };
  });
</script>

<div class="titlebar-drag titlebar-drag-traffic" aria-hidden="true"></div>
<div class="tb-left" id="tb-left">
  <button
    class="project-strip-btn project-home"
    class:on={homeOpen}
    type="button"
    title="Projects (⌘P)"
    aria-label="Projects"
    aria-pressed={homeOpen}
    onclick={showProjects}
  ><Icon {PM} name="home" /></button>
  <div class="tb-tools" inert={!!pending}>
    <ToolbarMount {PM} />
  </div>
</div>

<div class="titlebar-drag" aria-hidden="true"></div>

<div id="doc-title" class="tb-title" data-svelte-shell="title">
  {#if renaming}
    <input
      class="tb-title-input"
      bind:value={renameValue}
      maxlength="120"
      aria-label="Rename project"
      spellcheck="false"
      use:focusAndSelect
      onkeydown={renameKeydown}
      onblur={() => finishRename(true)}
    />
  {:else if titleName}
    <button
      bind:this={titleButton}
      class="tb-title-btn"
      class:pending={opening}
      class:dirty={titleDirty}
      class:away={homeOpen}
      type="button"
      data-project-id={titleId}
      aria-haspopup="menu"
      aria-busy={opening || undefined}
      aria-label={`${titleName}${titleDirty ? ', unsaved' : ''}${opening ? ', opening' : ''}`}
      title={opening ? `Opening ${titleName}` : titleFile?.path ? `${titleName} — ${titleFile.path}` : `${titleName} — Not saved to a file`}
      onclick={(event) => (homeOpen && titleId ? activate(titleId) : projectMenu(event))}
      oncontextmenu={projectMenu}
      onkeydown={titleKeydown}
    >
      <span class="tb-title-name">{titleName}</span>
      {#if titleDirty}<span class="tb-title-state">Edited</span>{/if}
      <svg class="tb-title-chev" width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><path d="m2 3 2 2 2-2" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/></svg>
    </button>
  {/if}
</div>

<div class="titlebar-drag" aria-hidden="true"></div>

<div class="tb-right" id="tb-right">
  {#if !homeOpen}
    <div class="tb-workspaces" role="radiogroup" aria-label="Workspace">
      {#each workspaces.bar as item (item.id)}
        <button type="button" role="radio" class:on={item.id === workspaces.current} aria-checked={item.id === workspaces.current}
          title={`${item.name} workspace`} onclick={() => { if (item.id !== workspaces.current) PM.WS.activate(item.id); }}>{item.name}</button>
      {/each}
      <button type="button" class="tb-workspaces-more" title="More workspaces" aria-label="More workspaces" onclick={moreWorkspaces}>
        <Icon {PM} name="more" />
      </button>
    </div>
  {/if}
  <div class="tb-project-controls" inert={!!pending}>
    <TaskPill {PM} />
    <AgentLauncher {PM} />
  </div>
  <button
    class="iconbtn"
    class:on={settingsOpen}
    type="button"
    title="Settings (⌘,)"
    aria-label="Open settings"
    aria-pressed={settingsOpen}
    onclick={() => (settingsOpen ? PM.SettingsUI?.close?.() : PM.SettingsUI?.open?.())}
  >
    <Icon {PM} name="gear" />
  </button>
  {#if !homeOpen}
    <button class="btn pri tb-export" type="button" disabled={!!pending} title="Export… (⌘E)" aria-label="Export…" onclick={() => PM.Export?.dialog?.()}>
      <Icon {PM} name="export" />Export
    </button>
  {/if}
</div>
