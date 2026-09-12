<script lang="ts">
  // One body that becomes four components. A single element carries the glow
  // and animates its width, height and corner radius between shapes; the
  // variants are only contents plus the shape the body should take to hold them.
  import { onMount } from 'svelte';
  import { buildLayers, rollPalette, type Layer } from './mask';

  type Variant = { key: string; radius: number | 'pill'; well?: boolean; control?: boolean };
  // Ordered so the body never makes the same kind of move twice running:
  // a pill, a wide node, a short row, then the sunken terminal.
  const VARIANTS: Variant[] = [
    { key: 'prompt', radius: 'pill', control: true },
    { key: 'node', radius: 12 },
    { key: 'progress', radius: 20 },
    { key: 'terminal', radius: 10, well: true },
  ];

  const PULSE_MS = 1600;
  const FADE_OUT_MS = 160;
  const MORPH_MS = 380;
  const SETTLE_MS = 80;
  /** How far into a pulse the handover starts when the scroll asks for a new shape. */
  const HANDOVER_AT = 140;
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
  // Resolve every radius to a real number against the box, so a pill travels linearly instead of snapping from 9999px.
  const radius = $derived(size ? (variant.radius === 'pill' ? (size.h + 2) / 2 : Math.min(variant.radius, (size.h + 2) / 2)) : 0);

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
    rollPalette(card, true);
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
  // new contents rise in once it has settled. If the target changes while the
  // body is already moving, the morph is retargeted in flight rather than
  // finishing and starting over, so a fast scroll reads as one motion.
  let settle: number | null = null;
  const clamp = (n: number) => Math.max(0, Math.min(VARIANTS.length - 1, n));

  function land() {
    settle = null;
    rebuildMask();
    morphing = false;
    gen++;
    showing = true;
    busy = false;
    if (clamp(target) !== slot) handover(); else scheduleIdle();
  }

  function morphTo(next: number) {
    slot = next;
    if (settle !== null) window.clearTimeout(settle);
    settle = window.setTimeout(land, MORPH_MS + SETTLE_MS);
  }

  function handover() {
    if (busy || !sizes) return;
    if (clamp(target) === slot) return;
    busy = true;
    if (idle !== null) { window.clearTimeout(idle); idle = null; }
    pulse();
    after(reduced ? 0 : HANDOVER_AT, () => {
      showing = false;
      after(FADE_OUT_MS, () => {
        body?.removeAttribute('data-playing');
        morphing = true;
        morphTo(clamp(target));
      });
    });
  }

  $effect(() => {
    const next = clamp(target);
    if (!sizes) return;
    if (morphing) { if (next !== slot) morphTo(next); }
    else handover();
  });

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
      if (settle !== null) window.clearTimeout(settle);
    };
  });

  const WORD_MS = 45;
</script>

{#snippet rising(text: string, cls: string, delay = 0)}
  <span class="lc-rise {cls}">
    {#each text.split(' ') as word, i (i)}
      <span class="lc-word" style:animation-delay={`${delay + (reduced ? 0 : i * WORD_MS)}ms`}>{word}</span>{' '}
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
    <span class="lc-progress">
      <span class="lc-row">
        {@render rising('Rendering frames', 'lc-body-text')}
        <span class="lc-trail lc-fade" style:animation-delay="120ms">142 / 300</span>
      </span>
      <span class="lc-bar lc-fade" style:animation-delay="180ms"><i></i></span>
    </span>
  {:else if key === 'terminal'}
    <span class="lc-terminal">
      <span class="lc-cmd lc-fade"><span class="lc-prompt-sign">$</span> pm build quiet-timeline</span>
      <span class="lc-cmd lc-fade" style:animation-delay="80ms"><span class="lc-prompt-sign">·</span> 3 files, 41 ms</span>
      {@render rising('Hot reloaded in place', 'lc-out', 160)}
    </span>
  {:else}
    <span class="lc-row lc-prompt">
      {@render rising('Push the title in slowly', 'lc-body-text')}
      <span class="lc-send lc-fade" style:animation-delay="200ms"><svg viewBox="0 0 12 12" aria-hidden="true"><path d="M6 9.5V2.5M6 2.5 3 5.5M6 2.5 9 5.5" /></svg></span>
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
    <div class="lc-face" class:well={variant.well}>
      {#key gen}
        <div class="lc-face-pad lc-contents" class:control={variant.control} style:opacity={showing ? 1 : 0} style:content-visibility={morphing ? 'hidden' : undefined} style:--f={`${FADE_OUT_MS}ms`}>
          {@render content(variant.key)}
        </div>
      {/key}
    </div>
  </div>
</div>
