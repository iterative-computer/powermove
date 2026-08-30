<script lang="ts">
  import { flushSync } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import Conversation from '../panels/agent/Conversation.svelte';
  import Composer from '../panels/agent/Composer.svelte';
  import { panelIcons } from '../panels/panel-icons';
  import { panelScope } from '../panels/agent/panel-focus';
  import { panelBelongsInLibrary, panelPreviewSize } from './panel-preview';

  let { PM }: { PM: Record<string, any> } = $props();

  let shown = $state(false);
  let view = $state<'panels' | 'workspaces'>('panels');
  let editingId = $state<string | null>(null);
  let searchText = $state('');
  const query = $derived(searchText.trim().toLowerCase());
  let version = $state(0);
  let lastFocus: HTMLElement | null = null;
  let searchEl = $state<HTMLInputElement | null>(null);
  let rootEl = $state<HTMLElement | null>(null);

  export function open(target?: 'panels' | 'workspaces'): void {
    if (target) view = target;
    version += 1;
    lastFocus = document.activeElement as HTMLElement | null;
    shown = true;
    const app = document.getElementById('app');
    if (app) app.inert = true;
    window.requestAnimationFrame(() => searchEl?.focus());
  }

  export function close(): void {
    if (!shown) return;
    /* Flush so the live-panel mirror returns its element to the pool before
       callers mutate the workspace layout. */
    flushSync(() => {
      leaveEditor(false);
      shown = false;
    });
    flying?.cancel();
    endDrag();
    const app = document.getElementById('app');
    if (app) app.inert = false;
    const focus = lastFocus;
    lastFocus = null;
    if (focus?.isConnected) focus.focus();
  }

  export function isOpen(): boolean {
    return shown;
  }

  export function refresh(): void {
    version += 1;
  }

  const panels = $derived.by(() => {
    void version;
    return Object.entries(PM.PANELS || {})
      .map(([id, def]: [string, any]) => ({ ...def, id, title: def.title || id }))
      .filter(panelBelongsInLibrary)
      .sort((a, b) => String(a.title).localeCompare(String(b.title)));
  });
  const icons = $derived(panelIcons(panels, PM.ICONS || {}));
  const matchedPanels = $derived(panels.filter((panel) =>
    !query || `${panel.title} ${panel.id}`.toLowerCase().includes(query)));

  const workspaces = $derived.by(() => {
    void version;
    return [...((PM.WS?.all ?? []) as any[])];
  });
  const currentWorkspaceId = $derived.by(() => {
    void version;
    return PM.WS?.current?.id as string | undefined;
  });
  const matchedWorkspaces = $derived(workspaces.filter((workspace) =>
    !query || String(workspace.name || workspace.id).toLowerCase().includes(query)));

  function inWorkspace(id: string): boolean {
    void version;
    if (id === 'toolbar') return true;
    const found = PM.Layout?.findPanel?.(PM.WS?.current, id);
    return !!(found && !found.dock.hidden && !found.spec.collapsed);
  }

  const editingPanel = $derived(editingId ? panels.find((panel) => panel.id === editingId) ?? null : null);
  const editingLive = $derived.by(() => {
    void version;
    if (!editingId) return false;
    const element = PM.panelInst?.[editingId]?.el as HTMLElement | undefined;
    return !!element && !element.classList.contains('popped');
  });

  /* The picked panel keeps its identity across the swap: it travels and grows
     from its card into the editor stage, and shrinks back onto the card on the
     way out. Measure where the box was, put it back there with a transform,
     then let it animate to where it now is (FLIP). Nothing in the library CSS
     may transform an ancestor of either box during this, or the measured
     geometry drifts under the animation. */
  const FLIP_MS = 260;
  const FLIP_EASE = 'cubic-bezier(.22,1,.36,1)';

  function wantsMotion(): boolean {
    return !window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches;
  }

  function cardBox(id: string): HTMLElement | null {
    return rootEl?.querySelector<HTMLElement>(
      `[data-panel-id="${CSS.escape(id)}"] .library-live`
    ) ?? null;
  }

  function stageBox(): HTMLElement | null {
    return rootEl?.querySelector<HTMLElement>('.library-stage-frame') ?? null;
  }

  /* Travels `node` from the `from` box to where it already sits. Transform
     only: any opacity on the moving panel — or on an ancestor fading it in —
     reads as a flash rather than as one object changing place. */
  function flip(node: HTMLElement, from: DOMRect, done?: () => void): Animation | null {
    const to = node.getBoundingClientRect();
    if (!from.width || !from.height || !to.width || !to.height) { done?.(); return null; }
    const origin = node.style.transformOrigin;
    node.style.transformOrigin = '0 0';
    const shift = `translate(${from.left - to.left}px,${from.top - to.top}px)`;
    const scale = `scale(${from.width / to.width},${from.height / to.height})`;
    const animation = node.animate(
      [{ transform: `${shift} ${scale}` }, { transform: 'none' }],
      { duration: FLIP_MS, easing: FLIP_EASE }
    );
    const settle = () => { node.style.transformOrigin = origin; done?.(); };
    animation.onfinish = settle;
    animation.oncancel = settle;
    return animation;
  }

  /* cloneNode leaves canvases blank and drops live form values; copy both so
     every panel — including future generated ones — previews as-is. */
  function copyLiveState(source: Element, target: Element): void {
    const canvases = source.querySelectorAll('canvas');
    target.querySelectorAll('canvas').forEach((node, index) => {
      const from = canvases[index];
      if (!from?.width || !from?.height) return;
      node.width = from.width;
      node.height = from.height;
      try { node.getContext('2d')?.drawImage(from, 0, 0); } catch { /* tainted or GL-only canvas */ }
    });
    const fields = source.querySelectorAll<HTMLInputElement>('input,textarea,select');
    target.querySelectorAll<HTMLInputElement>('input,textarea,select').forEach((node, index) => {
      const from = fields[index];
      if (from) node.value = from.value;
    });
  }

  /* On the way back the card cannot simply be flipped: at stage size it is
     clipped by its own tile and by the scrolling grid. So a copy of the card
     flies over the whole screen instead, and the card itself stays hidden
     until the copy lands on it. Anchored to #library-screen — a fixed ghost
     would be re-based by any transformed ancestor, and .library-card takes a
     transform on hover. */
  let flying: Animation | null = null;

  function flyBack(id: string, from: DOMRect): void {
    const card = cardBox(id);
    if (!card || !rootEl) return;
    card.closest('.library-card')?.scrollIntoView({ block: 'nearest' });
    const to = card.getBoundingClientRect();
    const base = rootEl.getBoundingClientRect();
    if (!to.width || !to.height) return;
    const ghost = card.cloneNode(true) as HTMLElement;
    ghost.classList.add('library-fly');
    ghost.style.left = `${to.left - base.left}px`;
    ghost.style.top = `${to.top - base.top}px`;
    ghost.style.width = `${to.width}px`;
    ghost.style.height = `${to.height}px`;
    copyLiveState(card, ghost);
    rootEl.appendChild(ghost);
    card.style.visibility = 'hidden';
    flying = flip(ghost, from, () => {
      flying = null;
      ghost.remove();
      card.style.visibility = '';
    });
  }

  function openEditor(id: string): void {
    /* A ghost still landing from a previous back would outlive its grid. */
    flying?.cancel();
    const from = wantsMotion() ? cardBox(id)?.getBoundingClientRect() ?? null : null;
    editingId = id;
    PM.AgentUI?.setScope?.(panelScope([id]));
    PM.AgentUI?.setDraft?.('', true);
    if (!from) return;
    /* Render the editor now so the stage frame can be measured at its real
       size before the first painted frame of the animation. */
    flushSync();
    const stage = stageBox();
    if (stage) flip(stage, from);
  }

  /* `animate` is off when the whole library is closing: close() returns the
     borrowed panel inside its own flushSync, and there is no card left on
     screen to shrink back onto. */
  function leaveEditor(animate = true): void {
    if (!editingId) return;
    const id = editingId;
    const from = animate && wantsMotion() ? stageBox()?.getBoundingClientRect() ?? null : null;
    editingId = null;
    PM.AgentUI?.setScope?.('workspace');
    if (!from) return;
    flushSync();
    flyBack(id, from);
  }

  /* Borrows the live panel element from the layout pool, exactly like the
     panel pop-out does, so the editor previews the real panel — state,
     handlers and agent edits included — then hands it back on teardown. */
  function mirrorPanel(node: HTMLElement, id: string) {
    const element = PM.panelInst?.[id]?.el as HTMLElement | undefined;
    let borrowed: HTMLElement | null = null;
    if (element && !element.classList.contains('popped')) {
      borrowed = element;
      borrowed.classList.add('popped');
      node.appendChild(borrowed);
    }
    return {
      destroy() {
        if (!borrowed) return;
        borrowed.classList.remove('popped');
        const host = document.querySelector<HTMLElement>(
          `#pm-panel-pool [data-panel-host="${CSS.escape(id)}"]`
        );
        host?.appendChild(borrowed);
        borrowed = null;
        /* Layout.apply flushes synchronously; running it inside this teardown
           (itself a Svelte flush) would abort the view swap, so defer it. */
        queueMicrotask(() => PM.Layout?.apply?.(PM.WS?.current));
      }
    };
  }

  function addToWorkspace(id: string): void {
    PM.LibraryUI?.reveal?.(id);
  }

  /* Each grid cell shows a snapshot clone of the live panel, laid out at the
     canonical size owned by its definition. The active dock is only a source
     of panel content; its width, height, collapsed state, and location never
     shape the Library. A clone, not a borrow, keeps the workspace intact while
     a preview is dragged back onto a dock. */
  function panelPreview(node: HTMLElement, params: { id: string; version: number }) {
    let frame: HTMLElement | null = null;
    let sourceWidth = 0;
    let sourceHeight = 0;
    const observer = new ResizeObserver(() => fit());
    observer.observe(node);
    function fit(): void {
      if (!frame || !sourceWidth || !sourceHeight) return;
      const scale = Math.min(node.clientWidth / sourceWidth, node.clientHeight / sourceHeight);
      frame.style.transform = `scale(${scale})`;
      frame.style.left = `${Math.max(0, (node.clientWidth - sourceWidth * scale) / 2)}px`;
      frame.style.top = `${Math.max(0, (node.clientHeight - sourceHeight * scale) / 2)}px`;
    }
    function render(id: string): void {
      frame?.remove();
      frame = null;
      node.classList.remove('has-preview');
      node.style.aspectRatio = '16 / 10';
      const element = PM.panelInst?.[id]?.el as HTMLElement | undefined;
      if (!element) return;
      const size = panelPreviewSize(PM.PANELS?.[id] ?? {});
      sourceWidth = size.width;
      sourceHeight = size.height;
      node.style.aspectRatio = `${sourceWidth} / ${sourceHeight}`;
      const clone = element.cloneNode(true) as HTMLElement;
      clone.classList.add('library-clone');
      clone.removeAttribute('id');
      clone.style.width = `${sourceWidth}px`;
      clone.style.height = `${sourceHeight}px`;
      clone.style.flex = 'none';
      for (const inner of clone.querySelectorAll('[id]')) inner.removeAttribute('id');
      copyLiveState(element, clone);
      frame = document.createElement('div');
      frame.className = 'library-live-frame';
      frame.style.width = `${sourceWidth}px`;
      frame.style.height = `${sourceHeight}px`;
      frame.appendChild(clone);
      node.appendChild(frame);
      node.classList.add('has-preview');
      fit();
    }
    render(params.id);
    return {
      update(next: { id: string; version: number }) {
        render(next.id);
      },
      destroy() {
        observer.disconnect();
        frame?.remove();
        node.classList.remove('has-preview');
        node.style.removeProperty('aspect-ratio');
      }
    };
  }

  /* Dragging a card temporarily lifts the library out of the way (and lifts
     the #app inert guard) so the card can be dropped straight onto a dock. */
  const PANEL_MIME = 'application/x-powermove-panel';
  let dragging = $state(false);
  let dragActive = false;
  let hintDock: HTMLElement | null = null;

  function setHint(dock: HTMLElement | null): void {
    if (hintDock === dock) return;
    hintDock?.classList.remove('library-drop-hint');
    hintDock = dock;
    hintDock?.classList.add('library-drop-hint');
  }

  function dockDragOver(event: DragEvent): void {
    if (!event.dataTransfer?.types.includes(PANEL_MIME)) return;
    const dock = (event.target as HTMLElement | null)?.closest?.('[data-dock]') as HTMLElement | null;
    setHint(dock);
    if (!dock) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
  }

  function dockDrop(event: DragEvent): void {
    const id = event.dataTransfer?.getData(PANEL_MIME);
    const dock = (event.target as HTMLElement | null)?.closest?.('[data-dock]') as HTMLElement | null;
    if (!id || !dock) return;
    event.preventDefault();
    const dockId = dock.dataset.dock!;
    PM.WS?.mutate?.((workspace: any) => {
      if (workspace.hiddenPanels?.some((panel: any) => panel.id === id)) PM.Layout.restorePanel(workspace, id);
      if (!PM.Layout.hasPanel(workspace, id)) PM.Layout.addPanel(workspace, id, dockId);
      else PM.Layout.movePanel?.(workspace, id, dockId);
      const found = PM.Layout.findPanel(workspace, id);
      if (found) { found.dock.hidden = false; found.spec.collapsed = false; }
    });
    endDrag();
    PM.LibraryUI?.close?.();
  }

  function endDrag(): void {
    if (!dragActive) return;
    dragActive = false;
    dragging = false;
    setHint(null);
    window.removeEventListener('dragover', dockDragOver, true);
    window.removeEventListener('drop', dockDrop, true);
    const app = document.getElementById('app');
    if (app && shown) app.inert = true;
  }

  function dragStart(event: DragEvent, id: string): void {
    if (!event.dataTransfer) return;
    event.dataTransfer.setData(PANEL_MIME, id);
    event.dataTransfer.effectAllowed = 'copy';
    dragActive = true;
    window.addEventListener('dragover', dockDragOver, true);
    window.addEventListener('drop', dockDrop, true);
    const app = document.getElementById('app');
    if (app) app.inert = false;
    /* Hiding the overlay inside dragstart would cancel the native drag, so
       reveal the editor underneath one tick later. */
    setTimeout(() => { if (dragActive) dragging = true; }, 0);
  }

  function activateWorkspace(id: string): void {
    PM.WS?.activate?.(id);
    version += 1;
  }

  function deleteWorkspace(event: MouseEvent, workspace: any): void {
    event.stopPropagation();
    PM.WS?.remove?.(workspace.id);
    version += 1;
  }

  function workspaceDocks(workspace: any): Array<{ flex: boolean; panels: string[] }> {
    const docks = (workspace.layout?.docks ?? []).filter((dock: any) => !dock.hidden);
    return docks.map((dock: any) => ({ flex: !!dock.flex, panels: (dock.panels ?? []).map((panel: any) => panel.id) }));
  }

  function workspaceSummary(workspace: any): string {
    const count = (workspace.layout?.docks ?? []).reduce((total: number, dock: any) => total + (dock.panels?.length ?? 0), 0);
    return `${workspace.builtin ? 'Built-in' : 'Custom'} · ${count} panel${count === 1 ? '' : 's'}`;
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      if (editingId) leaveEditor();
      else PM.LibraryUI?.close?.();
      return;
    }
    if (event.key !== 'Tab' || !rootEl) return;
    const items = [...rootEl.querySelectorAll<HTMLElement>('button,input,textarea,select')]
      .filter((item) => !item.hasAttribute('disabled') && item.offsetParent !== null);
    const first = items[0];
    const last = items.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  id="library-overlay"
  class:on={shown}
  class:dragging
  onpointerdown={(event) => { if (event.target === event.currentTarget) PM.LibraryUI?.close?.(); }}
  onkeydown={keydown}
>
  <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
  <section
    id="library-screen"
    class:on={shown}
    role="dialog"
    aria-modal="true"
    aria-label="Panel library"
    tabindex="-1"
    bind:this={rootEl}
  >
    <header class="library-head">
      {#if editingPanel}
        <button class="iconbtn library-back" type="button" aria-label="Back to panels" onclick={() => leaveEditor()}>
          <Icon {PM} name="chev" />
        </button>
        <div class="library-title"><b>{editingPanel.title}</b><span>Describe a change and the agent applies it to this panel</span></div>
      {:else}
        <div class="library-title"><b>Library</b></div>
      {/if}
      <button class="iconbtn" type="button" aria-label="Close Library" onclick={() => PM.LibraryUI?.close?.()}>
        <Icon {PM} name="x" />
      </button>
    </header>

    {#if editingPanel}
      <div class="library-editor">
        <aside class="library-chat" aria-label="Panel conversation">
          <div class="library-chat-log">
            <Conversation {PM} />
          </div>
          <div class="library-chat-foot">
            <Composer {PM} panelId="library" />
          </div>
        </aside>
        <section class="library-stage" aria-label="Panel preview">
          <div class="library-stage-bar">
            <Icon {PM} name={icons[editingPanel.id] || 'panel'} />
            <b>{editingPanel.title}</b>
            <span class="sp"></span>
            {#if inWorkspace(editingPanel.id)}
              <span class="library-count">In workspace</span>
            {:else}
              <button class="btn pri" type="button" onclick={() => addToWorkspace(editingPanel.id)}>
                Add to workspace
              </button>
            {/if}
          </div>
          {#key editingId}
            <div class="library-stage-frame pop-mirror" use:mirrorPanel={editingPanel.id}>
              {#if !editingLive}
                <div class="library-stage-empty">
                  <Icon {PM} name={icons[editingPanel.id] || 'panel'} />
                  <b>No live preview yet</b>
                  <span>Add {editingPanel.title} to your workspace to see and edit the real panel here.</span>
                </div>
              {/if}
            </div>
          {/key}
        </section>
      </div>
    {:else}
      <div class="library-shell">
        <main class="library-main">
          <div class="library-top">
            <div class="segmented" aria-label="Library sections">
              <button class:on={view === 'panels'} type="button" onclick={() => { view = 'panels'; searchText = ''; }}>
                Panels
              </button>
              <button class:on={view === 'workspaces'} type="button" onclick={() => { view = 'workspaces'; searchText = ''; }}>
                Workspaces
              </button>
            </div>
            <span class="sp"></span>
            <label class="library-search">
              <Icon {PM} name="search" />
              <input
                type="search"
                placeholder={view === 'panels' ? 'Search panels' : 'Search workspaces'}
                aria-label={view === 'panels' ? 'Search panels' : 'Search workspaces'}
                bind:this={searchEl}
                bind:value={searchText}
              />
            </label>
            <div class="library-top-action">
              {#if view === 'panels'}
                <button class="btn" type="button" onclick={() => PM.LibraryUI?.create?.()}>
                  <Icon {PM} name="plus" />
                  New panel
                </button>
              {:else}
                <button class="btn" type="button" onclick={() => PM.WS?.saveAsNew?.()}>
                  <Icon {PM} name="plus" />
                  Save current as new
                </button>
              {/if}
            </div>
          </div>
          <div class="library-content">
            {#if view === 'panels'}
              <div class="library-grid panels" role="list" aria-label="Available panels">
                {#each matchedPanels as panel (panel.id)}
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div
                    class="library-card library-card-live"
                    role="listitem"
                    data-panel-id={panel.id}
                    draggable="true"
                    ondragstart={(event) => dragStart(event, panel.id)}
                    ondragend={endDrag}
                  >
                    <button class="library-card-hit" type="button" aria-label={`Edit ${panel.title}`} title={panel.title} onclick={() => openEditor(panel.id)}>
                      <div class="library-live" use:panelPreview={{ id: panel.id, version }}>
                        <span class="library-thumb-icon">
                          <Icon {PM} name={icons[panel.id] || 'panel'} />
                          <b>{panel.title}</b>
                        </span>
                      </div>
                    </button>
                    <button
                      class="iconbtn library-card-add"
                      type="button"
                      aria-label={inWorkspace(panel.id) ? `Show ${panel.title} in workspace` : `Add ${panel.title} to workspace`}
                      onclick={() => addToWorkspace(panel.id)}
                    >
                      <Icon {PM} name={inWorkspace(panel.id) ? 'eye' : 'plus'} />
                    </button>
                  </div>
                {:else}
                  <div class="library-empty">
                    <b>No panels match</b>
                    <span>{query ? 'Try a different search.' : 'No panels are available.'}</span>
                  </div>
                {/each}
              </div>
            {:else}
              <div class="library-grid workspaces" role="list" aria-label="Workspaces">
                {#each matchedWorkspaces as workspace (workspace.id)}
                  <div class="library-card workspace-card" class:active={workspace.id === currentWorkspaceId} role="listitem" data-workspace-id={workspace.id}>
                    {#if workspace.id === currentWorkspaceId}
                      <span class="library-card-badge">Active</span>
                    {/if}
                    <button class="library-card-hit" type="button" aria-label={`Activate ${workspace.name}`} title={workspace.name} onclick={() => activateWorkspace(workspace.id)}>
                      <div class="workspace-map" style:--accent={workspace.theme?.accent || undefined}>
                        {#each workspaceDocks(workspace) as dock}
                          <div class="workspace-map-dock" class:flex={dock.flex}>
                            {#each dock.panels as panelId}
                              <span class="workspace-map-panel">
                                <Icon {PM} name={icons[panelId] || 'panel'} />
                                <b>{PM.PANELS?.[panelId]?.title || panelId}</b>
                              </span>
                            {:else}
                              <span class="workspace-map-panel empty"></span>
                            {/each}
                          </div>
                        {/each}
                      </div>
                      <span class="workspace-card-label">
                        <b>{workspace.name}</b>
                        <span>{workspaceSummary(workspace)}</span>
                      </span>
                    </button>
                    {#if !workspace.builtin}
                      <button
                        class="iconbtn library-card-add"
                        type="button"
                        aria-label={`Delete ${workspace.name}`}
                        onclick={(event) => deleteWorkspace(event, workspace)}
                      >
                        <Icon {PM} name="trash" />
                      </button>
                    {/if}
                  </div>
                {:else}
                  <div class="library-empty">
                    <b>No workspaces match</b>
                    <span>{query ? 'Try a different search.' : 'Save the current layout to create one.'}</span>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        </main>
      </div>
    {/if}
  </section>
</div>
