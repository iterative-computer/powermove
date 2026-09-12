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
  let marker = $state<HTMLElement | undefined>();
  let markerY: number | undefined;
  let flight: Animation | undefined;
  let lockUntil = 0;

  // One animation drives both axes, so the dot traces a real arc: it swings
  // outward (left, away from the labels) and lands without a second clock.
  $effect(() => {
    const el = railItems[active];
    if (!el || !marker) return;
    const y = el.offsetTop + el.offsetHeight / 2;
    if (markerY === undefined) { markerY = y; marker.style.transform = `translateY(${y}px)`; return; }
    if (y === markerY) return;
    let fromX = 0, fromY = markerY;
    markerY = y;
    const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
    // Interrupted mid-flight: pick up from where the dot actually is and keep
    // its momentum (ease-out) instead of stopping and winding up again.
    const midFlight = flight?.playState === 'running';
    if (midFlight) {
      const m = new DOMMatrix(getComputedStyle(marker).transform);
      fromX = m.m41; fromY = m.m42;
    }
    flight?.cancel();
    if (reduce) { marker.style.transform = `translateY(${y}px)`; return; }
    const steps = 24;
    const frames = Array.from({ length: steps + 1 }, (_, i) => {
      const p = i / steps;
      const x = fromX * (1 - p) - 10 * Math.sin(Math.PI * p);
      return { transform: `translate(${x.toFixed(2)}px, ${(fromY + (y - fromY) * p).toFixed(2)}px)` };
    });
    marker.style.transform = `translateY(${y}px)`;
    flight = marker.animate(frames, {
      duration: midFlight ? 420 : 520,
      easing: midFlight ? 'cubic-bezier(.2,.6,.2,1)' : 'cubic-bezier(.4,0,.2,1)',
    });
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
          <span class="rail-marker" bind:this={marker} aria-hidden="true"></span>
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
                <div class="ed ed-agent" aria-hidden="true">
                  <div class="ed-pane ed-thread">
                    <div class="ed-head"><span>Powermove agent</span><span class="ed-dim">Claude Code</span></div>
                    <div class="ed-msg you">Give the title a slow push-in and fade the kicker out at 2s.</div>
                    <div class="ed-msg">
                      <div class="ed-msg-title">Edited 2 layers</div>
                      <div class="ed-edit"><i></i>Powermove <span class="ed-dim">Scale 100 → 108 · 0:00–0:06</span></div>
                      <div class="ed-edit"><i></i>Kicker <span class="ed-dim">Opacity 100 → 0 · 0:02</span></div>
                      <div class="ed-msg-foot">Rendered 3 frames to check · Undo</div>
                    </div>
                    <div class="ed-composer">
                      <div class="ed-composer-text">Describe what you want</div>
                      <div class="ed-composer-row"><span>All panels</span><span>Claude Code</span><span class="ed-send"><ArrowUpRight size={11} /></span></div>
                    </div>
                  </div>
                  <div class="ed-pane ed-view">
                    <div class="ed-frame"><span class="ed-kicker">DESIGN / MOTION / SYSTEM</span><span class="ed-title">Make the move.</span></div>
                  </div>
                </div>
              {:else if f.id === 'timeline'}
                <div class="ed ed-timeline" aria-hidden="true">
                  <div class="ed-pane ed-view">
                    <div class="ed-frame"><span class="ed-kicker">DESIGN / MOTION / SYSTEM</span><span class="ed-title">Make the move.</span></div>
                  </div>
                  <div class="ed-pane ed-tl">
                    <div class="ed-tl-bar"><span class="ed-tl-play"></span><span class="ed-tab on">Curves</span><span class="ed-tab">Fit keys</span><span class="ed-tab">Shy</span></div>
                    <div class="ed-tl-body">
                      <div class="ed-tl-layers">
                        <div class="ed-tl-ruler"></div>
                        <div class="ed-tl-layer"><b class="ed-badge blue">T</b>Kicker</div>
                        <div class="ed-tl-layer on"><b class="ed-badge accent">T</b>Powermove</div>
                        <div class="ed-tl-layer"><b class="ed-badge sand">S</b>Signal</div>
                        <div class="ed-tl-layer"><b class="ed-badge violet">F</b>Atmosphere</div>
                        <div class="ed-tl-layer"><b class="ed-badge sand">E</b>Grain</div>
                        <div class="ed-tl-layer"><b class="ed-badge blue">A</b>Music</div>
                      </div>
                      <div class="ed-tl-tracks">
                        <div class="ed-tl-ruler"><span>0:00</span><span>0:01</span><span>0:02</span><span>0:03</span><span>0:04</span><span>0:05</span></div>
                        <div class="ed-tl-row"><span class="ed-bar blue" style="left:4%;width:56%"><i style="left:0"></i><i style="left:66%"></i></span></div>
                        <div class="ed-tl-row"><span class="ed-bar accent on" style="left:4%;width:88%"><i style="left:0"></i><i style="left:100%"></i></span></div>
                        <div class="ed-tl-row"><span class="ed-bar sand" style="left:0;width:100%"></span></div>
                        <div class="ed-tl-row"><span class="ed-bar violet" style="left:0;width:100%"></span></div>
                        <div class="ed-tl-row"><span class="ed-bar sand" style="left:0;width:100%"></span></div>
                        <div class="ed-tl-row"><span class="ed-bar blue" style="left:0;width:74%"><i style="left:0"></i><i style="left:100%"></i></span></div>
                        <div class="ed-playhead" style="left:36%"></div>
                      </div>
                    </div>
                  </div>
                </div>
              {:else if f.id === 'mods'}
                <div class="ed ed-mods" aria-hidden="true">
                  <div class="ed-pane ed-ext">
                    <div class="ed-head"><span>Extensions</span><span class="ed-dim">8 installed</span></div>
                    <div class="ed-ext-row on"><span>Quiet timeline</span><span class="ed-dim">replaces timeline</span><span class="ed-switch on"></span></div>
                    <div class="ed-ext-row"><span>Dusk theme</span><span class="ed-dim">theme</span><span class="ed-switch on"></span></div>
                    <div class="ed-ext-row"><span>Vim keymap</span><span class="ed-dim">keymap</span><span class="ed-switch on"></span></div>
                    <div class="ed-ext-row"><span>Film grain</span><span class="ed-dim">effect</span><span class="ed-switch"></span></div>
                    <div class="ed-ext-row"><span>Whip pan</span><span class="ed-dim">transition</span><span class="ed-switch on"></span></div>
                    <div class="ed-ext-row"><span>Frame checker</span><span class="ed-dim">agent tool</span><span class="ed-switch on"></span></div>
                    <div class="ed-ext-row"><span>Lottie import</span><span class="ed-dim">importer</span><span class="ed-switch"></span></div>
                    <div class="ed-ext-row"><span>Inspector, minimal</span><span class="ed-dim">replaces inspector</span><span class="ed-switch"></span></div>
                  </div>
                  <div class="ed-pane ed-code">
                    <div class="ed-head"><span>manifest.json</span><span class="ed-dim">hot reload</span></div>
<pre><code>{'{'}
  <span class="tok-key">"id"</span>: <span class="tok-str">"quiet-timeline"</span>,
  <span class="tok-key">"name"</span>: <span class="tok-str">"Quiet timeline"</span>,
  <span class="tok-key">"apiVersion"</span>: <span class="tok-num">1</span>,
  <span class="tok-key">"replaces"</span>: <span class="tok-str">"timeline"</span>,
  <span class="tok-key">"main"</span>: <span class="tok-str">"index.ts"</span>
{'}'}</code></pre>
                  </div>
                </div>
              {:else}
                <div class="ed ed-export" aria-hidden="true">
                  <div class="ed-pane ed-out">
                    <div class="ed-head"><span>Export</span><span class="ed-dim">Powermove.pm</span></div>
                    <div class="ed-seg"><span>Video</span><span class="on">Web player</span><span>SVG</span></div>
                    <div class="ed-field"><span>Scene</span><b>scene.json</b></div>
                    <div class="ed-field"><span>Player</span><b>player.js · ESM</b></div>
                    <div class="ed-field"><span>Size</span><b>1920 × 1080</b></div>
                    <div class="ed-field"><span>Loop</span><span class="ed-switch on"></span></div>
                    <div class="ed-field"><span>Fonts</span><b>Subset, inline</b></div>
                    <div class="ed-action">Export scene</div>
                  </div>
                  <div class="ed-pane ed-code">
                    <div class="ed-head"><span>main.js</span><span class="ed-dim">your site</span></div>
<pre><code><span class="tok-kw">import</span> {'{'} <span class="tok-fn">createPlayer</span> {'}'} <span class="tok-kw">from</span> <span class="tok-str">'./player.js'</span>;

<span class="tok-kw">const</span> player = <span class="tok-kw">await</span> <span class="tok-fn">createPlayer</span>({'{'}
  <span class="tok-key">canvas</span>: document.<span class="tok-fn">querySelector</span>(<span class="tok-str">'canvas'</span>),
  <span class="tok-key">scene</span>: <span class="tok-str">'./scene.json'</span>,
  <span class="tok-key">loop</span>: <span class="tok-kw">true</span>,
{'}'});
player.<span class="tok-fn">setText</span>(<span class="tok-str">'title'</span>, <span class="tok-str">'Make your move.'</span>);</code></pre>
                  </div>
                </div>
              {/if}
            </div>
          </article>
        {/each}
      </div>
    </div>
  </div>
</section>
