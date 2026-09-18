<script lang="ts">
  import { doc } from '../state/document.svelte';
  import Icon from './Icon.svelte';
  import type { PanelProps } from './registerSveltePanel';

  interface Take {
    id: string;
    label: string;
    at: number;
  }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  let takesVersion = $state(0);
  const takes = $derived.by(() => {
    takesVersion;
    doc.proj;
    return [...(PM.takes.all() as Take[])];
  });
  let status = $state('');

  $effect(() => {
    /* Phase 5 removal: takes have no document tick, so bridge the legacy bus
       event into the local rune until the store is migrated. */
    const off = PM.bus.on('takes', () => { takesVersion += 1; });
    return () => off?.();
  });

  /* Same-day takes read as a clock time; older ones as a short date, so the
     meta column stays one short token and never wraps. */
  function timeLabel(at: number): string {
    const when = new Date(at);
    const now = new Date();
    const sameDay = when.getFullYear() === now.getFullYear() && when.getMonth() === now.getMonth() && when.getDate() === now.getDate();
    return sameDay
      ? when.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
      : when.toLocaleDateString([], { month: 'short', day: 'numeric' });
  }

  function fullTime(at: number): string {
    return new Date(at).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  }

  function restoreTake(take: Take): void {
    PM.takes.restore(take.id);
    status = `Restored ${take.label}`;
  }

  function dropTake(event: MouseEvent, take: Take): void {
    event.stopPropagation();
    PM.takes.drop(take.id);
    status = `Deleted ${take.label}`;
  }

  function saveTake(): void {
    const saved = PM.takes.save();
    status = saved?.label ? `Saved ${saved.label}` : 'Take saved';
  }
</script>

<div class="simple-panel-list" data-svelte-panel={panelId}>
  {#each takes as take (take.id)}
    <div class="lyr simple-panel-row">
      <button class="simple-row-action" type="button" title={`${take.label} · ${fullTime(take.at)}`} onclick={() => restoreTake(take)}>
        <span class="nm">{take.label}</span>
        <span class="idx">{timeLabel(take.at)}</span>
      </button>
      <button class="stopwatch" type="button" aria-label={`Delete ${take.label}`} onclick={(event) => dropTake(event, take)}>
        <Icon {PM} name="x" />
      </button>
    </div>
  {:else}
    <div class="empty">Save a take before exploring a new motion direction.</div>
  {/each}
  <div class="simple-panel-actions">
    <button class="chip" type="button" onclick={saveTake}>
      <Icon {PM} name="plus" />
      Save take
    </button>
  </div>
  <span class="panel-sr-only" role="status">{status}</span>
</div>
