/* Source preview: show an imported asset in the viewer, on top of the
   composition, the way an editor's source monitor does. Owned by the viewer
   panel; driven by the media panel (Space to play/pause, Esc to close).

   The monitor is deliberately loud about being a monitor. Swapping the stage
   for unrelated footage with no chrome reads as a broken composition, so the
   overlay carries a titled header, its own transport, and an explicit way
   back to the timeline. */

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
  #source-preview[data-open="true"]{display:flex;animation:sp-in 140ms var(--ease-io, ease-out) both}
  @keyframes sp-in{from{opacity:0;transform:scale(.99)}to{opacity:1;transform:none}}
  /* An accent ring frames the whole stage so the swapped picture reads as a
     mode the user opened, not as the composition losing its render. */
  #source-preview::after{content:"";position:absolute;inset:0;z-index:3;pointer-events:none;border-radius:inherit;box-shadow:inset 0 0 0 1.5px color-mix(in srgb,var(--accent) 62%,transparent)}
  #source-preview .sp-head{position:relative;z-index:2;flex:none;display:flex;align-items:center;gap:8px;height:28px;padding:0 6px 0 8px;background:var(--bg-panel-2);border-bottom:1px solid var(--line)}
  #source-preview .sp-badge{flex:none;height:16px;display:flex;align-items:center;padding:0 5px;border-radius:var(--r-xs);background:var(--accent);color:var(--on-accent);font-size:9px;font-weight:var(--fw-medium);letter-spacing:.06em;text-transform:uppercase}
  #source-preview .sp-name{min-width:0;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--tx);font-size:var(--fs-xs)}
  #source-preview .sp-hint{flex:none;color:var(--tx-4);font-size:10px;white-space:nowrap}
  #source-preview .sp-close{flex:none;width:20px;height:20px;display:grid;place-items:center;border-radius:var(--r-xs);color:var(--tx-3);background:transparent}
  #source-preview .sp-close:hover{color:var(--tx);background:var(--bg-row)}
  #source-preview .sp-close svg{width:12px;height:12px}
  #source-preview .sp-media{position:relative;flex:1;min-height:0;display:grid;place-items:center;background:#000}
  #source-preview .sp-media img,#source-preview .sp-media video{max-width:100%;max-height:100%;object-fit:contain;display:block}
  /* Audio renders nothing on its own; a titled card keeps the stage from
     looking like a failed frame while the sound plays. */
  /* The picture area is always black, in both themes, so its labels carry
     fixed light values rather than theme text colors. */
  #source-preview .sp-audio{display:flex;flex-direction:column;align-items:center;gap:6px;padding:18px;color:rgb(255 255 255 / .5);font-size:10px;text-align:center}
  #source-preview .sp-audio svg{width:28px;height:28px;color:var(--accent)}
  #source-preview .sp-audio b{max-width:220px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:rgb(255 255 255 / .92);font-size:var(--fs-xs);font-weight:var(--fw-medium)}
  #source-preview .sp-status{position:absolute;color:rgb(255 255 255 / .55);font-size:10px}
  #source-preview .sp-status[hidden]{display:none}
  /* A flex display beats the hidden attribute's UA rule, and a still with a
     transport under it is exactly the kind of dead control that reads broken. */
  #source-preview .sp-foot[hidden]{display:none}
  #source-preview .sp-foot{position:relative;z-index:2;flex:none;display:flex;align-items:center;gap:8px;height:28px;padding:0 10px;background:var(--bg-panel-2);border-top:1px solid var(--line)}
  #source-preview .sp-play{flex:none;width:20px;height:20px;display:grid;place-items:center;border-radius:var(--r-xs);color:var(--tx-2);background:transparent}
  #source-preview .sp-play:hover{color:var(--tx);background:var(--bg-row)}
  #source-preview .sp-play svg{width:11px;height:11px}
  #source-preview .sp-time,#source-preview .sp-dur{flex:none;color:var(--tx-4);font-size:10px;font-variant-numeric:tabular-nums}
  #source-preview .sp-scrub{flex:1;min-width:0;height:14px;display:flex;align-items:center;cursor:pointer;touch-action:none}
  #source-preview .sp-scrub i{width:100%;height:3px;border-radius:2px;background:rgb(var(--ink-rgb, 10 10 12) / .22)}
  #source-preview .sp-scrub i b{display:block;width:0;height:100%;border-radius:inherit;background:var(--accent)}
`;

const KIND_LABEL: Record<string, string> = { video: 'Video', image: 'Image', audio: 'Audio' };
const KIND_ICON: Record<string, string> = { video: 'film', image: 'image', audio: 'music' };

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

  const head = document.createElement('div');
  head.className = 'sp-head';
  const badge = document.createElement('span');
  badge.className = 'sp-badge';
  badge.textContent = 'Source';
  const name = document.createElement('span');
  name.className = 'sp-name';
  const hint = document.createElement('span');
  hint.className = 'sp-hint';
  hint.textContent = 'Esc returns to the composition';
  const close = document.createElement('button');
  close.className = 'sp-close';
  close.type = 'button';
  close.title = 'Close source preview';
  close.setAttribute('aria-label', 'Close source preview');
  close.append(icon('x'));
  head.append(badge, name, hint, close);

  const media = document.createElement('div');
  media.className = 'sp-media';
  const status = document.createElement('span');
  status.className = 'sp-status';
  status.hidden = true;

  const foot = document.createElement('div');
  foot.className = 'sp-foot';
  const play = document.createElement('button');
  play.className = 'sp-play';
  play.type = 'button';
  const elapsed = document.createElement('span');
  elapsed.className = 'sp-time';
  elapsed.textContent = '0:00';
  const scrub = document.createElement('div');
  scrub.className = 'sp-scrub';
  scrub.setAttribute('role', 'slider');
  scrub.setAttribute('aria-label', 'Source position');
  const track = document.createElement('i');
  const fill = document.createElement('b');
  track.append(fill);
  scrub.append(track);
  const total = document.createElement('span');
  total.className = 'sp-dur';
  total.textContent = '0:00';
  foot.append(play, elapsed, scrub, total);

  root.append(styles, head, media, foot);
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
    elapsed.textContent = clock(at);
    total.textContent = known ? clock(duration) : '--:--';
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
    status.hidden = true;
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
    elapsed.textContent = '0:00';
    total.textContent = '0:00';
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
      const title = document.createElement('b');
      title.textContent = String(record.name || 'Audio');
      const sub = document.createElement('small');
      sub.textContent = 'Audio only — no picture';
      card.append(icon('music'), title, sub);
      media.append(audio, card); bindElement(audio);
    } else {
      return false;
    }
    /* Decoding takes a beat on some codecs. Naming the wait keeps an
       undecoded first frame from reading as a dead render. */
    status.textContent = 'Loading…';
    status.hidden = !element;
    media.append(status);
    name.textContent = String(record.name || assetId);
    badge.textContent = `Source · ${KIND_LABEL[kind] || 'Media'}`;
    foot.hidden = !element;
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
  const handleKeydown = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && activeId) { clear(); event.stopPropagation(); }
  };
  media.addEventListener('pointerdown', handleMediaPointerDown);
  head.addEventListener('pointerdown', handleHeadPointerDown);
  foot.addEventListener('pointerdown', handleHeadPointerDown);
  close.addEventListener('click', handleClose);
  play.addEventListener('click', handlePlay);
  scrub.addEventListener('pointerdown', handleScrubDown);
  scrub.addEventListener('pointermove', handleScrubMove);
  scrub.addEventListener('pointerup', handleScrubUp);
  window.addEventListener('keydown', handleKeydown, true);

  function dispose(): void {
    clear();
    media.removeEventListener('pointerdown', handleMediaPointerDown);
    head.removeEventListener('pointerdown', handleHeadPointerDown);
    foot.removeEventListener('pointerdown', handleHeadPointerDown);
    close.removeEventListener('click', handleClose);
    play.removeEventListener('click', handlePlay);
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
