<script lang="ts">
  import Icon from '../panels/Icon.svelte';
  import ToolbarMount from './ToolbarMount.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let refreshToken = $state(0);
  let rovingId = $state('');
  let renamingId = $state<string | null>(null);
  let renameValue = $state('');

  const tabIds = $derived.by(() => {
    refreshToken;
    return [...(PM.Projects?.tabs?.() ?? [])] as string[];
  });
  const metas = $derived.by(() => {
    refreshToken;
    return [...(PM.Projects?.list?.() ?? [])] as Array<{ id: string; name?: string }>;
  });
  const homeOpen = $derived.by(() => {
    refreshToken;
    return !!PM.ProjectsScreen?.isOpen;
  });
  const activeProjectId = $derived.by(() => {
    refreshToken;
    return PM.proj?.id as string | undefined;
  });
  const appDirty = $derived.by(() => {
    refreshToken;
    return !!PM.app?.dirty;
  });
  const selectedId = $derived(homeOpen ? 'home' : (activeProjectId ?? tabIds[0] ?? 'home'));


  function nameFor(id: string): string {
    return metas.find((meta) => meta.id === id)?.name || 'Untitled';
  }

  function tabIndex(id: string): 0 | -1 {
    return (rovingId || selectedId) === id ? 0 : -1;
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

  function showProjects(): void {
    rovingId = 'home';
    PM.ProjectsScreen?.show?.();
  }

  function openProject(id: string): void {
    if (id === PM.proj?.id) return;
    const raw = PM.Projects?.get?.(id);
    if (!raw) {
      PM.toast?.('That project could not be found');
      PM.Projects?.markClosed?.(id);
      PM.bus?.emit?.('projects:tabs');
      return;
    }
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: raw }));
  }

  async function closeProject(id: string): Promise<void> {
    if (PM.confirmCloseProject && !await PM.confirmCloseProject(id)) return;
    if (rovingId === id) rovingId = '';
    const active = id === PM.proj?.id;
    if (active) {
      try {
        PM.Projects?.put?.(PM.proj);
      } catch (error) {
        window.console.warn('Project save failed', error);
      }
    }
    PM.Projects?.markClosed?.(id);
    const remaining = [...(PM.Projects?.tabs?.() ?? [])] as string[];
    if (active) {
      if (remaining.length) openProject(remaining[0]!);
      else {
        const project = PM.mkProject?.({ name: 'Untitled', dur: 10, w: 1920, h: 1080, fps: 30, bg: '#09090A' });
        if (project) window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
      }
    }
    PM.bus?.emit?.('projects:tabs');
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
    PM.bus?.emit?.('projects:tabs');
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

  function projectClick(event: MouseEvent, id: string): void {
    if (renamingId === id || event.target instanceof HTMLInputElement) return;
    if (event.detail > 1) {
      event.preventDefault();
      beginRename(id);
      return;
    }
    rovingId = id;
    if (PM.ProjectsScreen?.isOpen) PM.ProjectsScreen.hide?.();
    openProject(id);
  }

  function projectKeydown(event: KeyboardEvent, id: string): void {
    if (event.key === 'F2') {
      event.preventDefault();
      event.stopPropagation();
      beginRename(id);
      return;
    }
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
      focusTab(event);
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
    const events = ['projects:tabs', 'projects:screen', 'workspaces', 'project', 'history'];
    const offs = events.map((event) => PM.bus?.on?.(event, () => {
      if (!renamingId) {
        if (!document.getElementById('tabs')?.contains(document.activeElement)) rovingId = '';
        refreshToken++;
      }
    }));
    return () => offs.forEach((off) => off?.());
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

  $effect(() => {
    refreshToken;
    activeProjectId;
    const frame = window.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>('#tabs .project-doc.on')?.scrollIntoView?.({
        block: 'nearest',
        inline: 'nearest'
      });
    });
    return () => window.cancelAnimationFrame(frame);
  });
</script>

<div id="tabs" data-svelte-shell="tabs">
  <!-- Phase 5.4 follow-up: connect these tabs to a tabpanel with aria-controls. -->
  <div role="tablist" aria-label="Open projects" style="display: contents">
    <button
      class="project-strip-btn project-home"
      class:on={homeOpen}
      type="button"
      role="tab"
      data-tab-id="home"
      title="Projects"
      aria-label="Projects"
      aria-selected={homeOpen}
      tabindex={tabIndex('home')}
      onclick={showProjects}
      onfocus={() => (rovingId = 'home')}
      onkeydown={focusTab}
    ><Icon {PM} name="home" /></button>
    <i class="project-strip-divider" aria-hidden="true"></i>

    {#each tabIds as id (id)}
      {@const active = id === activeProjectId && !homeOpen}
      {@const dirty = PM.projectFileState?.(id)?.dirty ?? (id === activeProjectId && appDirty)}
      {@const tabName = nameFor(id)}
      <div
        class="project-doc"
        class:on={active}
        class:dirty
        class:renaming={renamingId === id}
        title={PM.projectFileState?.(id)?.path || `${tabName} — Not saved to a file`}
        role="tab"
        data-tab-id={id}
        aria-selected={active}
        aria-label={renamingId === id ? `Rename ${tabName}` : `${tabName}${dirty ? ', unsaved' : ''}`}
        tabindex={tabIndex(id)}
        onclick={(event) => projectClick(event, id)}
        oncontextmenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
          PM.menu(event.currentTarget, [{ label: 'Rename project…', kb: 'F2', run: () => beginRename(id) }], { x: event.clientX, y: event.clientY });
        }}
        ondblclick={(event) => {
          if ((event.target as Element)?.closest?.('.project-doc-close')) return;
          event.preventDefault();
          event.stopPropagation();
          beginRename(id);
        }}
        onfocus={() => (rovingId = id)}
        onkeydown={(event) => projectKeydown(event, id)}
        onauxclick={(event) => {
          if (event.button === 1) {
            event.preventDefault();
            closeProject(id);
          }
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
          <button
            class="project-doc-close"
            type="button"
            title="Close project"
            aria-label={`Close ${tabName}`}
            tabindex="-1"
            onclick={(event) => {
              event.stopPropagation();
              closeProject(id);
            }}
          ><Icon {PM} name="x" /></button>
        {/if}
      </div>
    {/each}
  </div>

  <button
    class="project-strip-btn project-new"
    type="button"
    title="New project"
    aria-label="New project"
    onclick={() => {
      if (PM.ProjectsScreen?.isOpen) PM.ProjectsScreen.hide?.();
      PM.newProject?.();
    }}
  ><Icon {PM} name="plus" /></button>
</div>

<div class="titlebar-drag" aria-hidden="true"></div>
<ToolbarMount {PM} />
<div class="tb-right" id="tb-right">
  <button class="iconbtn" type="button" title="Panel library" aria-label="Open panel library" onclick={() => PM.LibraryUI?.open?.()}>
    <Icon {PM} name="grid" />
  </button>
  <button class="iconbtn" type="button" title="Settings" aria-label="Open settings" onclick={() => PM.SettingsUI?.open?.()}>
    <Icon {PM} name="gear" />
  </button>
</div>
