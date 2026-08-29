/* Source preview: show an imported asset in the viewer, on top of the
   composition, the way an editor's source monitor does. Owned by the viewer
   panel; driven by the media panel (Space to play/pause, Esc to close). */

export interface SourcePreview {
  /** Show the asset (paused). Returns false if it can't be previewed. */
  show(assetId: string): boolean;
  /** Show if hidden, then play; pause if already playing. */
  toggle(assetId: string): void;
  clear(): void;
  dispose(): void;
  readonly activeId: string | null;
  readonly playing: boolean;
}

const STYLES = `
  #source-preview{position:absolute;inset:0;z-index:6;display:none;flex-direction:column;background:var(--bg-panel-2);border-radius:inherit;overflow:hidden}
  #source-preview[data-open="true"]{display:flex}
  #source-preview .sp-media{flex:1;min-height:0;display:grid;place-items:center;background:#000}
  #source-preview .sp-media img,#source-preview .sp-media video{max-width:100%;max-height:100%;object-fit:contain;display:block}
`;

export function installSourcePreview(PM: any, stage: HTMLElement): SourcePreview {
  const existing = stage.querySelector<HTMLElement>('#source-preview');
  /* Extension hot updates reinstall this controller in place. Dispose the old
     listeners and overlay, but keep the WebGL stage and Electron window. */
  const previous = PM.Viewer?.preview;
  if (previous?.dispose) previous.dispose();
  else previous?.clear?.();
  existing?.remove();

  const root = document.createElement('div');
  root.id = 'source-preview';
  root.dataset.open = 'false';
  const styles = document.createElement('style');
  styles.textContent = STYLES;
  const media = document.createElement('div');
  media.className = 'sp-media';
  root.append(styles, media);
  (stage.querySelector('#stage-inner') || stage).appendChild(root);

  let activeId: string | null = null;
  let element: HTMLMediaElement | null = null;
  let audioUrl: string | null = null;

  function clear(): void {
    if (element) { try { element.pause(); } catch { } }
    if (audioUrl) { URL.revokeObjectURL(audioUrl); audioUrl = null; }
    element = null; activeId = null;
    media.replaceChildren();
    root.dataset.open = 'false';
    PM.bus?.emit?.('source-preview', null);
  }

  const liveUrl = (asset: any): string => String(asset?.url || asset?.el?.currentSrc || asset?.el?.src || '');

  function show(assetId: string): boolean {
    const record = PM.proj?.assets?.[assetId];
    const live = PM.assets?.get?.(assetId);
    if (!record) return false;
    if (activeId === assetId) return true;
    clear();
    const kind = record.kind;
    const url = liveUrl(live);
    if (kind === 'image' && url) {
      const img = document.createElement('img'); img.src = url; img.alt = '';
      media.appendChild(img);
    } else if (kind === 'video' && url) {
      const video = document.createElement('video');
      video.src = url; video.playsInline = true; video.preload = 'auto';
      media.appendChild(video); element = video;
    } else if (kind === 'audio' && live?.audioBlob) {
      /* Audio draws nothing — exactly what the composition would show if this
         were its only source. The stage stays black with no preview chrome. */
      audioUrl = URL.createObjectURL(live.audioBlob);
      const audio = document.createElement('audio'); audio.src = audioUrl; audio.preload = 'auto';
      media.append(audio); element = audio;
    } else {
      return false;
    }
    if (element) {
      element.addEventListener('ended', () => { element?.pause(); });
    }
    activeId = assetId;
    root.dataset.open = 'true';
    /* A source monitor and the composition never play at once. */
    if (PM.playing) PM.pause?.() ?? PM.play?.();
    PM.bus?.emit?.('source-preview', assetId);
    return true;
  }

  function toggle(assetId: string): void {
    if (activeId !== assetId && !show(assetId)) return;
    if (!element) return;
    if (element.paused) void element.play().catch(() => { });
    else element.pause();
  }

  /* Clicking the monitor returns to the composition. */
  const handlePointerDown = () => clear();
  const handleKeydown = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && activeId) { clear(); event.stopPropagation(); }
  };
  root.addEventListener('pointerdown', handlePointerDown);
  window.addEventListener('keydown', handleKeydown, true);

  function dispose(): void {
    clear();
    root.removeEventListener('pointerdown', handlePointerDown);
    window.removeEventListener('keydown', handleKeydown, true);
    root.remove();
  }

  const api: SourcePreview = {
    show, toggle, clear, dispose,
    get activeId() { return activeId; },
    get playing() { return !!element && !element.paused; },
  };
  if (PM.Viewer) PM.Viewer.preview = api;
  return api;
}
