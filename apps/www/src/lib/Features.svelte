<script lang="ts">
  import AppPreview from './AppPreview.svelte';

  type Feature = {
    id: string;
    label: string;
    lead: string;
    text: string;
    image?: { src: string; alt: string; width: number; height: number };
    preview?: 'extensions' | 'performance' | 'export';
  };

  const features: Feature[] = [
    {
      id: 'agent',
      label: 'An editor you customize with AI',
      lead: "It's up to you.",
      text: 'Connect a coding agent and do basically anything. Make extensions, restyle the interface, or even generate effects and shaders.',
      image: {
        src: '/agent-lens-flare.jpg',
        alt: 'Custom Lens Flare effect generated in Powermove',
        width: 1920,
        height: 1080,
      },
    },
    {
      id: 'extensions',
      label: 'Only what you need',
      lead: 'Keep your setup simple.',
      text: 'Turn extensions off in Settings when you don’t need them, and switch them back on whenever you do.',
      preview: 'extensions',
    },
    {
      id: 'performance',
      label: 'Blazing fast',
      lead: 'Built natively in Rust and Metal.',
      text: 'Uses over 6× less RAM than After Effects, scrubs 200-layer timelines smoothly without waiting on RAM previews, and exports over 3× faster.',
      preview: 'performance',
    },
    {
      id: 'export',
      label: 'Take it beyond the editor',
      lead: 'A finished video. Or the whole project.',
      text: 'Render MP4, WebM or ProRes with sound, a still or a PNG sequence, or save an editable project with every layer and its media.',
      preview: 'export',
    },
  ];
</script>

{#each features as f (f.id)}
  <article class="row" id={f.id} aria-labelledby="{f.id}-title">
    <div class="row-copy">
      <h3 id="{f.id}-title">{f.label}</h3>
      <p><span class="strong">{f.lead}</span> {f.text}</p>
    </div>
    <div class="row-media">
      {#if f.image}
        <img
          class="feature-photo"
          src={f.image.src}
          alt={f.image.alt}
          width={f.image.width}
          height={f.image.height}
          loading="lazy"
        />
      {:else if f.preview}
        <AppPreview kind={f.preview} bare />
      {/if}
    </div>
  </article>
{/each}
