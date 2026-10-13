<script lang="ts">
  import '@powermove/tokens/tokens.css';
  import '../app.css';

  let { children } = $props();

  // Cloudflare Web Analytics. The site token is public but lives in the
  // gitignored .env so the secret scan stays strict; unset means no beacon.
  const beaconToken = import.meta.env.VITE_CF_BEACON_TOKEN;
</script>

<svelte:head>
  {#if beaconToken}
    {@html `<script defer src="https://static.cloudflareinsights.com/beacon.min.js" data-cf-beacon='${JSON.stringify({ token: beaconToken })}'></script>`}
  {/if}
</svelte:head>

{@render children()}

<style>
  /* Geist, self-hosted. Replaces next/font/google: same family name the hero
     scene and poster reference ("Geist"), exposed as --font-geist-sans. */
  @font-face {
    font-family: Geist;
    src: url('/fonts/geist-latin-variable.woff2') format('woff2');
    font-weight: 100 900;
    font-style: normal;
    font-display: swap;
  }
  :global(:root) {
    --font-geist-sans: Geist;
    --font-geist-mono: ui-monospace, 'SF Mono', Menlo, monospace;
  }
</style>
