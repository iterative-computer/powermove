<script lang="ts">
  import { doc } from '../state/document.svelte';
  import Icon from './Icon.svelte';
  import type { PanelProps } from './registerSveltePanel';
  import { bridge } from '../kernel/bridge';
  import { createCompThumbnails } from './comp-thumbnails.svelte';
  import {
    assetFolder, childFolders, createFolder, deleteFolder, folderItemCount, folderPath, isWithin, moveItems,
    renameFolder, validFolder, type AssetFolder
  } from '../legacy/core/asset-folders';

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

  interface CompItem {
    id: string;
    name: string;
    w: number;
    h: number;
    fps: number;
    dur: number;
    bg: string;
    open: boolean;
    uses: number;
  }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  /* Compositions are project items, listed with the footage like AE's Project panel. */
  const comps = $derived((doc.tick.project, doc.tick.structure, doc.tick.history, doc.proj,
    (PM.Comps?.list?.() ?? []) as CompItem[]));
  let selectedCompId = $state<string | null>(null);
  const compThumbs = createCompThumbnails(PM);
  let draggingCompId = $state<string | null>(null);
  /* Media folders work like Finder's icon view: the grid shows one folder's
     contents, and the path above it leads back out. Undo can delete the open
     folder, so every read goes through validFolder. */
  let openFolderId = $state<string | null>(null);
  const folderView = $derived.by(() => {
    // Folder edits happen in place, so the ticks are what re-run this.
    const version = doc.tick.assets + doc.tick.history + doc.tick.project;
    const project = doc.proj ?? {};
    const current = validFolder(project, openFolderId);
    const folders = childFolders(project, current);
    return {
      version, current, folders,
      crumbs: folderPath(project, current),
      counts: new Map(folders.map((folder) => [folder.id, folderItemCount(project, folder.id)]))
    };
  });
  const currentFolder = $derived(folderView.current);
  const folders = $derived(folderView.folders);
  const crumbs = $derived(folderView.crumbs);
  const allAssets = $derived((doc.tick.assets, doc.tick.history, doc.proj, Object.values(doc.proj?.assets ?? {}) as Asset[]));
  const assets = $derived(allAssets.filter((asset) => assetFolder(doc.proj ?? {}, asset) === currentFolder));
  let selectedAssetId = $state<string | null>(null);
  let selectedFolderId = $state<string | null>(null);
  let renamingFolderId = $state<string | null>(null);
  let draggingFolderId = $state<string | null>(null);
  let folderDropTarget = $state<string | null>(null);
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

  /* Re-render composition thumbnails once the project settles after any change. */
  $effect(() => {
    const { values, structure, project, assets: media, history } = doc.tick;
    compThumbs.refresh(`${doc.generation}:${values}:${structure}:${project}:${media}:${history}`);
  });
  $effect(() => () => compThumbs.dispose());

  /* Project tabs can also change through the keyboard or commands, without a
     pointer event. A source preview belongs only to the project that selected
     it, so a document swap always releases it. */
  $effect(() => {
    const project = doc.proj;
    if (project !== observedProject) {
      if (project?.id !== observedProject?.id) openFolderId = null;
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
    if (liveAsset(asset)) return false;
    return !!PM.assets.loading?.has(asset.id) || (PM.assetsRestoring || 0) > 0 || PM.MediaStore?.pending?.(asset) === true;
  }

  /* Only completed restoration can establish that media is offline. */
  function isOffline(asset: Asset): boolean {
    return !liveAsset(asset) && !isLoading(asset);
  }

  function offlineDetail(asset: Asset): string {
    const source = finderPath(asset);
    const cloud = PM.assets.cloud?.get(asset.id);
    if (cloud) return cloud.state === 'downloading' ? `Downloading from ${cloud.provider}…`
      : cloud.state === 'error' ? `Download failed · ${cloud.error}` : `Stored in ${cloud.provider} · ${source}`;
    const error = PM.assets.errors?.get(asset.id);
    if (error) return `Could not load · ${error}`;
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
      ...(offline && PM.assets.cloud?.get(asset.id) ? [{ label: 'Download from cloud', run: () => PM.assets.cloud.download(asset.id) }] : []),
      ...(offline ? [] : [
        { label: 'Add to timeline', run: () => PM.cmd('addFromAsset', asset.id) },
        ...(asset.kind === 'audio' || asset.kind === 'model' ? [] : [{ label: 'New Comp from Selection', run: () => PM.cmd('newCompFromMedia', asset.id) }]),
      ]),
      { label: offline ? 'Locate File…' : 'Replace File…', run: () => PM.pickFiles(false, { replaceAssetId: asset.id }) },
      ...(currentFolder ? [{ label: 'Move Out of Folder', run: () => moveInto({ assets: [asset.id] }, crumbs.at(-2)?.id ?? null) }] : []),
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
    selectedCompId = null;
    selectedFolderId = null;
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
      if (PM.assets.cloud?.get(asset.id)) { void PM.assets.cloud.download(asset.id); return; }
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
      await bridge()!.media.revealSource(sourcePath);
      status = `Revealed ${asset.name} in Finder`;
    } catch {
      status = `Could not reveal ${asset.name} in Finder`;
      PM.toast(`Could not reveal ${asset.name} in Finder`);
    }
  }

  function deleteAsset(asset: Asset, project = PM.proj): void {
    if (PM.proj !== project) {
      status = 'Deletion stopped because you switched projects';
      PM.toast(status);
      return;
    }
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
    const project = PM.proj;
    const references = PM.MediaImport.referenceCount(project, asset.id);
    if (!references) {
      deleteAsset(asset);
      return;
    }
    void PM.confirm({
      message: `Delete “${asset.name}”?`,
      detail: `This also removes ${references} ${references === 1 ? 'layer that uses' : 'layers that use'} this media. You can undo this.`,
      confirmLabel: 'Delete'
    }).then((ok: boolean) => { if (ok) deleteAsset(asset, project); });
    status = `Confirm deletion of ${asset.name}`;
  }

  /* ── drag out: asset cards → timeline / viewer ── */
  let draggingId = $state<string | null>(null);
  function handleDragStart(event: DragEvent, asset: Asset): void {
    if ((event.target as Element).closest('button')) { event.preventDefault(); return; }
    const dt = event.dataTransfer;
    if (!dt) return;
    dt.setData('application/x-powermove-asset', JSON.stringify({ id: asset.id, name: asset.name, kind: asset.kind, dur: asset.dur }));
    // Copy onto the timeline, move into a media folder.
    dt.effectAllowed = 'copyMove';
    /* The ghost is just the tile, not the whole card: it reads as the thing
       you'll get on the timeline. */
    const tile = (event.currentTarget as HTMLElement).querySelector<HTMLElement>('.asset-preview');
    if (tile) dt.setDragImage(tile, tile.offsetWidth / 2, tile.offsetHeight / 2);
    draggingId = asset.id;
    selectedAssetId = asset.id;
    /* dragover can't read payload data, so the timeline ghost reads this. */
    PM.mediaDrag = { id: asset.id, name: asset.name, kind: asset.kind, dur: asset.dur };
  }
  function handleDragEnd(): void { draggingId = null; folderDropTarget = null; PM.mediaDrag = null; }

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
    importDrop(event, currentFolder);
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
    selectedCompId = null;
    selectedFolderId = null;
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
    if (selectedFolderId) {
      const card = event.target instanceof Element ? event.target.closest<HTMLElement>('.asset-card[data-folder-id]') : null;
      if (card?.dataset.folderId !== selectedFolderId) selectedFolderId = null;
      return;
    }
    if (selectedCompId) {
      const card = event.target instanceof Element ? event.target.closest<HTMLElement>('.asset-card[data-comp-id]') : null;
      if (card?.dataset.compId !== selectedCompId) selectedCompId = null;
      return;
    }
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
      if (PM.assets.cloud?.get(asset.id)) { void PM.assets.cloud.download(asset.id); return; }
      PM.pickFiles(false, { replaceAssetId: asset.id });
      return;
    }
    status = PM.Kernel?.services.get('viewer')?.preview?.show(asset.id)
      ? `Previewing ${asset.name}` : `Preview unavailable for ${asset.name}`;
  }

  function focusAsset(index: number): void {
    const options = listElement.querySelectorAll<HTMLElement>('.asset-card[data-asset-id]');
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
    // Nested buttons own their keys, including native Enter/Space activation.
    if (event.target !== event.currentTarget) {
      if (!event.metaKey && !event.ctrlKey && !event.altKey
        && ['Enter', ' ', 'ArrowDown', 'ArrowRight', 'ArrowUp', 'ArrowLeft', 'Home', 'End', 'Delete', 'Backspace', 'Escape'].includes(event.key)) {
        event.stopPropagation();
      }
      return;
    }
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

  /* ── compositions ── */
  function showProjectMenu(event: MouseEvent): void {
    if ((event.target as Element).closest('.asset-card')) return;
    event.preventDefault();
    event.stopPropagation();
    PM.menu(event.currentTarget, [
      { label: 'New Composition…', kb: '⌘N', run: () => PM.cmd('newComposition') },
      { label: 'New Folder', run: () => newFolder(currentFolder) },
      '-',
      { label: 'Import Media…', kb: '⌘I', run: () => PM.pickFiles() },
      { label: 'Import Folder…', run: () => PM.pickFolder({ folder: currentFolder }) },
    ], { x: event.clientX, y: event.clientY });
  }

  /* ── media folders ── */
  /** Dropped files and folders, filed into `folder`. ⌥ keeps a dropped folder
      as a folder of individual files instead of an image sequence, as in AE. */
  function importDrop(event: DragEvent, folder: string | null): void {
    const dt = event.dataTransfer;
    if (!dt?.files.length) return;
    const count = dt.files.length;
    status = `Importing ${count === 1 ? dt.files[0]?.name : `${count} items`}`;
    PM.importFiles(dt, { placement: null, folder, asFolder: event.altKey });
  }

  function folderEdit(label: string, edit: (project: Record<string, any>) => unknown): unknown {
    const result = PM.hist.do(label, () => edit(PM.proj));
    PM.bus.emit('assets');
    return result;
  }

  function newFolder(parent: string | null): void {
    const id = folderEdit('New folder', (project) => createFolder(project, 'Untitled Folder', parent)) as string;
    openFolderId = parent;
    selectedAssetId = null;
    selectedCompId = null;
    selectedFolderId = id;
    renamingFolderId = id;
    status = 'Created a folder';
  }

  function openFolder(id: string | null): void {
    clearSelection();
    openFolderId = id;
    status = id ? `Opened ${folderPath(PM.proj, id).at(-1)?.name ?? 'folder'}` : 'Showing all media';
  }

  function selectFolder(id: string, row?: HTMLElement): void {
    selectedAssetId = null;
    selectedCompId = null;
    PM.Kernel?.services.get('viewer')?.preview?.clear?.();
    selectedFolderId = id;
    row?.focus();
  }

  function commitRename(folder: AssetFolder, value: string): void {
    if (renamingFolderId !== folder.id) return;
    renamingFolderId = null;
    if (value.trim() && value.trim() !== folder.name) folderEdit('Rename folder', (project) => renameFolder(project, folder.id, value));
  }

  function removeFolder(folder: AssetFolder): void {
    folderEdit('Delete folder', (project) => deleteFolder(project, folder.id));
    if (selectedFolderId === folder.id) selectedFolderId = null;
    status = `Deleted ${folder.name}; its contents moved up a level`;
    PM.toast(status);
  }

  function moveInto(items: { assets?: string[]; folders?: string[] }, target: string | null): void {
    const moved = folderEdit('Move to folder', (project) => moveItems(project, items, target));
    if (moved) status = `Moved to ${target ? folderPath(PM.proj, target).at(-1)?.name : 'Media'}`;
  }

  function showFolderMenu(event: MouseEvent, folder: AssetFolder): void {
    event.preventDefault();
    event.stopPropagation();
    selectFolder(folder.id, event.currentTarget as HTMLElement);
    PM.menu(event.currentTarget, [
      { header: folder.name },
      { label: 'Open', run: () => openFolder(folder.id) },
      { label: 'Rename', run: () => { renamingFolderId = folder.id; } },
      { label: 'New Folder Inside', run: () => newFolder(folder.id) },
      { label: 'Import Folder Into…', run: () => PM.pickFolder({ folder: folder.id }) },
      ...(currentFolder ? [{ label: 'Move Out of Folder', run: () => moveInto({ folders: [folder.id] }, crumbs.at(-2)?.id ?? null) }] : []),
      '-',
      { label: 'Delete folder', run: () => removeFolder(folder) },
    ], { x: event.clientX, y: event.clientY });
  }

  /** What is being dragged inside the panel, for a folder or path drop. */
  function draggedItems(): { assets?: string[]; folders?: string[] } | null {
    if (draggingId) return { assets: [draggingId] };
    if (draggingFolderId) return { folders: [draggingFolderId] };
    return null;
  }

  function canDropInto(event: DragEvent, target: string | null): boolean {
    if (fileDrag(event)) return true;
    if (draggingFolderId && target && isWithin(PM.proj, target, draggingFolderId)) return false;
    return !!draggedItems();
  }

  function handleFolderDragOver(event: DragEvent, target: string | null): void {
    if (!canDropInto(event, target)) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.dataTransfer) event.dataTransfer.dropEffect = fileDrag(event) ? 'copy' : 'move';
    folderDropTarget = target ?? '';
  }

  function handleFolderDragLeave(event: DragEvent): void {
    const next = event.relatedTarget as Node | null;
    if (!next || !(event.currentTarget as Node).contains(next)) folderDropTarget = null;
  }

  function handleFolderDrop(event: DragEvent, target: string | null): void {
    if (!canDropInto(event, target)) return;
    event.preventDefault();
    event.stopPropagation();
    folderDropTarget = null;
    dropDepth = 0;
    dropOver = false;
    if (fileDrag(event)) { importDrop(event, target); return; }
    const items = draggedItems();
    if (items) moveInto(items, target);
  }

  function handleFolderDragStart(event: DragEvent, folder: AssetFolder): void {
    if (renamingFolderId === folder.id || (event.target as Element).closest('button, input')) { event.preventDefault(); return; }
    const dt = event.dataTransfer;
    if (!dt) return;
    dt.setData('application/x-powermove-folder', folder.id);
    dt.effectAllowed = 'move';
    const tile = (event.currentTarget as HTMLElement).querySelector<HTMLElement>('.asset-preview');
    if (tile) dt.setDragImage(tile, tile.offsetWidth / 2, tile.offsetHeight / 2);
    draggingFolderId = folder.id;
    selectFolder(folder.id);
  }

  function handleFolderKeydown(event: KeyboardEvent, folder: AssetFolder): void {
    if (event.target !== event.currentTarget) return;
    if (event.key === 'Enter') { event.preventDefault(); openFolder(folder.id); }
    else if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault(); event.stopPropagation();
      removeFolder(folder);
    } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); clearSelection(); }
  }

  function handleRenameKeydown(event: KeyboardEvent, folder: AssetFolder): void {
    event.stopPropagation();
    if (event.key === 'Enter') { event.preventDefault(); commitRename(folder, (event.currentTarget as HTMLInputElement).value); }
    else if (event.key === 'Escape') { event.preventDefault(); renamingFolderId = null; }
  }

  function focusRename(input: HTMLInputElement) {
    input.focus();
    input.select();
  }

  function folderDetails(folder: AssetFolder): string {
    const count = folderView.counts.get(folder.id) ?? 0;
    return count ? `${count} ${count === 1 ? 'item' : 'items'}` : 'Empty';
  }

  function compDetails(comp: CompItem): string {
    const fps = Math.round(comp.fps * 100) / 100;
    return `${comp.w}×${comp.h} · ${fps} fps · ${mediaDuration(comp.dur) || `${comp.dur}s`}`;
  }

  function selectComp(id: string, row?: HTMLElement): void {
    selectedAssetId = null;
    selectedFolderId = null;
    PM.Kernel?.services.get('viewer')?.preview?.clear?.();
    selectedCompId = id;
    row?.focus();
    status = `Selected ${comps.find((comp) => comp.id === id)?.name ?? 'composition'}`;
  }

  function openComp(comp: CompItem): void {
    PM.cmd('openComposition', comp.id);
    status = `Opened ${comp.name}`;
  }

  function nestComp(comp: CompItem): void {
    if (PM.cmd('addCompositionToTimeline', comp.id)) status = `Added ${comp.name} to ${PM.proj.compName}`;
  }

  function showCompMenu(event: MouseEvent, comp: CompItem): void {
    event.preventDefault();
    event.stopPropagation();
    selectComp(comp.id, event.currentTarget as HTMLElement);
    const nestable = PM.Comps.canNest(comp.id);
    PM.menu(event.currentTarget, [
      { header: comp.name },
      { label: 'New Composition…', kb: '⌘N', run: () => PM.cmd('newComposition') },
      { label: 'Open in Timeline', disabled: comp.open, run: () => openComp(comp) },
      { label: 'Add to timeline', disabled: !nestable, run: () => nestComp(comp) },
      '-',
      { label: 'Composition Settings…', run: () => PM.cmd('compositionSettings', comp.id) },
      { label: 'Duplicate', run: () => { const id = PM.cmd('duplicateComposition', comp.id); if (id) selectedCompId = id; } },
      '-',
      { label: 'Delete composition…', disabled: comps.length <= 1, run: () => PM.cmd('deleteComposition', comp.id) },
    ], { x: event.clientX, y: event.clientY });
  }

  function handleCompDragStart(event: DragEvent, comp: CompItem): void {
    if ((event.target as Element).closest('button')) { event.preventDefault(); return; }
    const dt = event.dataTransfer;
    if (!dt) return;
    const payload = { id: comp.id, name: comp.name, kind: 'comp', dur: comp.dur };
    dt.setData('application/x-powermove-asset', JSON.stringify(payload));
    dt.effectAllowed = 'copy';
    const tile = (event.currentTarget as HTMLElement).querySelector<HTMLElement>('.asset-preview');
    if (tile) dt.setDragImage(tile, tile.offsetWidth / 2, tile.offsetHeight / 2);
    draggingCompId = comp.id;
    selectedCompId = comp.id;
    PM.mediaDrag = payload;
  }
  function handleCompDragEnd(): void { draggingCompId = null; PM.mediaDrag = null; }

  function handleCompKeydown(event: KeyboardEvent, comp: CompItem): void {
    if (event.target !== event.currentTarget) return;
    if (event.key === 'Enter') { event.preventDefault(); openComp(comp); }
    else if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault(); event.stopPropagation();
      PM.cmd('deleteComposition', comp.id);
    } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); clearSelection(); }
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
  oncontextmenu={showProjectMenu}
>
  {#if currentFolder}
    <nav class="asset-path" aria-label="Folder path">
      <button class="asset-path-up" type="button" title="Up one level" aria-label="Up one level"
        onclick={() => openFolder(crumbs.at(-2)?.id ?? null)}
        ondragover={(event) => handleFolderDragOver(event, crumbs.at(-2)?.id ?? null)}
        ondragleave={handleFolderDragLeave}
        ondrop={(event) => handleFolderDrop(event, crumbs.at(-2)?.id ?? null)}>
        <Icon {PM} name="chev" />
      </button>
      <button class="asset-path-crumb" type="button" class:is-drop-target={folderDropTarget === ''}
        onclick={() => openFolder(null)}
        ondragover={(event) => handleFolderDragOver(event, null)}
        ondragleave={handleFolderDragLeave}
        ondrop={(event) => handleFolderDrop(event, null)}>Media</button>
      {#each crumbs as crumb, index (crumb.id)}
        <span class="asset-path-sep" aria-hidden="true">/</span>
        <button class="asset-path-crumb" type="button" class:is-drop-target={folderDropTarget === crumb.id}
          aria-current={index === crumbs.length - 1 ? 'location' : undefined}
          onclick={() => openFolder(crumb.id)}
          ondragover={(event) => handleFolderDragOver(event, crumb.id)}
          ondragleave={handleFolderDragLeave}
          ondrop={(event) => handleFolderDrop(event, crumb.id)}>{crumb.name}</button>
      {/each}
    </nav>
  {/if}
  <div class="asset-list" role="listbox" tabindex="-1" aria-label="Project media" bind:this={listElement} onpointerdown={handleListPointerDown}>
    {#each folders as folder (folder.id)}
      <div
        class="asset-card is-folder"
        class:is-dragging={draggingFolderId === folder.id}
        class:is-drop-target={folderDropTarget === folder.id}
        role="option"
        tabindex="-1"
        aria-selected={selectedFolderId === folder.id}
        data-folder-id={folder.id}
        draggable={renamingFolderId !== folder.id}
        title={`${folder.name} · double-click to open, or drop media onto it`}
        ondragstart={(event) => handleFolderDragStart(event, folder)}
        ondragend={() => { draggingFolderId = null; folderDropTarget = null; }}
        ondragover={(event) => handleFolderDragOver(event, folder.id)}
        ondragleave={handleFolderDragLeave}
        ondrop={(event) => handleFolderDrop(event, folder.id)}
        onpointerdown={(event) => { if (event.button === 0 && !(event.target as Element).closest('button, input')) selectFolder(folder.id, event.currentTarget as HTMLElement); }}
        oncontextmenu={(event) => showFolderMenu(event, folder)}
        ondblclick={(event) => { if ((event.target as Element).closest('button, input')) return; event.preventDefault(); openFolder(folder.id); }}
        onkeydown={(event) => handleFolderKeydown(event, folder)}
      >
        <span class="asset-preview folder">
          <Icon {PM} name="project" />
        </span>
        <span class="asset-copy">
          {#if renamingFolderId === folder.id}
            <input class="asset-rename" value={folder.name} aria-label="Folder name" use:focusRename
              onkeydown={(event) => handleRenameKeydown(event, folder)}
              onblur={(event) => commitRename(folder, event.currentTarget.value)} />
          {:else}
            <b title={folder.name}>{folder.name}</b>
          {/if}
          <small>{folderDetails(folder)}</small>
        </span>
      </div>
    {/each}
    {#each currentFolder ? [] : comps as comp (comp.id)}
      <div
        class="asset-card is-comp"
        class:is-dragging={draggingCompId === comp.id}
        class:is-open={comp.open}
        role="option"
        tabindex="-1"
        aria-selected={selectedCompId === comp.id}
        aria-current={comp.open ? 'true' : undefined}
        data-comp-id={comp.id}
        draggable="true"
        title={`${comp.name} · double-click to open, or drag into a timeline`}
        ondragstart={(event) => handleCompDragStart(event, comp)}
        ondragend={handleCompDragEnd}
        onpointerdown={(event) => { if (event.button === 0 && !(event.target as Element).closest('button')) selectComp(comp.id, event.currentTarget as HTMLElement); }}
        oncontextmenu={(event) => showCompMenu(event, comp)}
        ondblclick={(event) => { if ((event.target as Element).closest('button')) return; event.preventDefault(); openComp(comp); }}
        onkeydown={(event) => handleCompKeydown(event, comp)}
      >
        <span class="asset-preview comp" style={`--comp-bg:${comp.bg || '#000000'};--comp-ratio:${comp.w}/${comp.h}`}>
          <span class="comp-frame" aria-hidden="true">
            {#if compThumbs.urls[comp.id]}<img class="comp-thumb" src={compThumbs.urls[comp.id]} alt="" />{/if}
          </span>
          {#if !compThumbs.urls[comp.id]}<Icon {PM} name="layers" />{/if}
          <span class="asset-badge">{mediaDuration(comp.dur) || `${comp.dur}s`}</span>
          {#if comp.open}<span class="comp-open" role="img" aria-label="Open in the timeline"></span>{/if}
          <span class="asset-actions">
            <button class="asset-add" type="button" title="Add to timeline" aria-label={`Add ${comp.name} to timeline`} disabled={!(doc.tick.structure, PM.Comps.canNest(comp.id))}
              onclick={(event) => { event.stopPropagation(); nestComp(comp); }}>
              <Icon {PM} name="plus" />
            </button>
          </span>
        </span>
        <span class="asset-copy">
          <b title={comp.name}>{comp.name}</b>
          <small title={compDetails(comp)}>{compDetails(comp)}</small>
        </span>
      </div>
    {/each}
    {#each assets as asset, index (asset.id)}
      {@const currentAsset = (doc.tick.assets, liveAsset(asset))}
      {@const posterSrc = (doc.tick.assets, posterUrl(asset))}
      {@const loading = (doc.tick.assets, isLoading(asset))}
      {@const offline = !currentAsset && !loading}
      {@const cloud = (doc.tick.assets, offline ? PM.assets.cloud?.get(asset.id) : undefined)}
      {@const loadError = (doc.tick.assets, offline ? PM.assets.errors?.get(asset.id) : undefined)}
      {@const downloading = cloud?.state === 'downloading'}
      {@const detail = (doc.tick.assets, offlineDetail(asset))}
      <div
        class="asset-card"
        class:is-dragging={draggingId === asset.id}
        class:is-offline={offline}
        class:is-loading={loading}
        class:is-cloud={!!cloud}
        role="option"
        aria-busy={loading || downloading}
        draggable={!offline && !loading}
        ondragstart={(event) => handleDragStart(event, asset)}
        ondragend={handleDragEnd}
        tabindex={activeAssetId ? (activeAssetId === asset.id ? 0 : -1) : (index === 0 ? 0 : -1)}
        aria-selected={activeAssetId === asset.id}
        data-asset-id={asset.id}
        title={loading ? 'Loading media…' : offline ? detail : 'Select media · double-click to preview, or drag onto the timeline'}
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
            <span class="asset-loading" role="status" aria-label="Loading media">Loading…</span>
          {:else if offline}
            <span class="asset-offline" role="img" aria-label={cloud ? 'Media in cloud' : loadError ? 'Media unavailable' : 'Media offline'}>
              <Icon {PM} name={cloud ? 'download' : 'missing'} />
              {downloading ? 'Downloading…' : cloud ? 'In cloud' : loadError ? 'Media unavailable' : 'Media offline'}
            </span>
          {:else if asset.dur}<span class="asset-badge">{mediaDuration(asset.dur)}</span>{/if}
          <span class="asset-actions">
            {#if finderPath(asset)}
              <button class="asset-reveal" type="button" title="Reveal in Finder" aria-label={`Reveal ${asset.name} in Finder`} onclick={(event) => revealAsset(event, asset)}>
                <Icon {PM} name="project" />
              </button>
            {/if}
            {#if cloud}
              <button class="asset-download" type="button" disabled={downloading} title={downloading ? 'Downloading…' : cloud.state === 'error' ? 'Retry download' : 'Download from cloud'} aria-label={`Download ${asset.name} from cloud`} onclick={(event) => addAsset(event, asset)}>
                <Icon {PM} name="download" />
              </button>
            {:else if offline}
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
            <small class="asset-missing" title={detail}>{detail}</small>
          {:else}
            <small title={mediaDetails(asset)}>{mediaDetails(asset)}</small>
          {/if}
        </span>
      </div>
    {:else}
      {#if !folders.length}
        <div class="asset-empty">
          <Icon {PM} name="project" />
          <b>{currentFolder ? 'This folder is empty' : 'Add media'}</b>
          <span>{currentFolder ? 'Drag media or files here.' : 'Drag files here or import from your computer.'}</span>
        </div>
      {/if}
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
