<script lang="ts">
  // Sticky, scroll-driven: the section is several viewports tall, the card
  // pins, and scroll progress picks which shape the body should be.
  import { onMount } from 'svelte';
  import LightsCard from './lights/LightsCard.svelte';

  const STEPS = [
    { key: 'node', title: 'Agent passes', text: 'Every run is three steps you can watch: read the project, render frames to check, then edit.' },
    { key: 'progress', title: 'Frames in the background', text: 'The agent renders frames to check its own work while you keep editing.' },
    { key: 'terminal', title: 'Mods on save', text: 'Build a panel or effect from the command line. Save, and it hot-reloads in place.' },
    { key: 'prompt', title: 'Ask in words', text: 'Describe the change in the agent bar. It lands as an undoable edit.' },
  ];

  let section = $state<HTMLElement>();
  let target = $state(0);
  let shown = $state(0);

  onMount(() => {
    let raf = 0;
    const update = () => {
      raf = 0;
      if (!section) return;
      const r = section.getBoundingClientRect();
      const travel = r.height - window.innerHeight;
      const p = travel > 0 ? Math.min(1, Math.max(0, -r.top / travel)) : 0;
      target = Math.min(STEPS.length - 1, Math.floor(p * STEPS.length));
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
        <h2 id="showcase-title"><span class="strong">One editor, many shapes.</span><br />Every surface is the same kind of mod.</h2>
        <ol class="showcase-steps">
          {#each STEPS as s, i (s.key)}
            <li data-active={shown === i ? '' : undefined}>
              <span class="strong">{s.title}.</span> {s.text}
            </li>
          {/each}
        </ol>
      </div>
      <LightsCard {target} onshow={(i) => (shown = i)} />
    </div>
  </div>
</section>
