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
      lead: 'A finished video. Or a live animation.',
      text: 'Render a clip to share, or export a JavaScript player or React component with playback controls and editable text and colors.',
    },
  ];

</script>

{#each features as f (f.id)}
  <article class="card" id={f.id} aria-labelledby="{f.id}-title">
    <header class="card-head">
      <div>
        <h2 id="{f.id}-title">{f.label}</h2>
        <p><span class="strong">{f.lead}</span> {f.text}</p>
      </div>
    </header>
    <div class="card-body"><AppPreview kind={f.id} bare /></div>
  </article>
{/each}
