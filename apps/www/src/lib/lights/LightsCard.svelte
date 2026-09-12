<script lang="ts">
  // One body that becomes four components. A single element carries the glow
  // and animates its width, height and corner radius between shapes; the
  // variants are only contents plus the shape the body should take to hold them.
  import { onMount } from 'svelte';
  import { buildLayers, rollPalette, type Layer } from './mask';

  type Variant = { key: string; radius: number; dark?: boolean; control?: boolean };
  const VARIANTS: Variant[] = [
    { key: 'node', radius: 16 },
    { key: 'progress', radius: 15 },
    { key: 'terminal', radius: 16, dark: true },
    { key: 'prompt', radius: 18, control: true },
  ];

  const PULSE_MS = 1600;
  const FADE_OUT_MS = 160;
  const MORPH_MS = 380;
  const SETTLE_MS = 80;
  /** How far into a pulse the handover starts when the scroll asks for a new shape. */
  const HANDOVER_AT = 520;
  /** Idle pulses while resting on a shape. */
  const IDLE_GAP_MS = 2600;

  let { target = 0 }: { target?: number } = $props();

  let card = $state<HTMLElement>();
  let body = $state<HTMLElement>();
  let probe = $state<HTMLElement>();

  let slot = $state(0);
  let showing = $state(true);
  let morphing = $state(false);
  let gen = $state(0);
  let sizes = $state<{ w: number; h: number }[] | null>(null);
  let layers = $state<Layer[]>([]);
  let reduced = $state(false);

  const variant = $derived(VARIANTS[slot]);
  const size = $derived(sizes?.[slot]);
  const radius = $derived(size ? Math.min(variant.radius, (size.h + 2) / 2) : 0);

  let busy = false;
  let onScreen = false;
  let timers: number[] = [];
  let idle: number | null = null;

  const after = (ms: number, fn: () => void) => { timers.push(window.setTimeout(fn, ms)); };
  const clearTimers = () => { for (const t of timers) window.clearTimeout(t); timers = []; };

  function measure() {
    if (!probe) return;
    const boxes = Array.from(probe.querySelectorAll<HTMLElement>('[data-probe]')).map((el) => {
      const r = el.getBoundingClientRect();
      return { w: Math.round(r.width), h: Math.round(r.height) };
    });
    if (boxes.length === VARIANTS.length && boxes.every((b) => b.w && b.h)) sizes = boxes;
  }

  function rebuildMask() {
    if (!size) return;
    layers = buildLayers(size.w + 2, size.h + 2, radius);
  }

  function pulse() {
    if (!body || !card || reduced || !onScreen) return;
    rollPalette(card);
    body.dataset.playing = 'false';
    // Two frames, or the writes coalesce and nothing restarts.
    requestAnimationFrame(() => requestAnimationFrame(() => { if (body && !reduced) body.dataset.playing = 'true'; }));
  }

  function scheduleIdle() {
    if (idle !== null) window.clearTimeout(idle);
    idle = window.setTimeout(() => { idle = null; if (!busy && onScreen) { pulse(); scheduleIdle(); } }, PULSE_MS + IDLE_GAP_MS);
  }

  // Three beats that never overlap: the light pulses, the contents fade out
  // while the body is still the old shape, then the empty body morphs and the
  // new contents rise in once it has settled.
  function handover() {
    if (busy || !sizes) return;
    const next = Math.max(0, Math.min(VARIANTS.length - 1, target));
    if (next === slot) return;
    busy = true;
    if (idle !== null) { window.clearTimeout(idle); idle = null; }
    pulse();
    after(reduced ? 0 : HANDOVER_AT, () => {
      showing = false;
      after(FADE_OUT_MS, () => {
        body?.removeAttribute('data-playing');
        morphing = true;
        slot = next;
        after(MORPH_MS + SETTLE_MS, () => {
          rebuildMask();
          morphing = false;
          gen++;
          showing = true;
          busy = false;
          if (target !== slot) handover(); else scheduleIdle();
        });
      });
    });
  }

  $effect(() => { target; if (sizes) handover(); });

  onMount(() => {
    const mq = matchMedia('(prefers-reduced-motion: reduce)');
    const syncMotion = () => { reduced = mq.matches; if (reduced) body?.removeAttribute('data-playing'); };
    syncMotion();
    mq.addEventListener('change', syncMotion);

    measure();
    document.fonts?.ready.then(() => { measure(); requestAnimationFrame(rebuildMask); });
    requestAnimationFrame(rebuildMask);

    const io = new IntersectionObserver(([e]) => {
      onScreen = e.isIntersecting && !document.hidden;
      if (onScreen) { if (!busy) { pulse(); scheduleIdle(); } }
      else { if (idle !== null) window.clearTimeout(idle); idle = null; body?.removeAttribute('data-playing'); }
    }, { rootMargin: '120px' });
    if (card) io.observe(card);
    const onVis = () => { if (document.hidden) { onScreen = false; body?.removeAttribute('data-playing'); } };
    document.addEventListener('visibilitychange', onVis);

    return () => {
      mq.removeEventListener('change', syncMotion);
      io.disconnect();
      document.removeEventListener('visibilitychange', onVis);
      clearTimers();
      if (idle !== null) window.clearTimeout(idle);
    };
  });

  const CHAR_MS = 20;
</script>

{#snippet rising(text: string, cls: string, delay = 0)}
  <span class="lc-rise {cls}">
    {#each Array.from(text) as ch, i (i)}
      <span class="lc-ch" style:animation-delay={`${delay + (reduced ? 0 : i * CHAR_MS)}ms`} style:width={ch === ' ' ? '.28em' : undefined}>{ch === ' ' ? ' ' : ch}</span>
    {/each}
  </span>
{/snippet}

{#snippet content(key: string)}
  {#if key === 'node'}
    <span class="lc-node">
      <span class="lc-track" style:animation-delay="60ms"></span>
      {#each [['Read', 'done'], ['Render', 'live'], ['Edit', 'next']] as [label, state], i (label)}
        <span class="lc-step" data-state={state}>
          <span class="lc-dot lc-fade" style:animation-delay={`${100 + i * 70}ms`}>{#if state === 'live'}<span class="lc-ring"></span>{/if}</span>
          <span class="lc-step-label lc-fade" style:animation-delay={`${160 + i * 70}ms`}>{label}</span>
        </span>
      {/each}
    </span>
  {:else if key === 'progress'}
    <span class="lc-row lc-progress">
      {@render rising('Rendering', 'lc-body-text')}
      <span class="lc-trail lc-fade" style:animation-delay="120ms">45%</span>
      <svg viewBox="0 0 16 16" class="lc-spin" aria-hidden="true"><circle cx="8" cy="8" r="6" class="lc-spin-track" /><path d="M8 2A6 6 0 0 1 14 8" class="lc-spin-arc" /></svg>
    </span>
  {:else if key === 'terminal'}
    <span class="lc-terminal">
      <span class="lc-cmd"><span class="lc-prompt-sign">$</span> pm build quiet-timeline</span>
      {@render rising('hot reloaded', 'lc-out', 60)}
    </span>
  {:else}
    <span class="lc-row lc-prompt">
      {@render rising('push the title in slowly', 'lc-muted-text')}
      <span class="lc-send"><svg viewBox="0 0 12 12" aria-hidden="true"><path d="M6 9.5V2.5M6 2.5 3 5.5M6 2.5 9 5.5" /></svg></span>
    </span>
  {/if}
{/snippet}

<div class="lights-card" bind:this={card} aria-hidden="true">
  <div class="lc-probe" bind:this={probe}>
    {#each VARIANTS as v (v.key)}
      <div data-probe class="lc-face-pad" class:control={v.control}>{@render content(v.key)}</div>
    {/each}
  </div>

  <div
    class="ai-lights"
    bind:this={body}
    data-morphing={morphing ? '' : undefined}
    style:width={size ? `${size.w + 2}px` : undefined}
    style:height={size ? `${size.h + 2}px` : undefined}
    style:--r={`${radius}px`}
    style:--m={`${MORPH_MS}ms`}
  >
    {#each layers as l, i (i)}
      <span class="ai-lights-layer" style:inset={`${-l.pad}px`} style:mask-image={`url(${l.mask})`} style:-webkit-mask-image={`url(${l.mask})`}></span>
      <span class="ai-lights-layer mirror" style:inset={`${-l.pad}px`} style:mask-image={`url(${l.mask})`} style:-webkit-mask-image={`url(${l.mask})`}></span>
    {/each}
    <div class="lc-face" class:dark={variant.dark}>
      {#key gen}
        <div class="lc-face-pad lc-contents" class:control={variant.control} style:opacity={showing ? 1 : 0} style:content-visibility={morphing ? 'hidden' : undefined} style:--f={`${FADE_OUT_MS}ms`}>
          {@render content(variant.key)}
        </div>
      {/key}
    </div>
  </div>
</div>
