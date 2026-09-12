<script lang="ts">
  import '@powermove/tokens/tokens.css';
  import '../app.css';

  let { children } = $props();
</script>

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
