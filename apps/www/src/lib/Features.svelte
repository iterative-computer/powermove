<script lang="ts">
  import AppPreview from './AppPreview.svelte';

  type Feature = { id: string; label: string; lead: string; text: string };

  const features: Feature[] = [
    {
      id: 'agent', label: 'An editor you can rewrite',
      lead: "It's up to you.",
      text: 'Connect a coding agent and do basically anything. Make extensions, restyle the interface, or even generate effects and shaders.',
    },
    {
      id: 'extensions', label: 'Only what you need',
      lead: 'Keep your setup simple.',
      text: 'Turn extensions off in Settings when you don’t need them, and switch them back on whenever you do.',
    },
    {
      id: 'export', label: 'Take it beyond the editor',
      lead: 'A finished video. Or the whole project.',
      text: 'Render MP4, WebM or ProRes with sound, a still or a PNG sequence, or save an editable project with every layer and its media.',
    },
  ];

</script>

{#each features as f (f.id)}
  <article class="row" id={f.id} aria-labelledby="{f.id}-title">
    <div class="row-copy">
      <h3 id="{f.id}-title">{f.label}</h3>
      <p><span class="strong">{f.lead}</span> {f.text}</p>
    </div>
    <div class="row-media"><AppPreview kind={f.id} bare /></div>
  </article>
{/each}
