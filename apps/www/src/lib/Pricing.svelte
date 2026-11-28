<script lang="ts">
  import { onMount } from 'svelte';
  import BrandMark from '$lib/BrandMark.svelte';
  import { download } from '$lib/download.svelte';

  type Feature = { title: string; detail?: string; soon?: boolean };
  type Plan = {
    id: 'free' | 'aerolite';
    name: string;
    price: string;
    period?: string;
    note: string;
    lead: string;
    features: Feature[];
  };

  const plans: Plan[] = [
    {
      id: 'free',
      name: 'Free',
      price: '$0',
      note: 'The whole editor, with your own agent.',
      lead: 'Includes',
      features: [
        { title: 'The whole editor', detail: 'Every tool, effect and export' },
        { title: 'Extensions', detail: 'Install from the Store, or have the agent build one' },
        { title: 'Your own agent', detail: 'Bring your Claude Code or Codex subscription' },
      ],
    },
    {
      id: 'aerolite',
      name: 'Powermove Aerolite',
      price: '$9.99',
      period: 'per month',
      note: 'Built-in intelligence.',
      lead: 'Everything in Free, plus',
      features: [
        { title: '1,000 AI credits monthly' },
        { title: 'Review links', soon: true },
        { title: 'Cloud projects', soon: true },
      ],
    },
  ];

  let grid = $state<HTMLElement | undefined>();
  let shown = $state(false);
  let skyDots = $state<HTMLCanvasElement | undefined>();
  let skyBloom = $state<HTMLCanvasElement | undefined>();

  $effect(() => {
    if (!grid) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          shown = true;
          observer.disconnect();
        }
      },
      { threshold: 0.2 },
    );
    observer.observe(grid);
    return () => observer.disconnect();
  });

  onMount(() => {
    if (!skyDots || !skyBloom) return;
    const sdCtx = skyDots.getContext('2d');
    const sbCtx = skyBloom.getContext('2d');
    const skyStage = skyDots.parentElement;
    if (!sdCtx || !sbCtx || !skyStage) return;

    const fract = (x: number) => x - Math.floor(x);
    const hash = (x: number, y: number) => fract(Math.sin(x * 12.9898 + y * 78.233) * 43758.5453);
    const noise = (x: number, y: number) => {
      const ix = Math.floor(x), iy = Math.floor(y);
      const fx = x - ix, fy = y - iy;
      const ux = fx * fx * (3 - 2 * fx);
      const uy = fy * fy * (3 - 2 * fy);
      const a = hash(ix, iy);
      const b = hash(ix + 1, iy);
      const c = hash(ix, iy + 1);
      const d = hash(ix + 1, iy + 1);
      return (a + (b - a) * ux) * (1 - uy) + (c + (d - c) * ux) * uy;
    };
    const fbm = (x: number, y: number) => {
      let qx = x, qy = y, v = 0, amp = 0.5;
      for (let i = 0; i < 3; i++) {
        v += amp * noise(qx, qy);
        qx = qx * 2.03 + 11.7;
        qy = qy * 2.03 + 11.7;
        amp *= 0.5;
      }
      return v;
    };

    const S_PALE = [214, 239, 255];
    const S_SKY = [56, 189, 248];
    const S_DEEP = [2, 132, 199];
    let sw = 360, sh = 88, sdpr = 2;

    function resizeSky() {
      const r = skyStage!.getBoundingClientRect();
      sw = Math.max(240, Math.round(r.width || 360));
      sh = Math.max(72, Math.round(r.height || 88));
      sdpr = Math.min(window.devicePixelRatio || 1, 2);
      skyDots!.width = Math.round(sw * sdpr);
      skyDots!.height = Math.round(sh * sdpr);
      sdCtx!.setTransform(sdpr, 0, 0, sdpr, 0, 0);
      skyBloom!.width = Math.max(48, Math.round(sw / 4));
      skyBloom!.height = Math.max(18, Math.round(sh / 4));
    }

    function drawSky(age: number) {
      sdCtx!.clearRect(0, 0, sw, sh);
      const bw = skyBloom!.width, bh = skyBloom!.height;
      sbCtx!.clearRect(0, 0, bw, bh);
      const pitch = 5.5, rad = 1.35, cols = Math.ceil(sw / pitch), rows = Math.ceil(sh / pitch), PI2 = Math.PI * 2;
      for (let row = 0; row < rows; row++) {
        const py = row * pitch + pitch * 0.5;
        const up = 1 - py / sh;
        for (let col = 0; col < cols; col++) {
          const px = col * pitch + pitch * 0.5;
          const xn = (px / sw) * 2.2;
          let swirl = 0, rise = 0;
          for (let j = 0; j < 4; j++) {
            const hem =
              0.16 +
              0.21 * j +
              0.11 * Math.sin(xn * (1.6 + 0.65 * j) + age * (0.45 + 0.2 * j) + j * 2.0) +
              0.08 * (fbm(xn * 1.5 + age * 0.12, j * 3.7) - 0.5);
            const above = up - hem;
            const curtain = above < 0
              ? Math.exp(Math.max(above * 30, -40))
              : 0.56 * Math.exp(-above * 2.1) + 0.44 * Math.exp(-above * 13);
            const rays = fbm(xn * 14 + age * 0.22 + j * 5, j * 7 + up * 0.12 - age * 0.03);
            const r = Math.min(1, Math.max(0, (rays - 0.14) / 0.48));
            const rs = r * r * (3 - 2 * r);
            const v =
              1.6 *
              curtain *
              (0.26 + 0.74 * rs) *
              (0.82 + 0.18 * Math.sin(xn * 9.5 - age * 2.0 + j * 1.3)) *
              (1 - 0.06 * j);
            if (v > swirl) {
              swirl = v;
              rise = Math.min(1, Math.max(0, above * 2.6));
            }
          }
          const s = Math.min(1, Math.max(0, swirl));
          const alpha = Math.min(1, 0.06 + 0.38 * s + 0.56 * Math.pow(s, 1.6));
          if (alpha < 0.04) continue;
          const t1 = Math.min(1, rise / 0.38);
          const t2 = Math.min(1, Math.max(0, (rise - 0.38) / 0.62)) * 0.72;
          const rr = (S_PALE[0] * (1 - t1) + S_SKY[0] * t1) * (1 - t2) + S_DEEP[0] * t2;
          const gg = (S_PALE[1] * (1 - t1) + S_SKY[1] * t1) * (1 - t2) + S_DEEP[1] * t2;
          const bb = (S_PALE[2] * (1 - t1) + S_SKY[2] * t1) * (1 - t2) + S_DEEP[2] * t2;
          sdCtx!.fillStyle = `rgba(${rr | 0},${gg | 0},${bb | 0},${alpha.toFixed(3)})`;
          sdCtx!.beginPath();
          sdCtx!.arc(px, py, rad, 0, PI2);
          sdCtx!.fill();
          if (s > 0.28) {
            sbCtx!.fillStyle = `rgba(${rr | 0},${gg | 0},${bb | 0},${Math.min(0.6, Math.pow(s, 1.35) * 0.52).toFixed(3)})`;
            sbCtx!.fillRect((px / sw) * bw - 1.5, (py / sh) * bh - 1.5, 3, 3);
          }
        }
      }
    }

    resizeSky();
    drawSky(1.8);

    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
    let raf = 0;
    let visible = false;
    const sStart = performance.now();
    let sLast = 0;

    function tickSky(now: number) {
      if (!visible || document.hidden) {
        raf = 0;
        return;
      }
      if (now - sLast >= 40) {
        sLast = now;
        drawSky(1.8 + ((now - sStart) / 1000) * 0.8);
      }
      raf = requestAnimationFrame(tickSky);
    }

    function sync() {
      if (reduced) return;
      if (visible && !document.hidden) {
        if (!raf) raf = requestAnimationFrame(tickSky);
      } else if (raf) {
        cancelAnimationFrame(raf);
        raf = 0;
      }
    }

    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      sync();
    });
    observer.observe(skyStage);
    document.addEventListener('visibilitychange', sync);
    const onResize = () => { resizeSky(); drawSky(1.8); };
    window.addEventListener('resize', onResize);

    return () => {
      if (raf) cancelAnimationFrame(raf);
      observer.disconnect();
      document.removeEventListener('visibilitychange', sync);
      window.removeEventListener('resize', onResize);
    };
  });

  function lean(event: PointerEvent) {
    if (event.pointerType !== 'mouse') return;
    const card = event.currentTarget as HTMLElement;
    const box = card.getBoundingClientRect();
    card.style.setProperty('--mx', `${((event.clientX - box.left) / box.width) * 100}%`);
  }
  function settle(event: PointerEvent) {
    (event.currentTarget as HTMLElement).style.removeProperty('--mx');
  }
</script>

<!-- Icons: IconCheck from @pierre/icons 0.9.0 (Apache-2.0). -->
{#snippet check()}
  <svg class="feature-mark" viewBox="0 0 16 16" aria-hidden="true"><path d="M13.8551 3.17382C14.1733 3.439 14.2163 3.91192 13.9512 4.23013L6.86783 12.7301C6.73293 12.892 6.53616 12.9897 6.32565 12.9992C6.11515 13.0088 5.91034 12.9293 5.76134 12.7803L2.21967 9.23865C1.92678 8.94576 1.92678 8.47088 2.21967 8.17799C2.51256 7.8851 2.98744 7.8851 3.28033 8.17799L6.24127 11.1389L12.7988 3.26985C13.064 2.95164 13.5369 2.90865 13.8551 3.17382Z" fill="currentColor" /></svg>
{/snippet}

<section class="pricing" id="pricing" aria-labelledby="pricing-title">
  <h2 id="pricing-title" class="rows-title"><span class="strong">Our paid plans.</span></h2>

  <div class="plans" bind:this={grid} data-in={shown ? '' : undefined}>
    {#each plans as plan, i (plan.id)}
      {#if plan.id === 'free'}
        <article class="plan" data-plan="free" style:--i={i} aria-labelledby="plan-{plan.id}">
          <div class="plan-head">
            <h3 id="plan-{plan.id}" class="plan-name">{plan.name}</h3>
          </div>
          <p class="plan-price"><span class="amount tabular">{plan.price}</span></p>
          <p class="plan-note">{plan.note}</p>
          <div class="plan-cta"><a class="pill dark" href={download.href}>Download for Mac</a></div>
          <div class="plan-features">
            <p class="features-lead">{plan.lead}</p>
            <ul>
              {#each plan.features as feature (feature.title)}
                <li data-soon={feature.soon ? '' : undefined}>
                  {@render check()}
                  <span class="feature-text">
                    <span class="feature-title">{feature.title}{#if feature.soon}<span class="tag">Soon</span>{/if}</span>
                    {#if feature.detail}<span class="feature-detail">{feature.detail}</span>{/if}
                  </span>
                </li>
              {/each}
            </ul>
          </div>
        </article>
      {:else}
        <article
          class="plan"
          data-plan="aerolite"
          style:--i={i}
          id="aerolite-card"
          aria-labelledby="plan-{plan.id}"
          onpointermove={lean}
          onpointerleave={settle}
        >
          <div class="aerolite-aurora-head" aria-hidden="true">
            <canvas class="aerolite-aurora-bloom" bind:this={skyBloom}></canvas>
            <canvas bind:this={skyDots}></canvas>
          </div>
          <div class="aerolite-card-body">
            <div class="plan-head">
              <h3 id="plan-{plan.id}" class="plan-name" style="margin:0;">
                <BrandMark variant="aerolite" idPrefix="aero" height={36} margin="-6px 0" />
              </h3>
            </div>
            <p class="plan-price">
              <span class="amount tabular">{plan.price}</span>
              {#if plan.period}<span class="period">{plan.period}</span>{/if}
            </p>
            <p class="plan-note">{plan.note}</p>
            <div class="plan-cta"><a class="pill sky" href={download.href}>Get Aerolite</a></div>
            <div class="plan-features">
              <p class="features-lead">{plan.lead}</p>
              <ul>
                {#each plan.features as feature (feature.title)}
                  <li data-soon={feature.soon ? '' : undefined}>
                    {@render check()}
                    <span class="feature-text">
                      <span class="feature-title">{feature.title}{#if feature.soon}<span class="tag">Soon</span>{/if}</span>
                      {#if feature.detail}<span class="feature-detail">{feature.detail}</span>{/if}
                    </span>
                  </li>
                {/each}
              </ul>
            </div>
          </div>
        </article>
      {/if}
    {/each}
  </div>

  <p class="pricing-fine">Prices in US dollars. Credits pay for Aerolite’s inference.</p>
</section>
