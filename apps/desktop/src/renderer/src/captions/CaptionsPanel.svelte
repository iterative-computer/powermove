<script lang="ts">
  import { tick } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { sel } from '../state/selection.svelte';
  import { openPopoverMenu } from '../controls/popover-menu';
  import type { PanelProps } from '../panels/registerSveltePanel';
  import { cueIndexAt, type CaptionCue } from './model';

  /* A quiet list of the cues of one captions layer, for reading and fixing
     captions by eye: timecode and text, nothing else. Click seeks, typing
     edits in place, arrows walk the list. Long lists render only the rows
     in view. */
  let { panelId }: PanelProps = $props();
  const PM = window.PM as Record<string, any>;
  const ROW = 28;

  let selectionVersion = $state(0);
  $effect(() => PM.Captions?.onChange?.(() => { selectionVersion++; }));

  const layers = $derived((doc.tick.structure, doc.tick.values, doc.proj,
    (doc.proj?.layers ?? []).filter((layer: any) => layer.type === 'captions') as any[]));
  let chosen = $state<string | null>(null);
  /* The panel follows the captions layer selected anywhere in the editor. */
  const layer = $derived.by(() => {
    selectionVersion;
    const picked = layers.find(item => sel.layers.includes(item.id))
      ?? layers.find(item => item.id === chosen) ?? layers[0] ?? null;
    return picked;
  });
  const cues = $derived((doc.tick.values, doc.tick.structure, layer ? layer.d.cues as CaptionCue[] : []));
  const selected = $derived.by(() => {
    selectionVersion;
    const current = PM.Captions?.selection?.();
    return new Set<string>(current && layer && current.layerId === layer.id ? current.cues : []);
  });

  let query = $state('');
  const shown = $derived.by(() => {
    const needle = query.trim().toLocaleLowerCase();
    const list = cues.map((cue, index) => ({ cue, index }));
    return needle ? list.filter(entry => entry.cue.text.toLocaleLowerCase().includes(needle)) : list;
  });
  const playing = $derived(layer ? cueIndexAt(cues, transport.time - layer.from) : -1);

  /* ── windowing ─────────────────────────────────────────── */
  let scroller: HTMLElement | undefined = $state();
  let scrollTop = $state(0);
  let viewport = $state(300);
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 6));
  const last = $derived(Math.min(shown.length, Math.ceil((scrollTop + viewport) / ROW) + 6));
  $effect(() => {
    if (!scroller) return;
    const observer = new ResizeObserver(() => { viewport = scroller?.clientHeight ?? viewport; });
    observer.observe(scroller);
    return () => observer.disconnect();
  });
  function reveal(position: number) {
    if (!scroller || position < 0) return;
    const top = position * ROW;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (top + ROW > scroller.scrollTop + scroller.clientHeight) scroller.scrollTop = top + ROW - scroller.clientHeight;
  }
  /* Keep the spoken cue in view while playing, unless the person is typing. */
  let lastPlaying = -1;
  $effect(() => {
    const index = playing;
    if (index === lastPlaying || editing || !transport.playing) { lastPlaying = index; return; }
    lastPlaying = index;
    reveal(shown.findIndex(entry => entry.index === index));
  });

  /* ── actions ───────────────────────────────────────────── */
  const fps = () => Number(doc.proj?.fps) || 30;
  const timecode = (seconds: number) => PM.tc ? PM.tc(seconds, fps()) : seconds.toFixed(2);
  let focusIndex = $state(-1);
  let listEl: HTMLElement | undefined = $state();

  function choose(position: number, event?: MouseEvent | KeyboardEvent) {
    const entry = shown[position];
    if (!entry || !layer) return;
    focusIndex = position;
    const id = entry.cue.id;
    const extend = event && (event.metaKey || event.shiftKey);
    const next = extend ? (selected.has(id) ? [...selected].filter(other => other !== id) : [...selected, id]) : [id];
    PM.Captions?.select?.(layer.id, next);
    PM.setTime?.(layer.from + entry.cue.start);
    reveal(position);
  }

  let editing = $state<string | null>(null);
  let draft = $state('');
  let editor: HTMLTextAreaElement | undefined = $state();
  async function edit(position: number) {
    const entry = shown[position];
    if (!entry || !layer || layer.lock) return;
    editing = entry.cue.id;
    draft = entry.cue.text;
    await tick();
    editor?.focus();
    editor?.select();
  }
  function commit() {
    const id = editing;
    if (!id || !layer) { editing = null; return; }
    editing = null;
    const cue = cues.find(item => item.id === id);
    if (!cue || draft === cue.text) return;
    PM.Edit.apply({ type: 'edit_captions', target: layer.id, op: 'update', cues: [{ id, text: draft }] }, { label: 'Edit caption', origin: 'interface' });
    listEl?.focus();
  }
  function cancel() { editing = null; listEl?.focus(); }

  function onEditorKey(event: KeyboardEvent) {
    event.stopPropagation();
    if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); commit(); }
    else if (event.key === 'Escape') { event.preventDefault(); cancel(); }
  }

  function onListKey(event: KeyboardEvent) {
    if (editing) return;
    const position = focusIndex >= 0 ? focusIndex : shown.findIndex(entry => selected.has(entry.cue.id));
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault(); event.stopPropagation();
      const next = Math.max(0, Math.min(shown.length - 1, (position < 0 ? -1 : position) + (event.key === 'ArrowDown' ? 1 : -1)));
      choose(next);
    } else if (event.key === 'Enter' && position >= 0) {
      event.preventDefault(); event.stopPropagation();
      void edit(position);
    } else if ((event.key === 'Backspace' || event.key === 'Delete') && selected.size) {
      event.preventDefault(); event.stopPropagation();
      PM.Captions?.deleteSelectedCues?.();
    }
  }

  function onSearchKey(event: KeyboardEvent) {
    event.stopPropagation();
    if (event.key === 'Escape') { query = ''; (event.currentTarget as HTMLInputElement).blur(); }
    if (event.key === 'ArrowDown' && shown.length) { event.preventDefault(); listEl?.focus(); choose(0); }
  }

  function openLayerMenu(event: MouseEvent) {
    openPopoverMenu({
      anchor: event.currentTarget as HTMLElement,
      label: 'Captions layer',
      items: layers.map(item => ({ label: item.name, checked: item.id === layer?.id, run: () => { chosen = item.id; PM.selectLayers?.([item.id]); } }))
    });
  }
</script>

<div class="captions-panel" data-svelte-panel={panelId}>
  {#if layer}
    <div class="captions-head">
      {#if layers.length > 1}
        <button type="button" class="captions-layer" aria-haspopup="menu" onclick={openLayerMenu}>
          <span>{layer.name}</span>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 6.5 8 10l3.5-3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </button>
      {/if}
      <input class="captions-search" type="search" placeholder="Search captions" aria-label="Search captions"
        bind:value={query} onkeydown={onSearchKey} />
    </div>
    <div class="captions-scroll" bind:this={scroller} onscroll={() => { scrollTop = scroller?.scrollTop ?? 0; }}>
      <div class="captions-list" role="listbox" aria-label="Captions" aria-multiselectable="true" tabindex="0"
        bind:this={listEl} onkeydown={onListKey} style={`height:${shown.length * ROW}px`}>
        {#each shown.slice(first, last) as entry, offset (entry.cue.id)}
          {@const position = first + offset}
          <!-- svelte-ignore a11y_click_events_have_key_events (the listbox owns the keyboard) -->
          <div class="captions-row" role="option" aria-selected={selected.has(entry.cue.id)} tabindex="-1"
            class:picked={selected.has(entry.cue.id)} class:now={entry.index === playing} class:focus={position === focusIndex}
            style={`top:${position * ROW}px`} data-cue-id={entry.cue.id}
            onclick={(event) => { if (editing !== entry.cue.id) choose(position, event); }}
            ondblclick={() => edit(position)}>
            <span class="tc">{timecode(layer.from + entry.cue.start)}</span>
            {#if editing === entry.cue.id}
              <textarea class="captions-editor" bind:this={editor} bind:value={draft} rows="1" aria-label="Caption text"
                onkeydown={onEditorKey} onblur={commit}></textarea>
            {:else}
              <span class="tx">{entry.cue.text.replace(/\n/g, ' ')}</span>
            {/if}
          </div>
        {/each}
      </div>
      {#if !shown.length}
        <div class="empty">{query ? `No captions match “${query}”` : 'This captions layer is empty.'}</div>
      {/if}
    </div>
  {:else}
    <div class="empty captions-empty">
      <p>Import an SRT or WebVTT file, or select a clip and generate captions from its speech.</p>
      <div class="captions-empty-actions">
        <button class="chip" type="button" onclick={() => PM.cmd?.('importCaptions')}>Import…</button>
        <button class="chip" type="button" onclick={() => PM.cmd?.('generateCaptions')}>Generate</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .captions-panel { height: 100%; min-height: 0; display: flex; flex-direction: column; font-size: var(--fs-sm); }
  .captions-head { flex: none; display: flex; gap: 6px; align-items: center; padding: 8px var(--panel-edge-inset, 8px) 4px; }
  .captions-layer {
    flex: none; max-width: 45%; height: 26px; display: inline-flex; align-items: center; gap: 4px; padding: 0 6px 0 8px;
    border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-2); font: inherit; cursor: default;
  }
  .captions-layer span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .captions-layer svg { width: 12px; height: 12px; flex: none; color: var(--tx-3); }
  .captions-layer:hover { color: var(--tx); background: var(--ink-1); }
  .captions-layer:focus-visible { outline: 0; box-shadow: 0 0 0 2px var(--ink-2); }
  .captions-search {
    flex: 1; min-width: 0; height: 26px; padding: 0 9px; border: 0; border-radius: var(--r-sm);
    background: var(--bg-field); color: var(--tx); font-size: var(--fs-sm); box-shadow: var(--ctl-edge-inset);
  }
  .captions-search::placeholder { color: var(--tx-4); }
  .captions-search:focus { outline: 0; }
  .captions-search:focus-visible { box-shadow: var(--ctl-edge-inset), 0 0 0 2px var(--ink-2); }
  .captions-search::-webkit-search-cancel-button { -webkit-appearance: none; }
  .captions-scroll { flex: 1; min-height: 0; overflow: auto; padding: 2px var(--panel-edge-inset, 8px) 8px; }
  .captions-list { position: relative; outline: 0; }
  .captions-row {
    position: absolute; left: 0; right: 0; height: 28px; display: flex; align-items: center; gap: 12px; padding: 0 8px;
    border-radius: var(--r-sm); color: var(--tx-2); cursor: default; user-select: none; -webkit-user-select: none;
  }
  .captions-row:hover { background: var(--ink-1); }
  .captions-row.picked { background: var(--bg-row-hi, var(--ink-2)); color: var(--tx); }
  .captions-row.now .tx { color: var(--tx); }
  .captions-list:focus-visible .captions-row.focus { box-shadow: inset 0 0 0 1px var(--line-strong); }
  .tc { flex: none; width: 74px; color: var(--tx-4); font-family: var(--f-mono); font-size: var(--fs-xs); font-variant-numeric: tabular-nums; }
  .captions-row.now .tc { color: var(--tx-3); }
  .tx { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .captions-editor {
    flex: 1; min-width: 0; height: 22px; padding: 2px 6px; resize: none; border: 0; border-radius: var(--r-xs, 4px);
    background: var(--bg-field); color: var(--tx); font: inherit; line-height: 18px; box-shadow: var(--ctl-edge-inset);
    user-select: text; -webkit-user-select: text; overflow: hidden;
  }
  .captions-editor:focus { outline: 0; }
  .empty { padding: 22px 12px; color: var(--tx-3); font-size: var(--fs-sm); line-height: var(--lh); text-align: center; text-wrap: pretty; }
  .captions-empty p { margin: 0 0 12px; }
  .captions-empty-actions { display: flex; gap: 6px; justify-content: center; }
  .captions-empty-actions .chip { height: var(--ctl-h); padding: 0 12px; }
</style>
