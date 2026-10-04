<script lang="ts">
  /* Mixer: a channel strip for every audible layer of the open composition
     and a master strip for the composition's output level. Faders write the
     layers' own level properties (keyframes when animated), mute writes the
     layers' own mute, solo is a listening aid that never reaches the project.
     Meters read live taps on the audio graph and only run while this panel is
     visible and sound is playing. */
  import { onDestroy } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { sel } from '../state/selection.svelte';
  import { controlTime, transport } from '../state/transport.svelte';
  import { isProperty } from '../legacy/core/content-properties';
  import MixerStrip from '../mixer/MixerStrip.svelte';
  import { addKeyCommand, MIXER_ORIGIN, muteCommand, nextSolo } from '../mixer/edits';
  import { levelControl } from '../mixer/level-control';
  import { clampGain } from '../mixer/levels';
  import { MeterLoop, type LevelSource, type MeterPalette } from '../mixer/meter-loop';
  import { deriveStrips, type MixerStrip as Strip } from '../mixer/strips';
  import type { PowermoveAPI } from '../kernel/api';
  import type { PanelProps } from './registerSveltePanel';

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  const api: PowermoveAPI = PM.Kernel.api('mixer');
  const audio = () => PM.Audio as Record<string, any> | undefined;

  const IDLE: LevelSource = { MASTER: '$master', read: () => false, release: () => {}, running: () => false };
  const source: LevelSource = {
    get MASTER() { return audio()?.meters?.MASTER ?? IDLE.MASTER; },
    read: (id, out) => !!audio()?.meters?.read?.(id, out),
    release: () => audio()?.meters?.release?.(),
    running: () => !!audio()?.meters?.running?.() && !!PM.playing
  };
  const loop = new MeterLoop(source);

  let root: HTMLDivElement | undefined = $state();
  let scroller: HTMLDivElement | undefined = $state();
  /* Strips that run past an edge fade out there, so the row reads as scrollable. */
  let edges = $state({ start: false, end: false });
  function measureEdges(): void {
    if (!scroller) return;
    const start = scroller.scrollLeft > 1;
    const end = scroller.scrollLeft + scroller.clientWidth < scroller.scrollWidth - 1;
    if (start !== edges.start || end !== edges.end) edges = { start, end };
  }
  $effect(() => {
    if (!scroller) return;
    const element = scroller;
    const observer = new ResizeObserver(measureEdges);
    observer.observe(element);
    for (const child of element.children) observer.observe(child);
    measureEdges();
    return () => observer.disconnect();
  });
  $effect(() => { strips.length; queueMicrotask(measureEdges); });

  const strips = $derived.by<Strip[]>(() => {
    doc.tick.values; doc.tick.structure; doc.tick.project; doc.tick.history; doc.proj;
    const time = controlTime();
    return deriveStrips(PM.proj, {
      time,
      comps: PM.proj?.comps,
      evaluate: (layer, value, at, path) => isProperty(value) ? PM.evP(layer, value, at, path) : value,
      hasKeyAt: (layer, prop, at) => api.anim.hasKeyAt(layer, prop, at),
      groupAncestors: (layer, layers) => PM.groupAncestors?.(layer, layers) || []
    });
  });

  const masterGain = $derived((doc.tick.values, doc.tick.project, doc.proj, clampGain(Number(PM.proj?.audioGain ?? 1))));
  const selectedIds = $derived(new Set(sel.layers));

  /* ── monitor solo ── */
  let solo = $state<Set<string>>(new Set());
  function setSolo(next: Set<string>): void {
    solo = next;
    audio()?.setMonitorSolo?.([...next]);
  }
  // Solo follows the strips: a strip that leaves the composition leaves the solo set.
  $effect(() => {
    const ids = new Set(strips.map((strip) => strip.id));
    const kept = [...solo].filter((id) => ids.has(id));
    if (kept.length !== solo.size) setSolo(new Set(kept));
  });

  /* ── edits ── */
  const controls = new Map<string, ReturnType<typeof levelControl>>();
  function controlFor(id: string) {
    let control = controls.get(id);
    if (!control) {
      control = levelControl(api, audio, () => strips.find((strip) => strip.id === id) ?? { id, gainKey: 'gain', animated: false, name: '' });
      controls.set(id, control);
    }
    return control;
  }
  const masterControl = levelControl(api, audio, () => 'master');

  function apply(commands: Parameters<PowermoveAPI['edit']['apply']>[0], label: string): void {
    const result = api.edit.apply(commands, { label, origin: MIXER_ORIGIN });
    if (result && result.ok === false) api.ui.toast(result.message || 'Could not change the mix');
    audio()?.retune?.();
  }

  function toggleMute(strip: Strip): void {
    apply(muteCommand(strip), strip.muted ? 'Unmute audio' : 'Mute audio');
  }

  function toggleKey(strip: Strip): void {
    const layer = PM.L?.(strip.id);
    if (!layer) return;
    const time = api.transport.time();
    const raw = layer.d?.[strip.gainKey];
    if (strip.keyAtTime && isProperty(raw)) {
      const key = api.anim.hasKeyAt(layer, raw, time);
      if (!key) return;
      api.history.do('Remove keyframe for Gain', () => { api.anim.removeKey(raw, key); });
      api.transport.invalidate?.();
      audio()?.retune?.();
      return;
    }
    apply(addKeyCommand(strip, strip.gain, time), 'Add keyframe for Gain');
  }

  function select(strip: Strip): void {
    PM.selectLayers?.([strip.id]);
  }

  /* ── automation following ── */
  const followers = new Set<() => void>();
  const follow = (callback: () => void) => {
    followers.add(callback);
    return () => followers.delete(callback);
  };
  loop.onFrame = () => { for (const callback of followers) callback(); };
  const liveGain = (id: string) => () => {
    const layer = PM.L?.(id);
    const strip = strips.find((item) => item.id === id);
    if (!layer || !strip) return 1;
    const value = layer.d?.[strip.gainKey];
    return clampGain(Number(isProperty(value) ? PM.evP(layer, value, PM.time, `c.${strip.gainKey}`) : value ?? 1));
  };

  /* ── visibility: draw only while the panel can be seen ── */
  let intersecting = false;
  const pageVisible = () => typeof document === 'undefined' || document.visibilityState !== 'hidden';
  function syncVisibility(): void {
    const visible = intersecting && pageVisible() && !root?.closest('#pm-panel-pool');
    loop.setVisible(visible);
    // Solo is a listening aid; it never outlives the strips you could see.
    if (!visible && solo.size) setSolo(new Set());
  }
  $effect(() => {
    if (!root) return;
    const observer = new IntersectionObserver(([entry]) => {
      intersecting = !!entry?.isIntersecting && (entry.intersectionRect.height > 0);
      syncVisibility();
    });
    observer.observe(root);
    document.addEventListener('visibilitychange', syncVisibility);
    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', syncVisibility);
      intersecting = false;
      syncVisibility();
    };
  });

  // Playback starting from a stop clears the clip indicators and wakes the loop.
  $effect(() => {
    if (transport.playing) {
      loop.clearClips();
      loop.kick();
    }
  });

  /* ── colours: resolved once per theme, never per frame ── */
  function resolvePalette(): void {
    if (!root) return;
    const probe = document.createElement('span');
    probe.style.display = 'none';
    root.appendChild(probe);
    const read = (value: string) => { probe.style.color = value; return getComputedStyle(probe).color; };
    const palette: MeterPalette = {
      track: read('var(--mx-track)'),
      nominal: read('var(--success)'),
      warn: read('var(--warning)'),
      hot: read('var(--danger)')
    };
    probe.remove();
    loop.setPalette(palette);
  }
  $effect(() => {
    if (!root) return;
    resolvePalette();
    const observer = new MutationObserver(resolvePalette);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'style', 'class'] });
    return () => observer.disconnect();
  });

  onDestroy(() => {
    if (solo.size) audio()?.setMonitorSolo?.([]);
    loop.dispose();
  });
</script>

<div class="mixer" bind:this={root} data-svelte-panel={panelId}>
  {#if strips.length}
    <div class="mixer-strips" class:fade-start={edges.start} class:fade-end={edges.end} bind:this={scroller}
      role="group" aria-label="Channel strips" onscroll={measureEdges}>
      {#each strips as strip (strip.id)}
        <MixerStrip
          {api}
          id={strip.id}
          name={strip.name}
          gain={strip.gain}
          control={controlFor(strip.id)}
          {loop}
          {strip}
          soloed={solo.has(strip.id)}
          soloActive={solo.size > 0}
          selected={selectedIds.has(strip.id)}
          follow={strip.animated ? follow : undefined}
          liveGain={strip.animated ? liveGain(strip.id) : undefined}
          onMute={() => toggleMute(strip)}
          onSolo={(exclusive) => setSolo(nextSolo(solo, strip.id, exclusive))}
          onKey={() => toggleKey(strip)}
          onSelect={() => select(strip)}
        />
      {/each}
    </div>
    <div class="mixer-master">
      <MixerStrip {api} id={source.MASTER} name="Master" gain={masterGain} control={masterControl} {loop} />
    </div>
  {:else}
    <div class="mixer-empty">No audio in this composition</div>
  {/if}
</div>

<style>
  .mixer {
    /* The unlit meter: a visible channel on paper, a quiet one in the dark. */
    --mx-track: var(--ink-2);
    container: mixer / size;
    width: 100%;
    height: 100%;
    min-height: 0;
    display: flex;
    align-items: stretch;
  }

  :global(:root[data-theme="dark"]) .mixer { --mx-track: var(--ink-1); }

  .mixer-strips {
    flex: 1;
    min-width: 0;
    height: 100%;
    display: flex;
    gap: 2px;
    padding: 0 4px 0 6px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: thin;
    --fade-start: 0px;
    --fade-end: 0px;
    mask-image: linear-gradient(to right, transparent, #000 var(--fade-start), #000 calc(100% - var(--fade-end)), transparent);
  }

  .mixer-strips.fade-start { --fade-start: 18px; }
  .mixer-strips.fade-end { --fade-end: 18px; }

  .mixer-master {
    flex: none;
    height: 100%;
    padding: 0 8px 0 10px;
  }

  .mixer-empty {
    flex: 1;
    align-self: center;
    padding: 0 16px;
    text-align: center;
    font-size: var(--fs-sm);
    color: var(--tx-3);
  }
</style>
