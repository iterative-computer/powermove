<script lang="ts">
  import { doc } from '../state/document.svelte';
  import Icon from './Icon.svelte';
  import type { PanelProps } from './registerSveltePanel';

  interface Asset {
    id: string;
    name: string;
    kind: 'audio' | 'video' | 'image' | string;
    w?: number;
    h?: number;
    dur?: number;
    size?: number;
    vertices?: number;
    triangles?: number;
    format?: string;
    sourcePath?: string;
    path?: string;
  }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  const assets = $derived((doc.tick.assets, doc.proj, Object.values(doc.proj?.assets ?? {}) as Asset[]));
  let selectedAssetId = $state<string | null>(null);
  const activeAssetId = $derived(
    selectedAssetId && assets.some((asset) => asset.id === selectedAssetId) ? selectedAssetId : null
  );
  let listElement: HTMLElement;
  let rootElement: HTMLElement;
  let status = $state('');
  let observedProject = doc.proj;

  $effect(() => {
    const host = rootElement.parentElement;
    host?.classList.add('assets-panel-body');
    window.addEventListener('pointerdown', handleWindowPointerDown, true);
    return () => {
      host?.classList.remove('assets-panel-body');
      window.removeEventListener('pointerdown', handleWindowPointerDown, true);
      PM.Kernel?.services.get('viewer')?.preview?.clear?.();
    };
  });

  /* Project tabs can also change through the keyboard or commands, without a
     pointer event. A source preview belongs only to the project that selected
     it, so a document swap always releases it. */
  $effect(() => {
    const project = doc.proj;
    if (project !== observedProject) {
      observedProject = project;
      clearSelection();
    }
  });

  function mediaDuration(seconds?: number): string {
    if (!(Number(seconds) > 0)) return '';
    const whole = Math.round(Number(seconds));
    const minutes = Math.floor(whole / 60);
    return `${minutes}:${String(whole % 60).padStart(2, '0')}`;
  }

  function mediaSize(bytes?: number): string {
    const value = Number(bytes) || 0;
    if (!value) return '';
    if (value < 1024 * 1024) return `${Math.max(1, Math.round(value / 1024))} KB`;
    return `${(value / (1024 * 1024)).toFixed(value < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  }

  function mediaDetails(asset: Asset): string {
    const parts: string[] = [];
    if (asset.format === 'svg' || /\.svg$/i.test(asset.name)) parts.push('SVG');
    // An animated image becomes a clip, so name the container it came from.
    else if (asset.format && asset.kind !== 'model') parts.push(asset.format.toUpperCase());
    if (asset.kind !== 'audio' && asset.w && asset.h) parts.push(`${asset.w}×${asset.h}`);
    if (asset.dur) parts.push(mediaDuration(asset.dur));
    if (asset.kind === 'model' && asset.triangles) parts.push(`${asset.triangles.toLocaleString()} tris`);
    if (asset.size) parts.push(mediaSize(asset.size));
    return parts.join(' · ') || 'Ready to use';
  }

  function assetIcon(kind: string): string {
    return kind === 'audio' ? 'music' : kind === 'video' ? 'film' : kind === 'model' ? 'grid' : 'image';
  }

  function liveAsset(asset: Asset): Record<string, any> | undefined {
    return PM.assets.get(asset.id);
  }

  /* A cached still captured at import. It survives the source bytes going
     missing, so an offline card can still show what the media looked like. */
  function posterUrl(asset: Asset): string {
    return String(PM.assets.poster?.(asset.id) || '');
  }

  function isLoading(asset: Asset): boolean {
    return !!PM.assets.loading?.has(asset.id);
  }

  /* Only completed restoration can establish that media is offline. */
  function isOffline(asset: Asset): boolean {
    return !liveAsset(asset) && !isLoading(asset);
  }

  function offlineDetail(asset: Asset): string {
    const source = finderPath(asset);
    return source ? `Missing · ${source}` : 'Missing · locate the file to relink it';
  }

  function finderPath(asset: Asset): string {
    return asset.sourcePath || asset.path || '';
  }

  function showAssetMenu(event: MouseEvent, asset: Asset): void {
    event.preventDefault();
    event.stopPropagation();
    selectAsset(asset.id, event.currentTarget as HTMLElement);
    const sourcePath = finderPath(asset);
    if (isLoading(asset)) return;
    const offline = isOffline(asset);
    PM.menu(event.currentTarget, [
      { header: asset.name },
      ...(offline ? [] : [{ label: 'Add to timeline', run: () => PM.cmd('addFromAsset', asset.id) }]),
      { label: offline ? 'Locate File…' : 'Replace File…', run: () => PM.pickFiles(false, { replaceAssetId: asset.id }) },
      ...(sourcePath ? [{ label: 'Reveal in Finder', run: () => revealAssetSource(asset) }] : []),
      '-',
      { label: 'Delete media…', run: () => requestDelete(asset) },
    ], { x: event.clientX, y: event.clientY });
    status = `Actions for ${asset.name}`;
  }

  function videoSource(asset?: Record<string, any>): string {
    return String(asset?.url || asset?.el?.currentSrc || asset?.el?.src || '');
  }

  /* Reveal a video thumbnail only after Chromium has decoded a real frame.
     Seeking on metadata alone can leave the element permanently black on some
     codecs, which made the generic film icon look like the final thumbnail. */
  function poster(video: HTMLVideoElement) {
    const reveal = () => { video.dataset.ready = 'true'; };
    const seek = () => {
      const duration = Number(video.duration);
      if (!(Number.isFinite(duration) && duration > 0)) return;
      const target = Math.min(0.5, duration * 0.1);
      if (Math.abs(video.currentTime - target) < .01 && video.readyState >= 2) reveal();
      else {
        try { video.currentTime = target; }
        catch { /* The loadeddata listener gets another chance. */ }
      }
    };
    const onFrame = () => {
      if (video.currentTime > 0 || !(Number(video.duration) > 0)) reveal();
      else seek();
    };
    video.addEventListener('loadedmetadata', seek);
    video.addEventListener('loadeddata', seek);
    video.addEventListener('seeked', onFrame);
    video.addEventListener('canplay', onFrame);
    if (video.readyState >= 2) onFrame();
    else if (video.readyState >= 1) seek();
    try { video.load(); } catch { }
    return {
      destroy() {
        video.removeEventListener('loadedmetadata', seek);
        video.removeEventListener('loadeddata', seek);
        video.removeEventListener('seeked', onFrame);
        video.removeEventListener('canplay', onFrame);
      }
    };
  }

  /* Audio thumbnails are a real waveform, matching the timeline clip. One
     bar per pixel or so; each bar is the mean of the peaks under it, not the
     max — a max over hundreds of peaks is ~1.0 for any music and draws a
     wall. */
  function waveBins(asset: Asset, bins = 120): number[] {
    const peaks: ArrayLike<number> | undefined = liveAsset(asset)?.peaks;
    if (!peaks || !peaks.length) return Array.from({ length: bins }, (_, i) => 18 + 14 * Math.abs(Math.sin(i * .7)));
    const out: number[] = [];
    for (let b = 0; b < bins; b++) {
      const from = Math.floor(b * peaks.length / bins), to = Math.max(from + 1, Math.floor((b + 1) * peaks.length / bins));
      let sum = 0;
      for (let i = from; i < to; i++) sum += peaks[i] || 0;
      out.push(Math.min(98, Math.max(3, (sum / (to - from)) * 100)));
    }
    return out;
  }

  function selectAsset(id: string, row?: HTMLElement): void {
    selectedAssetId = id;
    row?.focus();
    status = `Selected ${assets.find((asset) => asset.id === id)?.name ?? 'media'}`;
  }

  function addAsset(event: MouseEvent | KeyboardEvent, asset: Asset): void {
    event.stopPropagation();
    if (isLoading(asset)) return;
    /* Offline media has nothing to put on the timeline; the primary action
       becomes locating the file, the same way a broken tile works in an NLE. */
    if (isOffline(asset)) {
      selectAsset(asset.id);
      PM.pickFiles(false, { replaceAssetId: asset.id });
      return;
    }
    PM.cmd('addFromAsset', asset.id);
    status = `Added ${asset.name} to the timeline`;
  }

  async function revealAsset(event: MouseEvent, asset: Asset): Promise<void> {
    event.stopPropagation();
    selectAsset(asset.id);
    await revealAssetSource(asset);
  }

  async function revealAssetSource(asset: Asset): Promise<void> {
    const sourcePath = finderPath(asset);
    if (!sourcePath) return;
    try {
      await window.powermove.media.revealSource(sourcePath);
      status = `Revealed ${asset.name} in Finder`;
    } catch {
      status = `Could not reveal ${asset.name} in Finder`;
      PM.toast(`Could not reveal ${asset.name} in Finder`);
    }
  }

  function deleteAsset(asset: Asset): void {
    const result = PM.hist.do('Delete media', () => PM.MediaImport.removeAsset(PM.proj, asset.id));
    selectedAssetId = null;
    PM.sel.layers = PM.sel.layers.filter((id: string) => !result.removedLayerIds.includes(id));
    PM.bus.emit('assets');
    PM.bus.emit('layers');
    PM.bus.emit('sel');
    PM.bus.emit('project');
    const message = result.removedLayers
      ? `Deleted ${asset.name} and ${result.removedLayers} ${result.removedLayers === 1 ? 'layer' : 'layers'}`
      : `Deleted ${asset.name}`;
    status = message;
    PM.toast(message);
  }

  function requestDelete(asset: Asset): void {
    const references = PM.MediaImport.referenceCount(PM.proj, asset.id);
    if (!references) {
      deleteAsset(asset);
      return;
    }
    void PM.confirm({
      message: `Delete “${asset.name}”?`,
      detail: `This also removes ${references} ${references === 1 ? 'layer that uses' : 'layers that use'} this media. You can undo this.`,
      confirmLabel: 'Delete'
    }).then((ok: boolean) => { if (ok) deleteAsset(asset); });
    status = `Confirm deletion of ${asset.name}`;
  }

  /* ── drag out: asset cards → timeline / viewer ── */
  let draggingId = $state<string | null>(null);
  function handleDragStart(event: DragEvent, asset: Asset): void {
    if ((event.target as Element).closest('button')) { event.preventDefault(); return; }
    const dt = event.dataTransfer;
    if (!dt) return;
    dt.setData('application/x-powermove-asset', JSON.stringify({ id: asset.id, name: asset.name, kind: asset.kind, dur: asset.dur }));
    dt.effectAllowed = 'copy';
    /* The ghost is just the tile, not the whole card: it reads as the thing
       you'll get on the timeline. */
    const tile = (event.currentTarget as HTMLElement).querySelector<HTMLElement>('.asset-preview');
    if (tile) dt.setDragImage(tile, tile.offsetWidth / 2, tile.offsetHeight / 2);
    draggingId = asset.id;
    selectedAssetId = asset.id;
    /* dragover can't read payload data, so the timeline ghost reads this. */
    PM.mediaDrag = { id: asset.id, name: asset.name, kind: asset.kind, dur: asset.dur };
  }
  function handleDragEnd(): void { draggingId = null; PM.mediaDrag = null; }

  /* ── drop in: OS files → project media only, no layer ── */
  let dropDepth = 0;
  let dropOver = $state(false);
  function fileDrag(event: DragEvent): boolean {
    return Array.from(event.dataTransfer?.types ?? []).includes('Files');
  }
  function handleDragEnter(event: DragEvent): void {
    if (!fileDrag(event)) return;
    event.preventDefault();
    dropDepth++;
    dropOver = true;
  }
  function handleDragOver(event: DragEvent): void {
    if (!fileDrag(event)) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
  }
  function handleDragLeave(event: DragEvent): void {
    if (!fileDrag(event)) return;
    dropDepth = Math.max(0, dropDepth - 1);
    if (!dropDepth) dropOver = false;
  }
  function handleDrop(event: DragEvent): void {
    dropDepth = 0;
    dropOver = false;
    if (!fileDrag(event)) return;
    event.preventDefault();
    event.stopPropagation();
    const files = Array.from(event.dataTransfer?.files ?? []);
    if (!files.length) return;
    PM.importFiles(files, { placement: null });
    status = `Importing ${files.length === 1 ? files[0]?.name : `${files.length} files`}`;
  }

  /* Select on pointerdown so a press anywhere on the tile (image, video,
     waveform) selects, not only on the card's own box. */
  function handleRowPointerDown(event: PointerEvent, asset: Asset): void {
    if (event.button !== 0 || (event.target as Element).closest('button')) return;
    selectAsset(asset.id, event.currentTarget as HTMLElement);
  }

  function clearSelection(): void {
    const preview = PM.Kernel?.services.get('viewer')?.preview;
    const shouldClearPreview = selectedAssetId !== null || !!preview?.activeId;
    selectedAssetId = null;
    status = '';
    if (shouldClearPreview) preview?.clear?.();
    (document.activeElement as HTMLElement | null)?.blur?.();
  }

  /* Selection is temporary ownership: preserve it only while the next press
     is inside the currently selected card. Capture runs before timeline drags
     and titlebar handlers, so the preview disappears at pointerdown.
     The source monitor itself is part of that ownership — its transport and
     close button must survive the press that reaches them. */
  function handleWindowPointerDown(event: PointerEvent): void {
    if (!selectedAssetId) return;
    const target = event.target;
    if (!(target instanceof Element)) { clearSelection(); return; }
    if (target.closest('#source-preview')) return;
    const card = target.closest<HTMLElement>('.asset-card[data-asset-id]');
    if (card?.dataset.assetId === selectedAssetId) return;
    clearSelection();
  }

  /* A press on empty list space unfocuses, like clicking the desktop. */
  function handleListPointerDown(event: PointerEvent): void {
    if ((event.target as Element).closest('.asset-card')) return;
    clearSelection();
  }

  function previewToggle(asset: Asset): void {
    if (asset.kind === 'model') { status = `${asset.name} is a 3D model`; return; }
    const preview = PM.Kernel?.services.get('viewer')?.preview;
    if (!preview) { status = 'Source preview is unavailable'; return; }
    preview.toggle(asset.id);
    status = preview.playing ? `Playing ${asset.name}` : `Paused ${asset.name}`;
  }

  function handleRowDoubleClick(event: MouseEvent, asset: Asset): void {
    if (isLoading(asset)) return;
    if ((event.target as Element).closest('button')) return;
    event.preventDefault();
    event.stopPropagation();
    selectAsset(asset.id, event.currentTarget as HTMLElement);
    if (isOffline(asset)) {
      PM.pickFiles(false, { replaceAssetId: asset.id });
      return;
    }
    status = PM.Kernel?.services.get('viewer')?.preview?.show(asset.id)
      ? `Previewing ${asset.name}` : `Preview unavailable for ${asset.name}`;
  }

  function focusAsset(index: number): void {
    const options = listElement.querySelectorAll<HTMLElement>('.asset-card');
    options[index]?.focus();
  }

  function moveSelection(event: KeyboardEvent, index: number): boolean {
    const last = assets.length - 1;
    let next = index;
    if (event.key === 'ArrowDown' || event.key === 'ArrowRight') next = index === last ? 0 : index + 1;
    else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') next = index === 0 ? last : index - 1;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = last;
    else return false;
    event.preventDefault();
    event.stopPropagation();
    const asset = assets[next];
    if (!asset) return false;
    selectAsset(asset.id);
    focusAsset(next);
    return true;
  }

  function handleRowKeydown(event: KeyboardEvent, asset: Asset, index: number): void {
    if (moveSelection(event, index)) return;
    if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault();
      event.stopPropagation();
      requestDelete(asset);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      addAsset(event, asset);
    } else if (event.key === ' ') {
      event.preventDefault();
      event.stopPropagation();
      selectAsset(asset.id, event.currentTarget as HTMLElement);
      previewToggle(asset);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      clearSelection();
    }
  }

</script>

<div
  class="assets-panel-body"
  class:is-drop-over={dropOver}
  role="region"
  aria-label="Media"
  style="height:100%"
  data-svelte-panel={panelId}
  bind:this={rootElement}
  ondragenter={handleDragEnter}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  <div class="asset-list" role="listbox" tabindex="-1" aria-label="Project media" bind:this={listElement} onpointerdown={handleListPointerDown}>
    {#each assets as asset, index (asset.id)}
      {@const currentAsset = (doc.tick.assets, liveAsset(asset))}
      {@const posterSrc = (doc.tick.assets, posterUrl(asset))}
      {@const loading = (doc.tick.assets, isLoading(asset))}
      {@const offline = !currentAsset && !loading}
      <div
        class="asset-card"
        class:is-dragging={draggingId === asset.id}
        class:is-offline={offline}
        role="option"
        aria-busy={loading}
        draggable={!offline && !loading}
        ondragstart={(event) => handleDragStart(event, asset)}
        ondragend={handleDragEnd}
        tabindex={activeAssetId ? (activeAssetId === asset.id ? 0 : -1) : (index === 0 ? 0 : -1)}
        aria-selected={activeAssetId === asset.id}
        data-asset-id={asset.id}
        title={loading ? 'Loading media…' : offline ? offlineDetail(asset) : 'Select media · double-click to preview, or drag onto the timeline'}
        onpointerdown={(event) => handleRowPointerDown(event, asset)}
        oncontextmenu={(event) => showAssetMenu(event, asset)}
        ondblclick={(event) => handleRowDoubleClick(event, asset)}
        onkeydown={(event) => handleRowKeydown(event, asset, index)}
      >
        <span class={`asset-preview ${asset.kind}`}>
          {#if asset.kind === 'audio'}
            <span class="asset-wave" aria-hidden="true">
              {#each waveBins(asset) as h}<i style={`height:${h}%`}></i>{/each}
            </span>
          {:else}
            <Icon {PM} name={assetIcon(asset.kind)} />
            {#if posterSrc}
              <img class="asset-poster" src={posterSrc} alt="" />
            {:else if asset.kind === 'image' && currentAsset?.url}
              <img src={currentAsset.url} alt="" />
            {:else if asset.kind === 'video' && videoSource(currentAsset)}
              <video src={videoSource(currentAsset)} muted playsinline preload="auto" use:poster></video>
            {/if}
          {/if}
          {#if loading}
            <span class="asset-badge">Loading…</span>
          {:else if offline}
            <span class="asset-offline" role="img" aria-label="Media offline">
              <Icon {PM} name="missing" />
              Media offline
            </span>
          {:else if asset.dur}<span class="asset-badge">{mediaDuration(asset.dur)}</span>{/if}
          <span class="asset-actions">
            {#if finderPath(asset)}
              <button class="asset-reveal" type="button" title="Reveal in Finder" aria-label={`Reveal ${asset.name} in Finder`} onclick={(event) => revealAsset(event, asset)}>
                <Icon {PM} name="project" />
              </button>
            {/if}
            {#if offline}
              <button class="asset-locate" type="button" title="Locate file" aria-label={`Locate ${asset.name}`} onclick={(event) => addAsset(event, asset)}>
                <Icon {PM} name="link" />
              </button>
            {:else}
              <button class="asset-add" type="button" disabled={loading} title="Add to timeline" aria-label={`Add ${asset.name} to timeline`} onclick={(event) => addAsset(event, asset)}>
                <Icon {PM} name="plus" />
              </button>
            {/if}
            <button
              class="asset-delete"
              type="button"
              title="Delete media"
              aria-label={`Delete ${asset.name}`}
              onclick={(event) => {
                event.stopPropagation();
                selectAsset(asset.id);
                requestDelete(asset);
              }}
            >
              <Icon {PM} name="trash" />
            </button>
          </span>
        </span>
        <span class="asset-copy">
          <b title={asset.name}>{asset.name}</b>
          {#if loading}
            <small>Loading media…</small>
          {:else if offline}
            <small class="asset-missing" title={offlineDetail(asset)}>{offlineDetail(asset)}</small>
          {:else}
            <small title={mediaDetails(asset)}>{mediaDetails(asset)}</small>
          {/if}
        </span>
      </div>
    {:else}
      <div class="asset-empty">
        <Icon {PM} name="project" />
        <b>Add media</b>
        <span>Drag files here or import from your computer.</span>
      </div>
    {/each}
  </div>
  <div class="asset-drop-overlay" aria-hidden="true">
    <span class="asset-drop-card">
      <Icon {PM} name="plus" />
      <b>Add to project media</b>
      <small>Drops here don't touch the timeline</small>
    </span>
  </div>
  <span class="panel-sr-only" role="status">{status}</span>
</div>
