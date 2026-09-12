/* Source preview: show an imported asset in the viewer, on top of the
   composition, the way an editor's source monitor does. Owned by the viewer
   panel; driven by the media panel (Space to play/pause, Esc to close).

   A quiet floating transport keeps playback controls together. */

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
  #source-preview{position:absolute;inset:0;z-index:6;display:none;background:var(--bg-panel-2);border-radius:inherit;overflow:hidden}
  #source-preview[data-open="true"]{display:block;animation:sp-in 140ms var(--ease-io, ease-out) both}
  @keyframes sp-in{from{opacity:0}to{opacity:1}}
  #source-preview .sp-media{position:absolute;inset:0;display:grid;place-items:center;background:#000}
  #source-preview[data-kind="audio"] .sp-media{background:var(--bg-panel-2)}
  #source-preview .sp-media img,#source-preview .sp-media video{width:100%;height:100%;min-height:0;object-fit:contain;display:block}
  #source-preview .sp-audio{display:flex;flex-direction:column;align-items:center;gap:var(--gap);min-width:0;max-width:calc(100% - 2 * var(--pad));color:var(--tx-4)}
  #source-preview .sp-audio svg{width:32px;height:32px}
  #source-preview .sp-audio .sp-name{flex:none;max-width:100%;padding:0;text-align:center}
  #source-preview .sp-foot{position:absolute;z-index:2;bottom:var(--pad);left:50%;transform:translateX(-50%);width:min(480px,calc(100% - 2 * var(--pad)));box-sizing:border-box;display:flex;align-items:center;gap:8px;padding:6px;border-radius:calc(var(--r-sm) + 6px);background:var(--bg-float);box-shadow:var(--shadow-float)}
  #source-preview .sp-name{min-width:0;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--tx-2);font-size:var(--fs-xs);font-weight:var(--fw-medium);padding-left:4px}
  #source-preview .sp-close,#source-preview .sp-play{flex:none;width:var(--hit);height:var(--hit);display:grid;place-items:center;border:0;border-radius:var(--r-sm);color:var(--tx-2);background:transparent;cursor:pointer}
  #source-preview .sp-close:hover,#source-preview .sp-play:hover{color:var(--tx);background:var(--bg-row-hi)}
  #source-preview .sp-close:active,#source-preview .sp-play:active{background:var(--bg-sunken)}
  #source-preview :focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  #source-preview .sp-close svg,#source-preview .sp-play svg{width:14px;height:14px}
  #source-preview .sp-scrub{flex:1;min-width:24px;height:var(--hit);display:flex;align-items:center;cursor:pointer;touch-action:none;border-radius:var(--r-xs)}
  #source-preview .sp-scrub i{width:100%;height:3px;border-radius:var(--r-pill);background:rgb(var(--ink-rgb) / .22)}
  #source-preview .sp-scrub i b{display:block;width:0;height:100%;border-radius:inherit;background:var(--accent)}
  #source-preview [hidden]{display:none!important}
  @media(prefers-reduced-motion:reduce){#source-preview[data-open="true"]{animation:none}}
`;

/** Source media runs on its own clock, so it reads in wall time, not project frames. */
function clock(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const whole = Math.floor(seconds);
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
}

export function installSourcePreview(PM: any, stage: HTMLElement): SourcePreview {
  const existing = stage.querySelector<HTMLElement>('#source-preview');
  /* Extension hot updates reinstall this controller in place. Dispose the old
     listeners and overlay, but keep the WebGL stage and Electron window. */
  const previous = PM.Viewer?.preview;
  if (previous?.dispose) previous.dispose();
  else previous?.clear?.();
  existing?.remove();

  const icon = (name: string): Element =>
    PM.icon?.(name) ?? document.createElementNS('http://www.w3.org/2000/svg', 'svg');

  const root = document.createElement('div');
  root.id = 'source-preview';
  root.dataset.open = 'false';
  const styles = document.createElement('style');
  styles.textContent = STYLES;

  const name = document.createElement('span');
  name.className = 'sp-name';
  const close = document.createElement('button');
  close.className = 'sp-close';
  close.type = 'button';
  close.title = 'Close source preview';
  close.setAttribute('aria-label', 'Close source preview');
  close.append(icon('x'));

  const media = document.createElement('div');
  media.className = 'sp-media';
  const foot = document.createElement('div');
  foot.className = 'sp-foot';
  const play = document.createElement('button');
  play.className = 'sp-play';
  play.type = 'button';
  const scrub = document.createElement('div');
  scrub.className = 'sp-scrub';
  scrub.setAttribute('role', 'slider');
  scrub.setAttribute('aria-label', 'Source position');
  const track = document.createElement('i');
  const fill = document.createElement('b');
  track.append(fill);
  scrub.append(track);
  foot.append(play, name, scrub, close);

  root.append(styles, media, foot);
  (stage.querySelector('#stage-inner') || stage).appendChild(root);

  let activeId: string | null = null;
  let element: HTMLMediaElement | null = null;
  let audioUrl: string | null = null;
  let frame = 0;

  function setPlayIcon(): void {
    play.replaceChildren(icon(element && !element.paused ? 'pause' : 'play'));
    const label = element && !element.paused ? 'Pause source' : 'Play source';
    play.title = label;
    play.setAttribute('aria-label', label);
  }

  function paint(): void {
    if (!element) return;
    const duration = Number(element.duration);
    const known = Number.isFinite(duration) && duration > 0;
    const at = Number(element.currentTime) || 0;
    scrub.setAttribute('aria-valuemin', '0');
    scrub.setAttribute('aria-valuemax', String(known ? duration : 0));
    scrub.setAttribute('aria-valuenow', String(at));
    fill.style.width = known ? `${Math.min(100, (at / duration) * 100)}%` : '0%';
    scrub.setAttribute('aria-valuetext', `${clock(at)} of ${known ? clock(duration) : 'unknown'}`);
  }

  /* 'timeupdate' only fires a few times a second, which reads as a stuttering
     playhead next to smooth video. Track the element on animation frames. */
  function follow(): void {
    frame = 0;
    if (!element || element.paused) return;
    paint();
    frame = window.requestAnimationFrame(follow);
  }

  function onPlayState(): void {
    setPlayIcon();
    paint();
    if (frame) window.cancelAnimationFrame(frame);
    frame = 0;
    if (element && !element.paused) frame = window.requestAnimationFrame(follow);
  }

  function onLoaded(): void {
    paint();
  }

  function bindElement(next: HTMLMediaElement): void {
    element = next;
    next.addEventListener('play', onPlayState);
    next.addEventListener('pause', onPlayState);
    next.addEventListener('ended', onPlayState);
    next.addEventListener('timeupdate', paint);
    next.addEventListener('durationchange', paint);
    next.addEventListener('loadeddata', onLoaded);
    next.addEventListener('loadedmetadata', onLoaded);
  }

  function clear(): void {
    if (frame) window.cancelAnimationFrame(frame);
    frame = 0;
    if (element) {
      try { element.pause(); } catch { }
      element.removeEventListener('play', onPlayState);
      element.removeEventListener('pause', onPlayState);
      element.removeEventListener('ended', onPlayState);
      element.removeEventListener('timeupdate', paint);
      element.removeEventListener('durationchange', paint);
      element.removeEventListener('loadeddata', onLoaded);
      element.removeEventListener('loadedmetadata', onLoaded);
    }
    if (audioUrl) { URL.revokeObjectURL(audioUrl); audioUrl = null; }
    element = null; activeId = null;
    fill.style.width = '0%';
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
    foot.insertBefore(name, scrub);
    const kind = record.kind;
    const url = liveUrl(live);
    if (kind === 'image' && url) {
      const img = document.createElement('img'); img.src = url; img.alt = '';
      media.append(img);
    } else if (kind === 'video' && url) {
      const video = document.createElement('video');
      video.src = url; video.playsInline = true; video.preload = 'auto';
      media.append(video); bindElement(video);
    } else if (kind === 'audio' && live?.audioBlob) {
      audioUrl = URL.createObjectURL(live.audioBlob);
      const audio = document.createElement('audio'); audio.src = audioUrl; audio.preload = 'auto';
      const card = document.createElement('div');
      card.className = 'sp-audio';
      const audioIcon = icon('music');
      audioIcon.setAttribute('aria-hidden', 'true');
      card.append(audioIcon, name);
      media.append(audio, card); bindElement(audio);
    } else {
      return false;
    }
    name.textContent = String(record.name || assetId);
    name.title = name.textContent;
    root.dataset.kind = kind;
    play.hidden = scrub.hidden = !element;
    scrub.tabIndex = element ? 0 : -1;
    setPlayIcon();
    paint();
    activeId = assetId;
    root.dataset.open = 'true';
    /* A source monitor and the composition never play at once. */
    if (PM.playing) PM.pause?.();
    PM.bus?.emit?.('source-preview', assetId);
    return true;
  }

  function playPause(): void {
    if (!element) return;
    if (!element.paused) { element.pause(); return; }
    const duration = Number(element.duration);
    /* Restart rather than sitting on the last frame doing nothing. */
    if (Number.isFinite(duration) && duration > 0 && element.currentTime >= duration - 0.01) {
      try { element.currentTime = 0; } catch { }
    }
    void element.play().catch(() => { });
  }

  function toggle(assetId: string): void {
    if (activeId !== assetId && !show(assetId)) return;
    playPause();
  }

  function seekTo(event: PointerEvent): void {
    if (!element) return;
    const duration = Number(element.duration);
    if (!(Number.isFinite(duration) && duration > 0)) return;
    const box = scrub.getBoundingClientRect();
    if (!box.width) return;
    const ratio = Math.min(1, Math.max(0, (event.clientX - box.left) / box.width));
    try { element.currentTime = ratio * duration; } catch { }
    paint();
  }

  /* Clicking the picture toggles playback like any monitor. Dismissing is an
     explicit act — the close button or Esc — so a stray click inside the
     preview never looks like the composition flickering back on its own. */
  const handleMediaPointerDown = (event: PointerEvent) => { event.stopPropagation(); playPause(); };
  const handleClose = (event: Event) => { event.stopPropagation(); clear(); };
  const handleHeadPointerDown = (event: PointerEvent) => event.stopPropagation();
  const handlePlay = (event: Event) => { event.stopPropagation(); playPause(); };
  const handleScrubDown = (event: PointerEvent) => {
    event.stopPropagation();
    scrub.setPointerCapture?.(event.pointerId);
    seekTo(event);
  };
  const handleScrubMove = (event: PointerEvent) => {
    if (scrub.hasPointerCapture?.(event.pointerId)) seekTo(event);
  };
  const handleScrubUp = (event: PointerEvent) => scrub.releasePointerCapture?.(event.pointerId);
  const handleScrubKey = (event: KeyboardEvent) => {
    if (!element || !Number.isFinite(element.duration)) return;
    const offsets: Record<string, number> = { ArrowLeft: -5, ArrowRight: 5 };
    if (!(event.key in offsets) && event.key !== 'Home' && event.key !== 'End') return;
    event.preventDefault(); event.stopPropagation();
    element.currentTime = event.key === 'Home' ? 0 : event.key === 'End' ? element.duration
      : Math.max(0, Math.min(element.duration, element.currentTime + (offsets[event.key] ?? 0)));
    paint();
  };
  const handleKeydown = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && activeId) { clear(); event.stopPropagation(); }
  };
  media.addEventListener('pointerdown', handleMediaPointerDown);
  foot.addEventListener('pointerdown', handleHeadPointerDown);
  close.addEventListener('click', handleClose);
  play.addEventListener('click', handlePlay);
  scrub.addEventListener('keydown', handleScrubKey);
  scrub.addEventListener('pointerdown', handleScrubDown);
  scrub.addEventListener('pointermove', handleScrubMove);
  scrub.addEventListener('pointerup', handleScrubUp);
  window.addEventListener('keydown', handleKeydown, true);

  function dispose(): void {
    clear();
    media.removeEventListener('pointerdown', handleMediaPointerDown);
    foot.removeEventListener('pointerdown', handleHeadPointerDown);
    close.removeEventListener('click', handleClose);
    play.removeEventListener('click', handlePlay);
    scrub.removeEventListener('keydown', handleScrubKey);
    scrub.removeEventListener('pointerdown', handleScrubDown);
    scrub.removeEventListener('pointermove', handleScrubMove);
    scrub.removeEventListener('pointerup', handleScrubUp);
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
