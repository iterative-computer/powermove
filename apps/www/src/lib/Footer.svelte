<script lang="ts">
  import { DISCORD, GITHUB, RELEASES } from '$lib/links';
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
        { label: 'Pricing', href: '/#pricing' },
        { label: 'FAQ', href: '/#faq' },
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
        { label: 'Join our Discord', href: DISCORD, external: true },
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
        <a class="pill light" href={download.href}>Download for Mac</a>
        <a class="pill glass" href={GITHUB} target="_blank" rel="noopener">Star on GitHub{#if stars.label} <span class="pill-count tabular">{stars.label}</span>{/if}</a>
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
                  <a href={link.href} target="_blank" rel="noopener">{link.label}<!-- IconArrowUpRight, @pierre/icons 0.9.0 (Apache-2.0) --><svg class="ext" viewBox="0 0 16 16" aria-hidden="true"><path d="M5 4.75C5 4.33579 5.33579 4 5.75 4H11.25C11.6642 4 12 4.33579 12 4.75V10.25C12 10.6642 11.6642 11 11.25 11C10.8358 11 10.5 10.6642 10.5 10.25V6.56066L5.53033 11.5303C5.23744 11.8232 4.76256 11.8232 4.46967 11.5303C4.17678 11.2374 4.17678 10.7626 4.46967 10.4697L9.43934 5.5H5.75C5.33579 5.5 5 5.16421 5 4.75Z" fill="currentColor"/></svg></a>
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
