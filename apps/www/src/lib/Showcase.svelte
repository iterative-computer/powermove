<script lang="ts">
  // Pinned section. Scroll progress through it picks the active step, the
  // rail dot arcs to it exactly as on the Product rail, and the card follows.
  import { onMount } from 'svelte';
  import LightsCard from './lights/LightsCard.svelte';

  const STEPS = [
    { key: 'prompt', title: 'Ask in words', text: 'Describe the change. It lands as an undoable edit.' },
    { key: 'node', title: 'Watch the passes', text: 'Read the project, render frames to check, then edit.' },
    { key: 'progress', title: 'Frames in the background', text: 'The agent checks its own work while you keep editing.' },
    { key: 'terminal', title: 'Mods on save', text: 'Build a panel or effect. Save, and it hot-reloads in place.' },
  ];

  let section = $state<HTMLElement>();
  let active = $state(0);

  let railItems = $state<Record<number, HTMLElement | undefined>>({});
  let marker = $state<HTMLElement | undefined>();
  let markerY: number | undefined;
  let flight: Animation | undefined;

  // Same marker as the Product rail: one arc, momentum carried through interruptions.
  $effect(() => {
    const el = railItems[active];
    if (!el || !marker) return;
    // Sit beside the title line, not the middle of the two-line block.
    const title = el.querySelector<HTMLElement>('.strong') ?? el;
    const y = title.offsetTop + title.offsetHeight / 2;
    if (markerY === undefined) { markerY = y; marker.style.transform = `translateY(${y}px)`; return; }
    if (y === markerY) return;
    let fromX = 0, fromY = markerY;
    markerY = y;
    const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
    const midFlight = flight?.playState === 'running';
    if (midFlight) {
      const m = new DOMMatrix(getComputedStyle(marker).transform);
      fromX = m.m41; fromY = m.m42;
    }
    flight?.cancel();
    if (reduce) { marker.style.transform = `translateY(${y}px)`; return; }
    const steps = 24;
    const frames = Array.from({ length: steps + 1 }, (_, i) => {
      const p = i / steps;
      const x = fromX * (1 - p) - 10 * Math.sin(Math.PI * p);
      return { transform: `translate(${x.toFixed(2)}px, ${(fromY + (y - fromY) * p).toFixed(2)}px)` };
    });
    marker.style.transform = `translateY(${y}px)`;
    flight = marker.animate(frames, {
      duration: midFlight ? 420 : 520,
      easing: midFlight ? 'cubic-bezier(.2,.6,.2,1)' : 'cubic-bezier(.4,0,.2,1)',
    });
  });

  onMount(() => {
    const desktop = matchMedia('(min-width: 1024px)');
    let raf = 0;
    const update = () => {
      raf = 0;
      if (!section || !desktop.matches) return;
      const r = section.getBoundingClientRect();
      const travel = r.height - window.innerHeight;
      const p = travel > 0 ? Math.min(1, Math.max(0, -r.top / travel)) : 0;
      active = Math.min(STEPS.length - 1, Math.floor(p * STEPS.length));
    };
    const onScroll = () => { if (!raf) raf = requestAnimationFrame(update); };
    update();
    addEventListener('scroll', onScroll, { passive: true });
    addEventListener('resize', onScroll);
    return () => { removeEventListener('scroll', onScroll); removeEventListener('resize', onScroll); if (raf) cancelAnimationFrame(raf); };
  });
</script>

<section class="showcase" bind:this={section} aria-labelledby="showcase-title">
  <div class="showcase-sticky">
    <div class="wrap showcase-layout">
      <div class="showcase-copy">
        <h2 id="showcase-title"><span class="strong">One editor, many shapes.</span><br />Every surface is a mod.</h2>
        <div class="rail-items showcase-rail">
          <span class="rail-marker" bind:this={marker} aria-hidden="true"></span>
          {#each STEPS as s, i (s.key)}
            <div class="showcase-step" bind:this={railItems[i]} data-active={active === i ? '' : undefined}>
              <span class="strong">{s.title}</span>
              <span class="showcase-step-text">{s.text}</span>
            </div>
          {/each}
        </div>
      </div>
      <LightsCard target={active} />
      <div class="showcase-controls">
        <div class="showcase-choices" role="group" aria-label="Explore the editor">
          {#each STEPS as s, i (s.key)}
            <button type="button" aria-pressed={active === i} onclick={() => active = i}>{s.title}</button>
          {/each}
        </div>
        <p class="showcase-caption" aria-live="polite">{STEPS[active].text}</p>
      </div>
    </div>
  </div>
</section>
