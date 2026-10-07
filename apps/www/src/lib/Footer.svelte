<script lang="ts">
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
  // Fifteen seconds of ruler, the same span as the hero's timeline.
  const ticks = Array.from({ length: 16 }, (_, i) => i);

  $effect(() => { resolveDownload(); });
</script>

<footer class="site-footer">
  <div class="wrap">
    <!-- Bookends the hero: the playhead travels as the footer scrolls in and
         parks on the last frame at the bottom of the page. -->
    <div class="foot-ruler" aria-hidden="true">
      {#each ticks as t (t)}<span class="tick" class:major={t % 3 === 0} style:--t={t / 15}></span>{/each}
      <span class="foot-playhead"></span>
    </div>

    <div class="foot-main">
      <div class="foot-cta">
        <h2>Your move.</h2>
        <p>Free and open source, for Macs with Apple silicon.</p>
        <div class="cta">
          <a class="button primary lg" href={download.href}>Download for macOS{#if download.version} <span class="muted">{download.version}</span>{/if}</a>
          <a class="button lg" href={GITHUB} target="_blank" rel="noopener">Star on GitHub{#if stars.label} <span class="muted">{stars.label}</span>{/if}</a>
        </div>
      </div>

      <nav class="foot-nav" aria-label="Footer">
        {#each columns as column (column.title)}
          <div>
            <h3>{column.title}</h3>
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
    </div>

    <div class="foot-base">
      <a class="foot-mark" href="/#top" aria-label="Powermove home"><img src="/powermove-logo.svg" width="16" height="14" alt="" /></a>
      <span>© <span class="tabular">{year}</span> Iterative Computer</span>
      <span class="foot-meta">
        {#if download.version}<a href={RELEASES} target="_blank" rel="noopener">Powermove <span class="tabular">{download.version}</span></a>{/if}
        <a href="{GITHUB}/blob/main/LICENSE" target="_blank" rel="noopener">AGPL-3.0</a>
      </span>
    </div>
  </div>
</footer>
