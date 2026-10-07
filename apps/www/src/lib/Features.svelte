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

<section class="features" aria-labelledby="features-title">
  <div class="wrap">
    <h2 id="features-title" class="chapter-title"><span class="strong">Welcome to malleable software.</span> Ask for a panel, an effect, or a whole workflow, and it loads into the app you’re already using.</h2>
    <div class="blocks">
      {#each features as f (f.id)}
        <article class="block" id={f.id}>
          <div class="copy">
            <h3>{f.label}</h3>
            <p class="lead"><span class="strong">{f.lead}</span> {f.text}</p>
          </div>
          <AppPreview kind={f.id} />
        </article>
      {/each}
    </div>
  </div>
</section>
