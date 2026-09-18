<script lang="ts">
  import { ArrowUpRight, FileCode2, Layers2, Puzzle, Undo2 } from '@lucide/svelte';
  import Header from '$lib/Header.svelte';
  import HeroAnimation from '$lib/HeroAnimation.svelte';
  import LogoCloud from '$lib/LogoCloud.svelte';
  import Features from '$lib/Features.svelte';
  import Showcase from '$lib/Showcase.svelte';
  import { GITHUB, PRODUCT_HUNT, PRODUCT_HUNT_BADGE } from '$lib/links';
  import { download, resolveDownload } from '$lib/download.svelte';


  const year = new Date().getFullYear();

  $effect(() => { resolveDownload(); });

  let stack = $state<HTMLElement | undefined>();
  let stackIn = $state(false);

  $effect(() => {
    if (!stack) return;
    const observer = new IntersectionObserver(
      ([entry]) => { if (entry.isIntersecting) { stackIn = true; observer.disconnect(); } },
      { threshold: 0.5 },
    );
    observer.observe(stack);
    return () => observer.disconnect();
  });
</script>

<svelte:head>
  <title>Powermove — Make your move.</title>
  <meta name="description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <link rel="canonical" href="https://trypowermove.com/" />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content="Powermove" />
  <meta property="og:url" content="https://trypowermove.com/" />
  <meta property="og:title" content="Powermove — Make your move." />
  <meta property="og:description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <meta property="og:image" content="https://trypowermove.com/og.png" />
  <meta property="og:image:width" content="1200" />
  <meta property="og:image:height" content="630" />
  <meta property="og:image:alt" content="The Powermove editor with a layer stack, inspector, agent panel, and keyframe timeline." />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:title" content="Powermove — Make your move." />
  <meta name="twitter:description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <meta name="twitter:image" content="https://trypowermove.com/og.png" />
</svelte:head>

<Header />

<main id="top">
  <section class="hero" aria-labelledby="headline">
    <a class="eyebrow ph" href={PRODUCT_HUNT} target="_blank" rel="noopener"><img src={PRODUCT_HUNT_BADGE} alt="Powermove on Product Hunt" width="250" height="54" /></a>
    <h1 id="headline" class="sr-only">Shape your video editor</h1>
    <HeroAnimation />
    <div class="wrap hero-intro">
      <p class="lede">A motion editor with an agent in the loop and you in the driver’s seat. Real layers, editable keyframes, and an interface that rewrites itself around the work.</p>
      <div class="cta center">
        <a class="button primary lg" href={download.href}>Download for macOS{#if download.version} <span class="muted">{download.version}</span>{/if}</a>
        <a class="button lg" href={GITHUB} target="_blank" rel="noopener">Star on GitHub <ArrowUpRight size={15} /></a>
      </div>
    </div>
    <figure class="stage" id="editor">
      <!-- Launch video (2026-09-18). To revert, delete the iframe and uncomment the img below. -->
      <div class="stage-video">
        <iframe
          src="https://www.youtube-nocookie.com/embed/r2t1dhxHjkQ?autoplay=1&mute=1&loop=1&playlist=r2t1dhxHjkQ&controls=1&rel=0&playsinline=1"
          title="Powermove launch video"
          allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
          allowfullscreen
          loading="eager"
        ></iframe>
      </div>
      <!-- <img src="/editor-hero.png" alt="Powermove editor over a desert night sky, with a cinematic title, layer stack, property inspector, agent panel, and keyframe timeline." width="1920" height="1280" fetchpriority="high" /> -->
    </figure>
    <LogoCloud />
  </section>

  <Showcase />

  <Features />

  <section class="principles" aria-labelledby="principles-title">
    <div class="wrap principles-layout">
      <div class="principles-copy">
        <h2 id="principles-title"><span class="strong">Yours to change.</span><br />Every layer of it.</h2>
        <ul class="principle-list">
          <li><Undo2 size={16} strokeWidth={1.75} aria-hidden="true" /><div><span class="strong">Undoable.</span> Agent edits and yours share one history. Nothing happens off the record.</div></li>
          <li><Layers2 size={16} strokeWidth={1.75} aria-hidden="true" /><div><span class="strong">Real layers.</span> Generated work is an ordinary composition, not a rendered clip.</div></li>
          <li><Puzzle size={16} strokeWidth={1.75} aria-hidden="true" /><div><span class="strong">Personal software.</span> Built-in panels are mods. Fork one, or ask for a new one.</div></li>
          <li><FileCode2 size={16} strokeWidth={1.75} aria-hidden="true" /><div><span class="strong">Yours on disk.</span> Projects save as files with embedded media and a kept backup.</div></li>
        </ul>
      </div>
      <div class="stack" bind:this={stack} data-in={stackIn ? '' : undefined} aria-hidden="true">
        <div class="slab yours" style="--i:4"><span class="slab-name">Your mods</span><span class="slab-note">Yours to add. Same API as everything below</span></div>
        <div class="slab" style="--i:3"><span class="slab-name">Panels</span><span class="slab-note">Timeline, inspector, agent, media</span></div>
        <div class="slab" style="--i:2"><span class="slab-name">Effects and transitions</span><span class="slab-note">Blur, grain, whip pan, dissolve</span></div>
        <div class="slab" style="--i:1"><span class="slab-name">Theme and keymap</span><span class="slab-note">Dusk, Vim, yours</span></div>
        <div class="slab kernel" style="--i:0"><span class="slab-name">Kernel</span><span class="slab-note">Project store, typed edits, undo, compositor</span></div>
      </div>
    </div>
  </section>

  <section class="final" aria-labelledby="final-title">
    <div class="wrap">
      <h2 id="final-title">Make your move.</h2>
      <p class="final-note">Free and open source for macOS.</p>
      <div class="cta center">
        <a class="button primary lg" href={download.href}>Download for macOS</a>
        <a class="button lg" href={PRODUCT_HUNT} target="_blank" rel="noopener">Upvote on Product Hunt <ArrowUpRight size={15} /></a>
        <a class="button lg" href={GITHUB} target="_blank" rel="noopener">Star on GitHub <ArrowUpRight size={15} /></a>
      </div>
    </div>
  </section>

  <footer>
    <div class="wrap foot">
      <a class="logo" href="/#top" aria-label="Powermove home"><img src="/powermove-logo.svg" width="24" height="21" alt="" /></a>
      <div>
        <span class="foot-title">Product</span>
        <ul><li><a href="#editor">Editor</a></li><li><a href="#agent">Agent</a></li><li><a href="#mods">Mods</a></li><li><a href="#export">Export</a></li></ul>
      </div>
      <div>
        <span class="foot-title">Company</span>
        <ul><li><a href={GITHUB} target="_blank" rel="noopener">GitHub</a></li><li><a href={PRODUCT_HUNT} target="_blank" rel="noopener">Product Hunt</a></li><li><a href="mailto:hello@iterative.computer">hello@iterative.computer</a></li><li><a href="/privacy">Privacy</a></li><li><a href="/terms">Terms</a></li></ul>
      </div>
      <div class="foot-copy">© Iterative Computer {year}. Open source.</div>
    </div>
  </footer>
</main>
