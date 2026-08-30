<script lang="ts">
  import type { PMRegistry } from '../legacy/registry';
  import { panelPoolHost } from './panel';
  import type { DockSpec, PanelSpec, Workspace } from './model';

  let { PM, workspace, tick }: { PM: PMRegistry; workspace: Workspace; tick: number } = $props();

  function mountOnDocumentBody(node: HTMLElement): { destroy(): void } {
    document.body.appendChild(node);
    return {
      destroy() {
        node.remove();
      }
    };
  }

  const entries = $derived.by(() => {
    void tick;
    const found = new Map<string, { spec: PanelSpec; dock: DockSpec }>();
    for (const dock of workspace.layout?.docks ?? []) {
      for (const spec of dock.panels ?? []) if (!found.has(spec.id)) found.set(spec.id, { spec, dock });
    }
    for (const hidden of workspace.hiddenPanels ?? []) {
      if (!hidden.id || found.has(hidden.id)) continue;
      found.set(hidden.id, {
        spec: { ...(hidden.spec ?? {}), id: hidden.id },
        dock: { id: hidden.dockId || hidden.dock?.id || 'right', panels: [], ...(hidden.dock ?? {}) }
      });
    }
    /* Panels outside the workspace stay mounted too, parked in the pool, so
       the library can always preview the live panel. */
    for (const id of Object.keys(PM.PANELS ?? {})) {
      if (id === 'toolbar' || found.has(id)) continue;
      found.set(id, { spec: { id }, dock: { id: 'right', panels: [] } });
    }
    return [...found.values()];
  });
</script>

<div
  id="pm-panel-pool"
  aria-hidden="true"
  style="position:absolute;inset-inline-start:-9999px;inset-block-start:0;width:1px;height:1px;overflow:hidden;pointer-events:none"
  use:mountOnDocumentBody
>
  {#each entries as entry (entry.spec.id)}
    <!-- Parked hosts get a real dock-like size so panels that measure
         themselves (canvases, virtual lists, composers) lay out sanely and
         preview correctly instead of collapsing to the pool's 1px width. -->
    <div
      data-panel-host={entry.spec.id}
      style="width:320px;height:420px;display:flex;flex-direction:column"
      use:panelPoolHost={{ PM, spec: entry.spec, dock: entry.dock }}
    ></div>
  {/each}
</div>
