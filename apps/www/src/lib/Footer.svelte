<script lang="ts">
  import { squircle } from '$lib/squircle';
  import { GITHUB, RELEASES } from '$lib/links';
  import { download, resolveDownload, stars } from '$lib/download.svelte';

  type Link = { label: string; href: string; external?: boolean };

  const year = new Date().getFullYear();
  const columns: { title: string; links: Link[] }[] = [
    {
      title: 'Product',
      links: [
        { label: 'Editor', href: '/#editor' },
        { label: 'Agent', href: '/#agent' },
        { label: 'Extensions', href: '/#extensions' },
        { label: 'Export', href: '/#export' },
      ],
    },
    {
      title: 'Open source',
      links: [
        { label: 'Source code', href: GITHUB, external: true },
        { label: 'Releases', href: RELEASES, external: true },
        { label: 'Launch video', href: 'https://www.youtube.com/watch?v=r2t1dhxHjkQ', external: true },
      ],
    },
    {
      title: 'Company',
      links: [
        { label: 'Iterative Computer', href: 'https://iterative.computer', external: true },
        { label: 'hello@iterative.computer', href: 'mailto:hello@iterative.computer' },
        { label: 'Privacy', href: '/privacy' },
        { label: 'Terms', href: '/terms' },
      ],
    },
  ];

  $effect(() => { resolveDownload(); });
</script>

<!-- The page sinks back into the stage it opened on: paper warming through our
     orange into near-black, with everything set at the header's scale. -->
<footer class="site-footer">
  <div class="foot-dusk" aria-hidden="true"></div>
  <div class="foot-inner">
    <div class="foot-lead">
      <a class="foot-mark" href="/#top" aria-label="Powermove home"><span aria-hidden="true"></span></a>
      <p class="foot-line">Powermove is free and open source, for Macs with Apple silicon.</p>
      <div class="foot-cta">
        <a class="pill light" use:squircle href={download.href}>Download for Mac</a>
        <a class="pill glass" use:squircle href={GITHUB} target="_blank" rel="noopener">Star on GitHub{#if stars.label} <span class="pill-count tabular">{stars.label}</span>{/if}</a>
      </div>
    </div>

    <nav class="foot-nav" aria-label="Footer">
      {#each columns as column (column.title)}
        <div>
          <h2>{column.title}</h2>
          <ul>
            {#each column.links as link (link.href)}
              <li>
                {#if link.external}
                  <a href={link.href} target="_blank" rel="noopener">{link.label}<span class="ext" aria-hidden="true">↗</span></a>
                {:else}
                  <a href={link.href}>{link.label}</a>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </nav>

    <div class="foot-base">
      <span>© <span class="tabular">{year}</span> Iterative Computer</span>
      <span class="foot-meta">
        {#if download.version}<a href={RELEASES} target="_blank" rel="noopener">Version <span class="tabular">{download.version}</span></a>{/if}
        <a href="{GITHUB}/blob/main/LICENSE" target="_blank" rel="noopener">AGPL-3.0</a>
      </span>
    </div>
  </div>
</footer>
