<script lang="ts">
  // Sticky, scroll-driven: the section is several viewports tall, the card
  // pins, and scroll progress picks which shape the body should be.
  import LightsCard from './lights/LightsCard.svelte';

  const STEPS = [
    { key: 'prompt', title: 'Ask in words', text: 'Describe the change in the agent bar. It lands as an undoable edit.' },
    { key: 'node', title: 'Watch the passes', text: 'Every run is three steps you can open: read the project, render frames to check, then edit.' },
    { key: 'progress', title: 'Frames in the background', text: 'The agent renders frames to check its own work while you keep editing.' },
    { key: 'terminal', title: 'Mods on save', text: 'Build a panel or effect from the command line. Save, and it hot-reloads in place.' },
  ];

  let active = $state(0);
  let blocks = $state<Record<number, HTMLElement | undefined>>({});

  // Same driver as the Product rail: the block nearest the reading line wins.
  $effect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.filter((e) => e.isIntersecting).sort((a, b) => b.intersectionRatio - a.intersectionRatio);
        const next = visible[0]?.target.getAttribute('data-step');
        if (next != null) active = Number(next);
      },
      { rootMargin: '-35% 0px -45% 0px', threshold: [0.15, 0.35, 0.55, 0.75] },
    );
    for (let i = 0; i < STEPS.length; i++) { const el = blocks[i]; if (el) observer.observe(el); }
    return () => observer.disconnect();
  });
</script>

<section class="showcase" aria-labelledby="showcase-title">
  <div class="wrap showcase-layout">
    <div class="showcase-stick">
      <LightsCard target={active} />
    </div>
    <div class="showcase-copy">
      <h2 id="showcase-title"><span class="strong">One editor, many shapes.</span><br />Every surface is a mod.</h2>
      <ol class="showcase-steps">
        {#each STEPS as s, i (s.key)}
          <li data-step={i} bind:this={blocks[i]} data-active={active === i ? '' : undefined}>
            <span class="strong">{s.title}.</span> {s.text}
          </li>
        {/each}
      </ol>
    </div>
  </div>
</section>
