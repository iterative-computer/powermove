<script lang="ts">
  import Header from '$lib/Header.svelte';
  import Hero from '$lib/Hero.svelte';
  import Footer from '$lib/Footer.svelte';
  import Features from '$lib/Features.svelte';
  import { GITHUB } from '$lib/links';
  import { download, resolveDownload, stars } from '$lib/download.svelte';


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
  <title>Powermove</title>
  <meta name="description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <link rel="canonical" href="https://trypowermove.com/" />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content="Powermove" />
  <meta property="og:url" content="https://trypowermove.com/" />
  <meta property="og:title" content="Powermove" />
  <meta property="og:description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <meta property="og:image" content="https://trypowermove.com/og.png" />
  <meta property="og:image:width" content="1200" />
  <meta property="og:image:height" content="630" />
  <meta property="og:image:alt" content="The Powermove editor with a layer stack, inspector, agent panel, and keyframe timeline." />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:title" content="Powermove" />
  <meta name="twitter:description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <meta name="twitter:image" content="https://trypowermove.com/og.png" />
</svelte:head>

<Header />

<main id="top">
  <section class="hero" aria-labelledby="headline">
    <div class="wrap">
      <h1 id="headline" class="sr-only">Shape your video editor</h1>
      <Hero />
      <div class="hero-intro">
        <p class="lede">An iterative video editor for macOS. Ask an agent to add or fork panels, effects, and workflows, and it loads them into the running app. Steer it when it’s wrong, undo anything.</p>
        <div class="cta">
          <a class="button primary lg" href={download.href}>Download for Mac{#if download.version} <span class="muted">{download.version}</span>{/if}</a>
          <a class="button lg" href={GITHUB} target="_blank" rel="noopener">GitHub{#if stars.label} <span class="muted">{stars.label}</span>{/if}</a>
        </div>
      </div>
    </div>
    <figure class="stage" id="editor">
      <img class="editor-screenshot" src="/editor-screenshot.png" alt="Powermove editor in a light theme, showing the Change literally anything composition, agent conversation, timeline, properties, and effects." width="1920" height="1305" fetchpriority="high" />
    </figure>
  </section>

  <Features />

  <section class="kernel" aria-labelledby="kernel-title">
    <div class="wrap kernel-layout">
      <div class="kernel-copy">
        <h2 id="kernel-title" class="chapter-title"><span class="strong">A deliberately tiny kernel.</span> Everything above it is an extension, including the parts we wrote.</h2>
        <p class="lead">Timeline, effects, inspector, the UI itself. The agent writes new extensions the way we built the editor, through a typed, undoable tool layer that inspects, renders, and edits the live project.</p>
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
</main>

<Footer />
