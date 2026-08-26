<script lang="ts">
  import type { PanelProps } from './registerSveltePanel';

  interface EffectDefinition {
    label: string;
    group: string;
  }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  const groups = Object.entries(PM.FX as Record<string, EffectDefinition>).reduce(
    (result, [id, definition]) => {
      (result[definition.group] ||= []).push([id, definition]);
      return result;
    },
    {} as Record<string, Array<[string, EffectDefinition]>>
  );
  let status = $state('');

  function addEffect(event: MouseEvent, effect: string, label: string): void {
    /* Native double-click dispatches click, click, dblclick. Preserve the
       legacy guard so one gesture still creates only one effect. */
    if (event.detail > 1) return;
    const layer = PM.firstSel();
    if (!layer) {
      status = 'Select a layer first';
      PM.toast(status);
      return;
    }
    PM.Edit.apply(
      { type: 'add_effect', target: layer.id, effect },
      { label: `Add ${label}`, origin: 'effects-panel' }
    );
    PM.Inspector.refresh();
    PM.invalidate();
    status = `Added ${label}`;
  }
</script>

<div class="simple-panel-list" data-svelte-panel={panelId}>
  {#each Object.entries(groups) as [group, effects] (group)}
    <div class="sec simple-panel-section">{group}</div>
    {#each effects as [effect, definition] (effect)}
      <button class="lyr simple-effect-row" type="button" onclick={(event) => addEffect(event, effect, definition.label)}>
        <span class="nm">{definition.label}</span>
        <span class="idx" aria-hidden="true">+</span>
      </button>
    {/each}
  {/each}
  <span class="panel-sr-only" role="status">{status}</span>
</div>
