<script lang="ts">
  import { tick } from 'svelte';
  import { flip } from 'svelte/animate';
  import Icon from '../panels/Icon.svelte';
  import ToolbarMount from './ToolbarMount.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let refreshToken = $state(0);
  let metadataToken = $state(0);
  let strip = $state<HTMLDivElement>();
  let overflow = $state({ start: false, end: false });
  let rovingId = $state('');
  let renamingId = $state<string | null>(null);
  let renameValue = $state('');
  /* The tab under the pointer while it is being dragged along the strip, and
     how far it sits from its slot so it stays under the cursor. */
  let dragId = $state<string | null>(null);
  let dragOffset = $state(0);
  /* Pulled out of the strip, the tab leaves its slot and follows the pointer
     as a ghost; let go there, it moves to another window or one of its own. */
  let tear = $state<{ x: number; y: number } | null>(null);

  const tabIds = $derived.by(() => {
    metadataToken;
    return [...(PM.Tabs?.list?.() ?? (PM.proj?.id && !PM.isHomeProject?.() ? [PM.proj.id] : []))] as string[];
  });
  const names = $derived.by(() => {
    metadataToken;
    return new Map<string, string>((PM.Projects?.list?.() ?? []).map((meta: { id: string; name?: string }) => [meta.id, meta.name || 'Untitled']));
  });
  const activeProjectId = $derived.by(() => {
    refreshToken;
    return PM.proj?.id as string | undefined;
  });
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
  // re-derives when a save, rename or dirty change bumps the token. The state
  // is one mutable object per project, so each read is copied: an unchanged
  // reference would otherwise hide a changed flag from the tab.
  const fileStateFor = $derived.by(() => {
    refreshToken;
    return (id: string): { path?: string; dirty?: boolean } | null => {
      const state = PM.projectFileState?.(id, { initialize: false });
      return state ? { path: state.path, dirty: state.dirty } : null;
    };
  });
  const multiWindow = $derived(!!PM.windows?.supported);
  const selectedId = $derived(homeOpen || !activeProjectId || !tabIds.includes(activeProjectId) ? 'home' : activeProjectId);

  function nameFor(id: string): string {
    return id === activeProjectId ? activeName : names.get(id) || 'Untitled';
  }

  function tabIndex(id: string): 0 | -1 {
    return (rovingId || selectedId) === id ? 0 : -1;
  }

  function showProjects(): void {
    rovingId = 'home';
    PM.ProjectsScreen?.show?.();
  }

  function activate(id: string): void {
    rovingId = id;
    if (id === activeProjectId) {
      // The tab is the way back out of Projects to the composition.
      if (PM.ProjectsScreen?.isOpen) PM.ProjectsScreen.hide?.();
      return;
    }
    void PM.Tabs?.activate?.(id);
  }

  async function close(id: string): Promise<void> {
    const focused = document.activeElement?.closest('[data-tab-id]')?.getAttribute('data-tab-id') === id;
    const closed = await PM.Tabs?.close?.(id);
    if (!closed) return;
    if (rovingId === id) rovingId = '';
    if (focused) {
      await tick();
      [...document.querySelectorAll<HTMLElement>('#tabs [role="tab"]')]
        .find((node) => node.dataset.tabId === selectedId)?.focus();
    }
  }

  function beginRename(id: string): void {
    if (renamingId) return;
    renamingId = id;
    renameValue = nameFor(id);
  }

  function finishRename(commit: boolean): void {
    if (!renamingId) return;
    const id = renamingId;
    if (commit) {
      try { PM.Projects?.rename?.(id, renameValue); }
      catch (error) { PM.toast?.(`Could not rename project: ${error instanceof Error ? error.message : String(error)}`); return; }
    }
    renamingId = null;
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

  function tabMenu(event: MouseEvent, id: string): void {
    event.preventDefault();
    event.stopPropagation();
    const items: Array<Record<string, unknown> | string> = [
      { label: 'Close Tab', kb: '⌘W', run: () => void close(id) },
      { label: 'Close Other Tabs', disabled: tabIds.length < 2, run: () => void PM.Tabs?.closeOthers?.(id) },
      '-',
      { label: 'Rename Project…', kb: 'F2', run: () => beginRename(id) }
    ];
    if (multiWindow) {
      items.push(
        { label: 'Move to New Window', disabled: tabIds.length < 2, run: () => void PM.Tabs?.moveToNewWindow?.(id) },
        '-',
        { label: 'New Window', kb: '⇧⌘N', run: () => void PM.newWindow?.() }
      );
    }
    PM.menu(event.currentTarget, items, { x: event.clientX, y: event.clientY });
  }

  function focusTab(event: KeyboardEvent): void {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    const tabs = [...document.querySelectorAll<HTMLElement>('#tabs [role="tab"]')];
    if (!tabs.length) return;
    const current = Math.max(0, tabs.indexOf(event.currentTarget as HTMLElement));
    let next = current;
    if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = tabs.length - 1;
    else if (event.key === 'ArrowLeft') next = (current - 1 + tabs.length) % tabs.length;
    else next = (current + 1) % tabs.length;
    event.preventDefault();
    const target = tabs[next];
    if (!target) return;
    rovingId = target.dataset.tabId ?? '';
    target.focus();
  }

  function tabKeydown(event: KeyboardEvent, id: string): void {
    if (event.key === 'F2') {
      event.preventDefault();
      event.stopPropagation();
      beginRename(id);
      return;
    }
    if (event.key === 'Backspace' || event.key === 'Delete') {
      event.preventDefault();
      void close(id);
      return;
    }
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
      focusTab(event);
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      activate(id);
    }
  }

  /* ── dragging a tab along the strip, or out of it ──────────── */
  let press: { id: string; pointer: number; x: number; y: number; grab: number; grabY: number; node: HTMLElement } | null = null;
  let suppressClick = false;

  function tabPointerDown(event: PointerEvent, id: string): void {
    if (event.button !== 0 || renamingId === id || !strip) return;
    if ((event.target as Element)?.closest?.('.project-doc-close')) return;
    const node = event.currentTarget as HTMLElement;
    const left = strip.getBoundingClientRect().left;
    press = {
      id, pointer: event.pointerId, x: event.clientX, y: event.clientY, node,
      grab: event.clientX - (left + node.offsetLeft - strip.scrollLeft),
      grabY: event.clientY - node.getBoundingClientRect().top
    };
    window.addEventListener('pointermove', tabPointerMove, true);
    window.addEventListener('pointerup', tabPointerUp, true);
    window.addEventListener('pointercancel', tabPointerUp, true);
  }

  /** Far enough from the titlebar that the pointer means "somewhere else":
   *  well below it, above it, or out of the window altogether. */
  function pulledOut(event: PointerEvent): boolean {
    if (!multiWindow) return false;
    const bar = document.getElementById('titlebar')?.getBoundingClientRect();
    if (!bar) return false;
    return event.clientY < bar.top - 8 || event.clientY > bar.bottom + 28
      || event.clientX < 0 || event.clientX > window.innerWidth;
  }

  function tabPointerMove(event: PointerEvent): void {
    if (!press || event.pointerId !== press.pointer || !strip) return;
    if (!dragId) {
      if (Math.hypot(event.clientX - press.x, event.clientY - press.y) < 5) return;
      dragId = press.id;
      press.node.setPointerCapture?.(press.pointer);
    }
    const { node, id, grab, grabY } = press;
    if (pulledOut(event)) {
      tear = { x: event.clientX - grab, y: event.clientY - grabY };
      dragOffset = 0;
      return;
    }
    tear = null;
    // Where the tab's leading edge wants to be, in the strip's content space.
    const want = event.clientX - strip.getBoundingClientRect().left + strip.scrollLeft - grab;
    const centre = want + node.offsetWidth / 2;
    const others = [...strip.querySelectorAll<HTMLElement>('[data-tab-id]')].filter((tab) => tab.dataset.tabId !== id);
    const index = others.filter((tab) => tab.offsetLeft + tab.offsetWidth / 2 < centre).length;
    if (tabIds.indexOf(id) !== index) PM.Tabs?.move?.(id, index);
    dragOffset = want - node.offsetLeft;
    void tick().then(() => { if (dragId === id && press && !tear) dragOffset = want - node.offsetLeft; });
  }

  function tabPointerUp(event: PointerEvent): void {
    if (!press || event.pointerId !== press.pointer) return;
    window.removeEventListener('pointermove', tabPointerMove, true);
    window.removeEventListener('pointerup', tabPointerUp, true);
    window.removeEventListener('pointercancel', tabPointerUp, true);
    const torn = event.type === 'pointerup' && tear && dragId;
    if (dragId) suppressClick = true;
    press = null;
    dragId = null;
    dragOffset = 0;
    tear = null;
    // Screen coordinates, so main can tell which window, if any, it landed on.
    if (torn) void PM.Tabs?.detach?.(torn, { x: event.screenX, y: event.screenY });
  }

  function tabClick(event: MouseEvent, id: string): void {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    if (renamingId === id || event.target instanceof HTMLInputElement) return;
    if (event.detail > 1) {
      event.preventDefault();
      beginRename(id);
      return;
    }
    activate(id);
  }

  /* ── overflow ───────────────────────────────────────────── */
  function measureOverflow(): void {
    if (!strip) return;
    const start = strip.scrollLeft > 1;
    const end = strip.scrollLeft + strip.clientWidth < strip.scrollWidth - 1;
    if (start !== overflow.start || end !== overflow.end) overflow = { start, end };
  }

  function scrollTabs(event: WheelEvent): void {
    if (!strip || event.ctrlKey || Math.abs(event.deltaX) >= Math.abs(event.deltaY)) return;
    if (strip.scrollWidth <= strip.clientWidth) return;
    const scale = event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? strip.clientWidth : 1;
    strip.scrollLeft += event.deltaY * scale;
    event.preventDefault();
  }

  $effect(() => {
    const node = strip;
    if (!node) return;
    node.addEventListener('wheel', scrollTabs, { passive: false });
    node.addEventListener('scroll', measureOverflow, { passive: true });
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measureOverflow);
    observer?.observe(node);
    return () => {
      observer?.disconnect();
      node.removeEventListener('wheel', scrollTabs);
      node.removeEventListener('scroll', measureOverflow);
    };
  });

  $effect(() => {
    tabIds;
    const frame = window.requestAnimationFrame(measureOverflow);
    return () => window.cancelAnimationFrame(frame);
  });

  $effect(() => {
    selectedId;
    if (dragId) return;
    const frame = window.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>('#tabs .project-doc.on')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    });
    return () => window.cancelAnimationFrame(frame);
  });

  /* ── native window behaviour ────────────────────────────── */
  function startWindowDrag(event: PointerEvent): void {
    if (event.button !== 0) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('button, #tabs, #toolbar-strip, input, a, .tb-right')) return;
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
    if (target?.closest('button, #tabs, #toolbar-strip, input, a, .tb-right')) return;
    (window as any).webkit?.messageHandlers?.windowZoom?.postMessage({});
  }

  $effect(() => {
    const events = ['projects:open', 'projects:screen', 'settings:screen', 'project', 'history'];
    const offs = events.map((event) => PM.bus?.on?.(event, () => {
      if (!document.getElementById('tabs')?.contains(document.activeElement)) rovingId = '';
      if (event === 'projects:open' || event === 'project') metadataToken++;
      refreshToken++;
    }));
    return () => offs.forEach((off) => off?.());
  });

  /* Electron carves the tab strip out of the titlebar's native drag region
     using rectangles it derives from layout. Those rectangles can go stale
     (the strip changes width with every rename or open tab), and a stale
     rectangle turns a click on a tab into a window drag that the page never
     sees. Flipping the titlebar's app-region for one frame forces Chromium to
     recompute and resend the regions. Do it whenever the strip changes shape,
     the window resizes, or the pointer arrives on the titlebar, so the regions
     are fresh before any click. */
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
    tabIds;
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
<div id="tabs" data-svelte-shell="tabs">
  <div role="tablist" aria-label="Open projects" class="project-tablist">
    <button
      class="project-strip-btn project-home"
      class:on={homeOpen}
      type="button"
      role="tab"
      data-tab-id="home"
      title="Projects (⌘P)"
      aria-label="Projects"
      aria-selected={homeOpen}
      tabindex={tabIndex('home')}
      onclick={showProjects}
      onfocus={() => (rovingId = 'home')}
      onkeydown={focusTab}
    ><Icon {PM} name="home" /></button>

    <div
      class="project-tab-scroll"
      class:fade-start={overflow.start}
      class:fade-end={overflow.end}
      class:dragging={!!dragId}
      bind:this={strip}
    >
      {#each tabIds as id (id)}
        {@const active = id === activeProjectId && !homeOpen}
        {@const file = fileStateFor(id)}
        {@const dirty = id === activeProjectId ? (file?.dirty ?? appDirty) : !!file?.dirty}
        {@const tabName = nameFor(id)}
        <div
          class="project-doc"
          class:on={active}
          class:current={id === activeProjectId}
          class:dirty
          class:renaming={renamingId === id}
          class:lifted={dragId === id && !tear}
          class:torn={dragId === id && !!tear}
          style:transform={dragId === id && !tear ? `translateX(${dragOffset}px)` : null}
          title={file?.path ? `${tabName} — ${file.path}` : `${tabName} — Not saved to a file`}
          role="tab"
          data-tab-id={id}
          data-project-id={id}
          aria-selected={active}
          aria-label={renamingId === id ? `Rename ${tabName}` : `${tabName}${dirty ? ', unsaved' : ''}`}
          tabindex={tabIndex(id)}
          animate:flip={{ duration: dragId === id ? 0 : 180 }}
          onpointerdown={(event) => tabPointerDown(event, id)}
          onclick={(event) => tabClick(event, id)}
          oncontextmenu={(event) => tabMenu(event, id)}
          ondblclick={(event) => {
            if ((event.target as Element)?.closest?.('.project-doc-close')) return;
            event.preventDefault();
            event.stopPropagation();
            beginRename(id);
          }}
          onfocus={() => (rovingId = id)}
          onkeydown={(event) => tabKeydown(event, id)}
          onauxclick={(event) => {
            if (event.button !== 1) return;
            event.preventDefault();
            void close(id);
          }}
        >
          {#if renamingId === id}
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
            <span class="project-doc-label">{tabName}</span>
            {#if dirty}<span class="project-doc-state">Edited</span>{/if}
            <button
              class="project-doc-close"
              type="button"
              title="Close tab (⌘W)"
              aria-label={`Close ${tabName}`}
              tabindex="-1"
              onclick={(event) => {
                event.stopPropagation();
                void close(id);
              }}
            ><Icon {PM} name="x" /></button>
          {/if}
        </div>
      {/each}
    </div>
  </div>

  {#if tear && dragId}
    <div class="project-doc tab-ghost" style:left="{tear.x}px" style:top="{tear.y}px" aria-hidden="true">
      <span class="project-doc-label">{nameFor(dragId)}</span>
    </div>
  {/if}

  <button
    class="project-strip-btn project-new"
    type="button"
    title="New project (⌘N)"
    aria-label="New project"
    onclick={() => PM.newProject?.()}
  ><Icon {PM} name="plus" /></button>
</div>

<div class="titlebar-drag" aria-hidden="true"></div>

<div class="tb-right" id="tb-right">
  <ToolbarMount {PM} />
  {#if !homeOpen}
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
  {#if !homeOpen}
    <button class="btn pri tb-export" type="button" title="Export… (⌘E)" aria-label="Export…" onclick={() => PM.Export?.dialog?.()}>
      <Icon {PM} name="export" />Export
    </button>
  {/if}
</div>
