<script lang="ts">
  import { onMount } from 'svelte';
  import { download } from '$lib/download.svelte';

  // Three claims set back in the gradient, then the one that matters, bright.
  const lines = [
    'Keyframe real layers.',
    'Direct an agent on the timeline.',
    'Ask for the features you’re missing.',
    'Shape your video editor.',
  ];
  const headline = lines.join(' ');

  // Prompts typed in the margins, the kind people actually send the agent.
  const margins = [
    { side: 'left', top: 11, text: 'Make the title spring in.\n0.4s, no bounce.' },
    { side: 'right', top: 15, text: 'Keyframe scale 100 → 112\nat 00:02:10.' },
    { side: 'left', top: 44, text: 'Add a whip pan between\nshots 2 and 3.' },
    { side: 'right', top: 50, text: 'Build me a beat-sync panel.' },
    { side: 'left', top: 74, text: 'Grain at 0.35,\nheadline only.' },
    { side: 'right', top: 78, text: 'Fork the inspector.\nHide what I don’t use.' },
  ] as const;

  let typed = $state(Infinity);
  let caret = $state(false);
  let marginTyped = $state<number[]>(margins.map(m => m.text.length));
  let pending = $state(true);

  // Characters before each line starts, so one counter drives all four.
  const starts = lines.reduce<number[]>((acc, line, i) => [...acc, i ? acc[i - 1] + lines[i - 1].length : 0], []);
  const total = starts[lines.length - 1] + lines[lines.length - 1].length;

  onMount(() => {
    pending = false;
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    let alive = true;
    const timers: ReturnType<typeof setTimeout>[] = [];
    const later = (fn: () => void, ms: number) => timers.push(setTimeout(() => alive && fn(), ms));

    // Headline: a steady hand with a little human jitter, a breath at each line end.
    typed = 0; caret = true;
    const step = () => {
      typed += 1;
      if (typed >= total) { later(() => (caret = false), 1400); return; }
      const atLineEnd = starts.includes(typed);
      later(step, atLineEnd ? 260 : 22 + Math.random() * 26);
    };
    later(step, 380);

    // Margins: each prompt types, holds, clears, and comes back on its own clock.
    marginTyped = margins.map(() => 0);
    margins.forEach((m, i) => {
      const cycle = () => {
        let n = 0;
        const tick = () => {
          marginTyped[i] = ++n;
          if (n < m.text.length) later(tick, 34 + Math.random() * 40);
          else later(() => { marginTyped[i] = 0; later(cycle, 1600 + Math.random() * 2400); }, 3200 + Math.random() * 2000);
        };
        tick();
      };
      later(cycle, 900 + i * 700 + Math.random() * 600);
    });
    return () => { alive = false; timers.forEach(clearTimeout); };
  });
</script>

<div class="margins" aria-hidden="true">
  {#each pending ? [] : margins as m, i (m.text)}
    <p class="margin" data-side={m.side} style:top="{m.top}%">{m.text.slice(0, marginTyped[i])}{#if marginTyped[i] > 0 && marginTyped[i] < m.text.length}<span class="margin-caret">▌</span>{/if}</p>
  {/each}
</div>

<div class="hero-copy">
  <h1 class="headline" aria-label={headline} data-pending={pending ? '' : undefined}>
    {#each lines as line, i (line)}
      {@const shown = Math.max(0, Math.min(line.length, typed - starts[i]))}
      {@const here = caret && (typed < total ? typed >= starts[i] && typed < starts[i] + line.length : i === lines.length - 1)}
      <span class="headline-line" class:punch={i === lines.length - 1} aria-hidden="true"><span>{line.slice(0, shown)}</span>{#if here}<span class="caret"></span>{/if}<span class="unset">{line.slice(shown)}</span></span>
    {/each}
  </h1>
  <div class="hero-cta">
    <a class="pill light" href={download.href}>Download for Mac</a>
    <a class="pill glass" href="#film">
      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path fill="currentColor" d="M4.5 2.8v10.4a.7.7 0 0 0 1.06.6l8.2-5.2a.7.7 0 0 0 0-1.2l-8.2-5.2a.7.7 0 0 0-1.06.6Z" /></svg>
      Watch the film
    </a>
  </div>
</div>
