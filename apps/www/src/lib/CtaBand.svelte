<script lang="ts">
  import { download, resolveDownload, stars } from './download.svelte';
  import { GITHUB } from './links';

  let {
    title = 'The video editor you customize with AI.',
    text = 'Powermove is free and open source, for Macs with Apple silicon. Connect Claude or Codex and ask for what you need.',
  }: { title?: string; text?: string } = $props();

  $effect(() => { resolveDownload(); });
</script>

<section class="page-section" aria-labelledby="cta-title">
  <div class="cta-band">
    <h2 id="cta-title">{title}</h2>
    <p>{text}</p>
    <div class="hero-cta">
      <a class="pill light" href={download.href}>Download for Mac</a>
      <a class="pill glass" href={GITHUB} target="_blank" rel="noopener">Star on GitHub{#if stars.label} <span class="pill-count tabular">{stars.label}</span>{/if}</a>
    </div>
  </div>
</section>
