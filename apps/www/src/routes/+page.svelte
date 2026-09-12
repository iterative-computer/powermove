<script lang="ts">
  import { ArrowUpRight, Bot, FileCode2, History, Layers2, Puzzle, Sparkles, Undo2, Wand2, Zap } from '@lucide/svelte';
  import Header from '$lib/Header.svelte';
  import HeroAnimation from '$lib/HeroAnimation.svelte';
  import LogoCloud from '$lib/LogoCloud.svelte';

  type Feature = { id: string; rail: string; label: string; lead: string; text: string; points: { icon: typeof Bot; label: string }[] };

  const features: Feature[] = [
    {
      id: 'agent', rail: 'Agent', label: 'Your agent, in the editor',
      lead: 'Describe the change.',
      text: 'Powermove runs Codex or Claude Code natively. The agent sees the live project, renders real frames to check its work, and edits through the same typed commands you do.',
      points: [
        { icon: Bot, label: 'Reads project state and panel layout' },
        { icon: Wand2, label: 'Renders frames before it commits' },
        { icon: Undo2, label: 'Every edit lands on the Undo stack' },
      ],
    },
    {
      id: 'timeline', rail: 'Timeline', label: 'Real layers and keyframes',
      lead: 'Nothing is baked.',
      text: 'What the agent makes is a normal composition: layers, properties, keyframes, curves, expressions, groups, and 2.5D space. Open any of it and keep going by hand.',
      points: [
        { icon: Layers2, label: 'Groups, precomps, and 2.5D layers' },
        { icon: History, label: 'Take history for every generated pass' },
        { icon: Zap, label: 'GPU compositor with two-input transitions' },
      ],
    },
    {
      id: 'mods', rail: 'Mods', label: 'An editor you can rewrite',
      lead: 'Everything above the kernel is a mod.',
      text: 'The timeline, inspector, viewer, toolbar, theme, keymap, effects, and transitions all ship as extensions written against the same public API your own mods use. Replace any of them.',
      points: [
        { icon: Puzzle, label: 'A manifest plus TypeScript and Svelte files' },
        { icon: FileCode2, label: 'Compiled on save, hot-loaded in place' },
        { icon: Sparkles, label: 'Ask the agent to build the panel you want' },
      ],
    },
    {
      id: 'export', rail: 'Export', label: 'Leave as pixels or as code',
      lead: 'Render video, or ship the motion itself.',
      text: 'Export video through the built-in encoder, or hand off a scene as a self-contained web player or SVG. The headline at the top of this page is a Powermove scene running live.',
      points: [
        { icon: Zap, label: 'Video encoding with bundled ffmpeg' },
        { icon: FileCode2, label: 'Web player export with React and Svelte examples' },
        { icon: Layers2, label: 'SVG export for text and affine groups' },
      ],
    },
  ];

  const year = new Date().getFullYear();
</script>

<svelte:head>
  <title>Powermove — Make your move.</title>
  <meta name="description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite. In development." />
</svelte:head>

<Header />

<main id="top">
  <section class="hero" aria-labelledby="headline">
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

  <section class="features" aria-labelledby="features-title">
    <div class="wrap">
      <h2 id="features-title"><span class="strong">Built for the whole move.</span><br />From the first prompt to the final frame.</h2>
      <div class="feature-layout">
        <div class="rail">
          <div class="rail-title">Product</div>
          {#each features as f (f.id)}
            <a href={'#' + f.id}>{f.rail}</a>
          {/each}
        </div>
        <div class="blocks">
          {#each features as f, i (f.id)}
            <article class="block" id={f.id}>
              <div class="copy">
                <div>
                  <h3>{f.label}</h3>
                  <p class="lead"><span class="strong">{f.lead}</span> {f.text}</p>
                </div>
                <ul class="points">
                  {#each f.points as p (p.label)}
                    <li><p.icon size={16} strokeWidth={1.75} aria-hidden="true" />{p.label}</li>
                  {/each}
                </ul>
              </div>
              <div class="panel">
                {#if f.id === 'agent'}
                  <div class="mock composer" aria-hidden="true">
                    <div class="composer-text">Give the title a slow push-in and fade the kicker out at 2s.</div>
                    <div class="composer-row">
                      <span class="chip">All panels</span>
                      <span class="chip">Claude Code</span>
                      <span class="send"><ArrowUpRight size={14} /></span>
                    </div>
                  </div>
                  <div class="mock result" aria-hidden="true">
                    <div class="result-title">Edited 2 layers</div>
                    <div class="result-row"><i></i>Powermove · Scale 100 → 108 over 0:00–0:06</div>
                    <div class="result-row"><i></i>Kicker · Opacity 100 → 0 at 0:02</div>
                    <div class="result-foot">Undo available</div>
                  </div>
                {:else if f.id === 'timeline'}
                  <img src="/editor-2.png" alt="" width="1600" height="1000" loading="lazy" class="shot shot-timeline" />
                {:else if f.id === 'mods'}
                  <div class="mock manifest" aria-hidden="true">
                    <div class="file-tab">manifest.json</div>
<pre><code>{`{
  "id": "quiet-timeline",
  "name": "Quiet timeline",
  "apiVersion": 1,
  "replaces": "timeline",
  "main": "index.ts"
}`}</code></pre>
                  </div>
                  <div class="mock toggle" aria-hidden="true">
                    <span>Quiet timeline</span><span class="switch on"></span>
                  </div>
                {:else}
                  <div class="mock manifest handoff" aria-hidden="true">
                    <div class="file-tab">main.js</div>
<pre><code>{`import { createPlayer } from './player.js';

const player = await createPlayer({
  canvas: document.querySelector('canvas'),
  scene: './scene.json',
  loop: true,
});
player.setText('title', 'Make your move.');`}</code></pre>
                  </div>
                  <div class="mock toggle" aria-hidden="true">
                    <span>scene.json · player.js</span><span class="chip">ESM</span>
                  </div>
                {/if}
              </div>
            </article>
          {/each}
        </div>
      </div>
    </div>
  </section>

  <section class="principles" aria-labelledby="principles-title">
    <div class="wrap">
      <h2 id="principles-title"><span class="strong">Yours to change.</span><br />Every layer of it.</h2>
      <div class="wide">
        <div class="wide-inner">
          <img src="/editor-1.png" alt="Powermove inspector and timeline with keyframes selected." width="1600" height="1000" loading="lazy" />
        </div>
      </div>
      <div class="grid4">
        <p><span class="strong"><Undo2 size={15} class="inline" aria-hidden="true" /> Undoable.</span> Agent edits and yours share one history. Nothing happens off the record.</p>
        <p><span class="strong"><Layers2 size={15} class="inline" aria-hidden="true" /> Real layers.</span> Generated work is an ordinary composition, not a rendered clip.</p>
        <p><span class="strong"><Puzzle size={15} class="inline" aria-hidden="true" /> Personal software.</span> Built-in panels are mods. Fork one, or ask for a new one.</p>
        <p><span class="strong"><FileCode2 size={15} class="inline" aria-hidden="true" /> Yours on disk.</span> Projects save as files with embedded media and a kept backup.</p>
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
          <div><div class="stat">Local</div><p>Native agent processes, projects as files</p></div>
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
        <ul><li><a href="mailto:hello@motioner.app">hello@motioner.app</a></li><li><a href="mailto:hello@motioner.app?subject=Powermove%20waitlist">Waitlist</a></li></ul>
      </div>
      <div class="foot-copy">© Powermove {year}. In development.</div>
    </div>
  </footer>
</main>
