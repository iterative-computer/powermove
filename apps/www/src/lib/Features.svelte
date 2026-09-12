<script lang="ts">
  import { ArrowUpRight, Bot, FileCode2, History, Layers2, Puzzle, Sparkles, Undo2, Wand2, Zap } from '@lucide/svelte';

  type Point = { icon: typeof Bot; label: string };
  type Feature = { id: string; rail: string; label: string; lead: string; text: string; points: Point[] };

  const features: Feature[] = [
    {
      id: 'agent', rail: 'Agent', label: 'Your agent, in the editor',
      lead: 'Describe the change.',
      text: 'Codex or Claude Code work inside the editor, render frames to check themselves, and edit through the same typed commands you do.',
      points: [
        { icon: Bot, label: 'Sees the live project and layout' },
        { icon: Wand2, label: 'Renders frames before committing' },
        { icon: Undo2, label: 'Every edit is undoable' },
      ],
    },
    {
      id: 'timeline', rail: 'Timeline', label: 'Real layers and keyframes',
      lead: 'Nothing is baked.',
      text: 'Generated work is a normal composition. Open any layer or keyframe and keep going by hand.',
      points: [
        { icon: Layers2, label: 'Groups, precomps, 2.5D' },
        { icon: History, label: 'Take history per pass' },
        { icon: Zap, label: 'GPU compositor' },
      ],
    },
    {
      id: 'mods', rail: 'Mods', label: 'An editor you can rewrite',
      lead: 'Everything above the kernel is a mod.',
      text: 'Panels, theme, keymap, and effects ship as extensions on the same public API yours use. Replace any of them.',
      points: [
        { icon: Puzzle, label: 'Manifest, TypeScript, Svelte' },
        { icon: FileCode2, label: 'Hot-loaded on save' },
        { icon: Sparkles, label: 'Or ask the agent to build one' },
      ],
    },
    {
      id: 'export', rail: 'Export', label: 'Leave as pixels or as code',
      lead: 'Pixels or code.',
      text: 'Render video, or ship the scene itself as a web player or SVG. The headline above is one, running live.',
      points: [
        { icon: Zap, label: 'Video with bundled ffmpeg' },
        { icon: FileCode2, label: 'Web player, React and Svelte examples' },
        { icon: Layers2, label: 'SVG for text and groups' },
      ],
    },
  ];

  let active = $state<string>(features[0].id);
  let blocks = $state<Record<string, HTMLElement | undefined>>({});
  let railItems = $state<Record<string, HTMLButtonElement | undefined>>({});
  let markerY = $state(0);
  let hops = $state(0);
  let ready = $state(false);
  let lockUntil = 0;

  $effect(() => {
    const el = railItems[active];
    if (!el) return;
    const y = el.offsetTop + el.offsetHeight / 2;
    if (!ready) { markerY = y; ready = true; return; }
    if (y !== markerY) { markerY = y; hops++; }
  });

  $effect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.filter((e) => e.isIntersecting).sort((a, b) => b.intersectionRatio - a.intersectionRatio);
        if (Date.now() < lockUntil) return;
        const next = visible[0]?.target.id;
        if (next) active = next;
      },
      { rootMargin: '-25% 0px -55% 0px', threshold: [0.15, 0.35, 0.55, 0.75] },
    );
    for (const f of features) { const el = blocks[f.id]; if (el) observer.observe(el); }
    return () => observer.disconnect();
  });

  function jump(id: string) {
    lockUntil = Date.now() + 900;
    active = id;
    blocks[id]?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
</script>

<section class="features" aria-labelledby="features-title">
  <div class="wrap">
    <h2 id="features-title"><span class="strong">Built for the whole move.</span><br />From the first prompt to the final frame.</h2>
    <div class="feature-layout">
      <div class="rail">
        <div class="rail-title">Product</div>
        <div class="rail-items">
          <span class="rail-marker" data-ready={ready ? '' : undefined} style:transform={`translateY(${markerY}px)`} aria-hidden="true">
            {#key hops}<i class="rail-drop"></i>{/key}
          </span>
          {#each features as f (f.id)}
            <button type="button" class="rail-item" bind:this={railItems[f.id]} data-active={active === f.id ? '' : undefined} onclick={() => jump(f.id)}>{f.rail}</button>
          {/each}
        </div>
      </div>
      <div class="blocks">
        {#each features as f (f.id)}
          <article class="block" id={f.id} bind:this={blocks[f.id]}>
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
