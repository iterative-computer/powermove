<script lang="ts">
  import { ArrowUpRight, FileCode2, Layers2, Puzzle, Undo2 } from '@lucide/svelte';
  import Header from '$lib/Header.svelte';
  import HeroAnimation from '$lib/HeroAnimation.svelte';
  import LogoCloud from '$lib/LogoCloud.svelte';
  import Features from '$lib/Features.svelte';


  const year = new Date().getFullYear();
</script>

<svelte:head>
  <title>Powermove — Make your move.</title>
  <meta name="description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite. In development." />
</svelte:head>

<Header />

<main id="top">
  <section class="hero" aria-labelledby="headline">
    <a class="eyebrow" href="https://github.com/motionerapp/Powermove" target="_blank" rel="noopener">Open source <span class="eyebrow-sep"></span><span class="muted">Star Powermove on GitHub</span> <ArrowUpRight size={13} aria-hidden="true" /></a>
    <h1 id="headline" class="sr-only">Shape your video editor</h1>
    <HeroAnimation />
    <div class="wrap hero-intro">
      <p class="lede">A motion editor with an agent in the loop and you in the driver’s seat. Real layers, editable keyframes, and an interface that rewrites itself around the work.</p>
      <div class="cta center">
        <a class="button primary lg" href="mailto:hello@motioner.app?subject=Powermove%20waitlist">Join the waitlist <ArrowUpRight size={15} /></a>
        <a class="button lg" href="#editor">See the editor</a>
      </div>
    </div>
    <figure class="stage" id="editor">
      <img src="/editor-hero.png" alt="Powermove editor over a desert night sky, with a cinematic title, layer stack, property inspector, agent panel, and keyframe timeline." width="1920" height="1280" fetchpriority="high" />
    </figure>
    <LogoCloud />
  </section>

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
      <div class="stack" aria-hidden="true">
        <div class="slab yours"><span class="slab-name">Your mods</span><span class="slab-note">Same API as everything below</span></div>
        <div class="slab"><span class="slab-name">Panels</span><span class="slab-note">Timeline, inspector, agent, media</span></div>
        <div class="slab"><span class="slab-name">Effects and transitions</span><span class="slab-note">Blur, grain, whip pan, dissolve</span></div>
        <div class="slab"><span class="slab-name">Theme and keymap</span><span class="slab-note">Dusk, Vim, yours</span></div>
        <div class="slab kernel"><span class="slab-name">Kernel</span><span class="slab-note">Project store, typed edits, undo, compositor</span></div>
      </div>
    </div>
  </section>

  <section class="facts" aria-labelledby="facts-title">
    <div class="wrap two">
      <h2 id="facts-title"><span class="strong">Small kernel.</span><br />Everything else is replaceable.</h2>
      <div class="facts-body">
        <p class="lead">The kernel owns the project store, a typed edit boundary with undo and revision checks, the GPU compositor, and the registries. The default theme, keymap, panels, effects, and transitions sit on top, as extensions.</p>
        <div class="stats">
          <div><div class="stat">Typed</div><p>One command boundary for people and agents</p></div>
          <div><div class="stat">Hot</div><p>Mods compile on save and reload in place</p></div>
          <div><div class="stat">Open</div><p>Source on GitHub. The editor, the site, and the player in one repo</p></div>
        </div>
      </div>
    </div>
  </section>

  <section class="final" aria-labelledby="final-title">
    <div class="wrap">
      <h2 id="final-title">Make your move.</h2>
      <div class="cta center">
        <a class="button primary lg" href="mailto:hello@motioner.app?subject=Powermove%20waitlist">Join the waitlist <ArrowUpRight size={15} /></a>
        <a class="button lg" href="#editor">See the editor</a>
      </div>
    </div>
  </section>

  <footer>
    <div class="wrap foot">
      <a class="logo" href="#top" aria-label="Powermove home"><img src="/powermove-logo.svg" width="24" height="21" alt="" /></a>
      <div>
        <span class="foot-title">Product</span>
        <ul><li><a href="#editor">Editor</a></li><li><a href="#agent">Agent</a></li><li><a href="#mods">Mods</a></li><li><a href="#export">Export</a></li></ul>
      </div>
      <div>
        <span class="foot-title">Company</span>
        <ul><li><a href="https://github.com/motionerapp/Powermove" target="_blank" rel="noopener">GitHub</a></li><li><a href="mailto:hello@motioner.app">hello@motioner.app</a></li><li><a href="mailto:hello@motioner.app?subject=Powermove%20waitlist">Waitlist</a></li></ul>
      </div>
      <div class="foot-copy">© Powermove {year}. Open source, in development.</div>
    </div>
  </footer>
</main>
