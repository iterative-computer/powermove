<script lang="ts">
  import Icon from '../panels/Icon.svelte';
  import ToolbarMount from './ToolbarMount.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let refreshToken = $state(0);
  let metadataToken = $state(0);
  let renaming = $state(false);
  let renameValue = $state('');

  // One project per window, so the strip names this window's document instead
  // of listing every open one. Other windows are other windows.
  const activeProjectId = $derived.by(() => {
    refreshToken;
    return PM.proj?.id as string | undefined;
  });
  const projectName = $derived.by(() => {
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
  const file = $derived.by(() => {
    refreshToken;
    const state = activeProjectId ? PM.projectFileState?.(activeProjectId, { initialize: false }) : null;
    return state ? { path: state.path, dirty: state.dirty } : null;
  });
  const dirty = $derived(file?.dirty ?? appDirty);
  const multiWindow = $derived(!!PM.windows?.supported);

  function showProjects(): void {
    PM.ProjectsScreen?.show?.();
  }

  function beginRename(): void {
    if (renaming || !activeProjectId) return;
    renaming = true;
    renameValue = projectName;
  }

  function finishRename(commit: boolean): void {
    if (!renaming) return;
    const id = activeProjectId;
    if (commit && id) {
      try { PM.Projects?.rename?.(id, renameValue); }
      catch (error) { PM.toast?.(`Could not rename project: ${error instanceof Error ? error.message : String(error)}`); return; }
    }
    renaming = false;
    PM.bus?.emit?.('projects:open');
    if (commit) PM.bus?.emit?.('project');
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

  function documentMenu(event: MouseEvent): void {
    event.preventDefault();
    event.stopPropagation();
    const items: Array<Record<string, unknown> | string> = [
      { label: 'Rename project…', kb: 'F2', run: () => beginRename() }
    ];
    if (multiWindow) {
      items.push('-', { label: 'New Window', kb: '⇧⌘N', run: () => void PM.newWindow?.() });
    }
    PM.menu(event.currentTarget, items, { x: event.clientX, y: event.clientY });
  }

  function documentClick(event: MouseEvent): void {
    if (renaming || event.target instanceof HTMLInputElement) return;
    if (event.detail > 1) {
      event.preventDefault();
      beginRename();
      return;
    }
    // The document button is the way back out of Projects to the composition.
    if (PM.ProjectsScreen?.isOpen) PM.ProjectsScreen.hide?.();
  }

  function documentKeydown(event: KeyboardEvent): void {
    if (event.key === 'F2') {
      event.preventDefault();
      event.stopPropagation();
      beginRename();
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      (event.currentTarget as HTMLElement).click();
    }
  }

  function startWindowDrag(event: PointerEvent): void {
    if (event.button !== 0) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('button, #doc-strip, #toolbar-strip, input, a, .tb-right')) return;
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

  function zoomWindow(event: MouseEvent): void {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('button, #doc-strip, #toolbar-strip, input, a, .tb-right')) return;
    (window as any).webkit?.messageHandlers?.windowZoom?.postMessage({});
  }

  $effect(() => {
    const events = ['projects:open', 'projects:screen', 'settings:screen', 'project', 'history'];
    const offs = events.map((event) => PM.bus?.on?.(event, () => {
      if (event === 'projects:open' || event === 'project') metadataToken++;
      refreshToken++;
    }));
    return () => offs.forEach((off) => off?.());
  });

  /* Electron carves the tab strip out of the titlebar's native drag region
     using rectangles it derives from layout. Those rectangles can go stale
     (the strip is absolutely centred and changes width with every rename or
     open tab), and a stale rectangle turns a click on a tab into a window
     drag that the page never sees. Flipping the titlebar's app-region for
     one frame forces Chromium to recompute and resend the regions. Do it
     whenever the strip changes shape, the window resizes, or the pointer
     arrives on the titlebar, so the regions are fresh before any click. */
  let dragRegionFrame = 0;
  function refreshDragRegions(): void {
    if (dragRegionFrame) return;
    const titlebar = document.getElementById('titlebar');
    if (!titlebar) return;
    titlebar.style.setProperty('-webkit-app-region', 'no-drag');
    dragRegionFrame = window.requestAnimationFrame(() => {
      dragRegionFrame = 0;
      titlebar.style.removeProperty('-webkit-app-region');
    });
  }

  $effect(() => {
    refreshToken;
    activeProjectId;
    homeOpen;
    refreshDragRegions();
  });

  $effect(() => {
    const titlebar = document.getElementById('titlebar');
    const tabs = document.getElementById('tabs');
    if (!titlebar) return;
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(() => refreshDragRegions()) : null;
    if (tabs) observer?.observe(tabs);
    observer?.observe(titlebar);
    let lastEnter = 0;
    const onEnter = () => { const now = performance.now(); if (now - lastEnter > 250) { lastEnter = now; refreshDragRegions(); } };
    titlebar.addEventListener('pointerenter', onEnter);
    titlebar.addEventListener('pointermove', onEnter);
    window.addEventListener('resize', refreshDragRegions);
    window.addEventListener('focus', refreshDragRegions);
    return () => {
      observer?.disconnect();
      titlebar.removeEventListener('pointerenter', onEnter);
      titlebar.removeEventListener('pointermove', onEnter);
      window.removeEventListener('resize', refreshDragRegions);
      window.removeEventListener('focus', refreshDragRegions);
      if (dragRegionFrame) window.cancelAnimationFrame(dragRegionFrame);
    };
  });

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
<div id="tools-center" aria-hidden="false">
<div class="titlebar-drag" aria-hidden="true"></div>
<ToolbarMount {PM} />
<div class="titlebar-drag" aria-hidden="true"></div>
</div>
<div id="doc-strip" data-svelte-shell="doc-strip">
  <button
    class="project-strip-btn project-home"
    class:on={homeOpen}
    type="button"
    title="Projects"
    aria-label="Projects"
    aria-pressed={homeOpen}
    onclick={showProjects}
  ><Icon {PM} name="home" /></button>

  {#if activeProjectId}
    <div
      class="project-doc"
      class:on={!homeOpen}
      class:dirty
      class:renaming
      title={file?.path ? `${projectName} — ${file.path}` : `${projectName} — Not saved to a file`}
      role="button"
      data-project-id={activeProjectId}
      aria-label={renaming ? `Rename ${projectName}` : `${projectName}${dirty ? ', unsaved' : ''}`}
      tabindex="0"
      onclick={documentClick}
      oncontextmenu={documentMenu}
      ondblclick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        beginRename();
      }}
      onkeydown={documentKeydown}
    >
      {#if renaming}
        <input
          class="project-doc-input"
          bind:value={renameValue}
          maxlength="120"
          aria-label="Rename project"
          spellcheck="false"
          use:focusAndSelect
          onclick={(event) => event.stopPropagation()}
          onpointerdown={(event) => event.stopPropagation()}
          onkeydown={renameKeydown}
          onblur={() => finishRename(true)}
        />
      {:else}
        <span class="project-doc-label">{projectName}</span>
      {/if}
    </div>
  {/if}

</div>

<div class="tb-right" id="tb-right">
  {#if !homeOpen}
    <button class="btn tb-export" type="button" title="Export… (⌘E)" aria-label="Export…" onclick={() => PM.Export?.dialog?.()}>
      <Icon {PM} name="export" />Export
    </button>
    <button class="iconbtn" type="button" title="Panel library" aria-label="Open panel library" onclick={() => PM.LibraryUI?.open?.()}>
      <Icon {PM} name="grid" />
    </button>
  {/if}
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
</div>
