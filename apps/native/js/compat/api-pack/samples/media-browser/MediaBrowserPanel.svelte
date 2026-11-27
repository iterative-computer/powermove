<script lang="ts">
  import type { AssetDragPayload, AssetRecord, PowermoveAPI } from 'powermove';

  type MediaItem = { id: string; name: string; url: string };
  type MediaSource = () => Promise<readonly MediaItem[]>;

  let { api, source }: { api: PowermoveAPI; source: MediaSource } = $props();
  let items = $state<readonly MediaItem[]>([]);
  let imported = $state<Record<string, AssetRecord>>({});
  let busy = $state<string | null>(null);
  let message = $state('Import an image, then drag it to the timeline.');

  // A source can be an SDK, local index, or HTTP catalogue. Cancellation keeps
  // a late response from updating a panel that has already been disposed.
  $effect(() => {
    let current = true;
    source().then(
      (next) => { if (current) items = next; },
      (error) => {
        if (!current) return;
        message = error instanceof Error ? error.message : 'Could not load media.';
      }
    );
    return () => { current = false; };
  });

  async function importItem(item: MediaItem): Promise<void> {
    busy = item.id;
    message = `Importing ${item.name}`;
    try {
      // `File` is a named Blob, which is exactly what the durable asset API
      // accepts. The returned id is the only value placed in the project/drop.
      const response = await fetch(item.url);
      if (!response.ok) throw new Error(`Source returned ${response.status}`);
      const blob = await response.blob();
      const file = new File([blob], item.name, { type: blob.type || 'image/png' });
      const asset = await api.assets.import(file);
      imported = { ...imported, [item.id]: asset };
      message = `${item.name} is ready to drag.`;
    } catch (error) {
      message = error instanceof Error ? error.message : `Could not import ${item.name}.`;
      api.ui.toast(message);
    } finally {
      busy = null;
    }
  }

  function startDrag(event: DragEvent, item: MediaItem): void {
    const asset = imported[item.id];
    if (!asset) { event.preventDefault(); return; }
    const payload: AssetDragPayload = {
      id: asset.id,
      name: asset.name,
      kind: asset.kind,
      dur: asset.dur
    };
    // The MIME payload is read on drop; mediaDrag supplies the timeline's
    // preview because browsers hide DataTransfer contents during dragover.
    api.dnd.mediaDrag = payload;
    api.dnd.startAssetDrag(event.dataTransfer, payload);
  }

  function endDrag(): void {
    api.dnd.mediaDrag = null;
  }

  function undo(): void {
    // Timeline drops are undoable edits. Imported media remains in the asset
    // library, so undo removes the placed layer without refetching the source.
    api.project.undo();
    message = 'Undid the last project edit.';
  }
</script>

<div class="browser" aria-busy={busy !== null}>
  <p class="help pm-caption">Choose an image to make it available to the timeline.</p>
  <ul class="pm-list" aria-label="Available images">
    {#each items as item (item.id)}
      <li class="pm-list-item"
        draggable={Boolean(imported[item.id])}
        ondragstart={(event) => startDrag(event, item)}
        ondragend={endDrag}
      >
        <span>{item.name}</span>
        {#if imported[item.id]}
          <span class="ready pm-caption">Ready to drag</span>
        {:else}
          <button class="pm-button" type="button" disabled={busy !== null} onclick={() => importItem(item)}>
            {busy === item.id ? 'Importing' : 'Import'}
          </button>
        {/if}
      </li>
    {/each}
  </ul>
  <button type="button" class="undo pm-button pm-button--quiet" onclick={undo}>Undo last timeline edit</button>
  <p class="status pm-caption" role="status" aria-live="polite">{message}</p>
</div>

<style>
  .browser { display:flex; flex-direction:column; gap:var(--pm-gap); padding:var(--pm-inset); }
  .help,.status { margin:0; }
  ul { display:flex; flex-direction:column; gap:var(--pm-space-xs); margin:0; padding:0; list-style:none; }
  li { display:flex; align-items:center; gap:var(--pm-gap); padding:var(--pm-space-xs) var(--pm-gap); }
  li[draggable="true"] { cursor:grab; }
  li[draggable="true"]:active { cursor:grabbing; }
  li > span:first-child { min-width:0; flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .ready { flex:none; }
  .undo { align-self:flex-start; }
</style>
