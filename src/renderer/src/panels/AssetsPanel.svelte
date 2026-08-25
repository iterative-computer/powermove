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

  $effect(() => {
    const host = rootElement.parentElement;
    host?.classList.add('assets-panel-body');
    return () => host?.classList.remove('assets-panel-body');
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
    if (asset.kind !== 'audio' && asset.w && asset.h) parts.push(`${asset.w}×${asset.h}`);
    if (asset.dur) parts.push(mediaDuration(asset.dur));
    if (asset.size) parts.push(mediaSize(asset.size));
    return parts.join(' · ') || 'Ready to use';
  }

  function assetIcon(kind: string): string {
    return kind === 'audio' ? 'clock' : kind === 'video' ? 'cam' : 'frame';
  }

  function liveAsset(asset: Asset): Record<string, any> | undefined {
    return PM.assets.get(asset.id);
  }

  function selectAsset(id: string, row?: HTMLElement): void {
    selectedAssetId = id;
    row?.focus();
    status = `Selected ${assets.find((asset) => asset.id === id)?.name ?? 'media'}`;
  }

  function addAsset(event: MouseEvent | KeyboardEvent, asset: Asset): void {
    event.stopPropagation();
    PM.cmd('addFromAsset', asset.id);
    status = `Added ${asset.name} to the timeline`;
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
    PM.modal({
      title: `Delete “${asset.name}”?`,
      width: 420,
      body: PM.h(
        'div',
        { style: { color: 'var(--tx-2)', fontSize: '12.5px', lineHeight: 1.6 } },
        `This also removes ${references} ${references === 1 ? 'layer that uses' : 'layers that use'} this media. You can undo this.`
      ),
      actions: [
        { label: 'Cancel' },
        { label: 'Delete', pri: true, run: () => deleteAsset(asset) }
      ]
    });
    status = `Confirm deletion of ${asset.name}`;
  }

  function handleRowClick(event: MouseEvent, asset: Asset): void {
    if ((event.target as Element).closest('button')) return;
    selectAsset(asset.id, event.currentTarget as HTMLElement);
  }

  function handleRowDoubleClick(event: MouseEvent, asset: Asset): void {
    if ((event.target as Element).closest('button')) return;
    PM.cmd('addFromAsset', asset.id);
    status = `Added ${asset.name} to the timeline`;
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
    }
  }

  function importMedia(): void {
    PM.pickFiles();
    status = 'Import media dialog opened';
  }
</script>

<div class="assets-panel-body" style="height:100%" data-svelte-panel={panelId} bind:this={rootElement}>
  <div class="assets-top">
    <button class="asset-import" type="button" title="Import media (⌘I)" onclick={importMedia}>
      <Icon {PM} name="plus" />
      <span>Import</span>
    </button>
  </div>
  <div class="asset-list" role="listbox" aria-label="Project media" bind:this={listElement}>
    {#each assets as asset, index (asset.id)}
      {@const currentAsset = liveAsset(asset)}
      <div
        class="asset-card"
        role="option"
        tabindex={activeAssetId ? (activeAssetId === asset.id ? 0 : -1) : (index === 0 ? 0 : -1)}
        aria-selected={activeAssetId === asset.id}
        data-asset-id={asset.id}
        title="Select media · double-click to add to the timeline"
        onclick={(event) => handleRowClick(event, asset)}
        ondblclick={(event) => handleRowDoubleClick(event, asset)}
        onkeydown={(event) => handleRowKeydown(event, asset, index)}
      >
        <span class={`asset-preview ${asset.kind}`}>
          <Icon {PM} name={assetIcon(asset.kind)} />
          {#if asset.kind === 'image' && currentAsset?.url}
            <img src={currentAsset.url} alt="" />
          {/if}
        </span>
        <span class="asset-copy">
          <b>{asset.name}</b>
          <small>{mediaDetails(asset)}</small>
        </span>
        <span class="asset-actions">
          <button class="asset-add" type="button" title="Add to timeline" aria-label={`Add ${asset.name} to timeline`} onclick={(event) => addAsset(event, asset)}>
            <Icon {PM} name="plus" />
          </button>
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
      </div>
    {:else}
      <div class="asset-empty">
        <Icon {PM} name="project" />
        <b>No imported media</b>
        <span>Images, video, and audio stay with this project.</span>
      </div>
    {/each}
  </div>
  <span class="panel-sr-only" role="status">{status}</span>
</div>
