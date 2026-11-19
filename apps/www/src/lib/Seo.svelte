<script lang="ts">
  import { absolute, jsonLd, OG_IMAGE, OG_IMAGE_ALT } from './site';

  type Props = {
    title: string;
    description: string;
    /** Site-relative path without a trailing slash, e.g. "/privacy". */
    path: string;
    image?: string;
    imageAlt?: string;
    type?: 'website' | 'article' | 'video.other';
    graph?: Record<string, unknown>[];
  };
  let { title, description, path, image = OG_IMAGE, imageAlt = OG_IMAGE_ALT, type = 'website', graph = [] }: Props = $props();

  const url = $derived(absolute(path));
  const imageUrl = $derived(absolute(image));
  const ld = $derived(graph.length ? `<script type="application/ld+json">${jsonLd(graph)}</` + `script>` : '');
</script>

<svelte:head>
  <title>{title}</title>
  <meta name="description" content={description} />
  <link rel="canonical" href={url} />
  <meta property="og:type" content={type} />
  <meta property="og:site_name" content="Powermove" />
  <meta property="og:url" content={url} />
  <meta property="og:title" content={title} />
  <meta property="og:description" content={description} />
  <meta property="og:image" content={imageUrl} />
  {#if image === OG_IMAGE}
    <meta property="og:image:width" content="1200" />
    <meta property="og:image:height" content="630" />
  {/if}
  <meta property="og:image:alt" content={imageAlt} />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:title" content={title} />
  <meta name="twitter:description" content={description} />
  <meta name="twitter:image" content={imageUrl} />
  {#if ld}{@html ld}{/if}
</svelte:head>
