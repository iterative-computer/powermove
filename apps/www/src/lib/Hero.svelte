<script lang="ts">
  import { onMount } from 'svelte';
  import { download, stars } from '$lib/download.svelte';
  import { GITHUB } from '$lib/links';

  let dotsCanvas = $state<HTMLCanvasElement | undefined>();
  let bloomCanvas = $state<HTMLCanvasElement | undefined>();

  onMount(() => {
    if (!dotsCanvas || !bloomCanvas) return;
    const dCtx = dotsCanvas.getContext('2d');
    const bCtx = bloomCanvas.getContext('2d');
    const stage = dotsCanvas.parentElement;
    if (!dCtx || !bCtx || !stage) return;

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

    const GOLD = [255, 210, 122];
    const ACCENT = [250, 99, 16];
    const ROSE = [237, 93, 133];

    let W = 728, H = 200, dpr = 2;
    const PITCH = 6.5;
    const RADIUS = 1.65;

    function resize() {
      const rect = stage!.getBoundingClientRect();
      W = Math.max(320, Math.round(rect.width || 728));
      H = Math.max(160, Math.round(rect.height || 200));
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      dotsCanvas!.width = Math.round(W * dpr);
      dotsCanvas!.height = Math.round(H * dpr);
      dCtx!.setTransform(dpr, 0, 0, dpr, 0, 0);
      bloomCanvas!.width = Math.max(64, Math.round(W / 4));
      bloomCanvas!.height = Math.max(40, Math.round(H / 4));
    }

    function drawFrame(age: number) {
      dCtx!.clearRect(0, 0, W, H);
      const bw = bloomCanvas!.width, bh = bloomCanvas!.height;
      bCtx!.clearRect(0, 0, bw, bh);

      const cols = Math.ceil(W / PITCH);
      const rows = Math.ceil(H / PITCH);
      const PI2 = Math.PI * 2;

      for (let row = 0; row < rows; row++) {
        const py = row * PITCH + PITCH * 0.5;
        const up = 1 - py / H;
        for (let col = 0; col < cols; col++) {
          const px = col * PITCH + PITCH * 0.5;
          const xn = (px / W) * 2.35;

          let swirl = 0, rise = 0;
          for (let j = 0; j < 4; j++) {
            const hem =
              0.14 +
              0.20 * j +
              0.11 * Math.sin(xn * (1.55 + 0.65 * j) + age * (0.45 + 0.2 * j) + j * 1.95) +
              0.08 * (fbm(xn * 1.6 + age * 0.12, j * 3.7) - 0.5);
            const above = up - hem;
            const curtain = above < 0
              ? Math.exp(Math.max(above * 32, -40))
              : 0.58 * Math.exp(-above * 2.1) + 0.42 * Math.exp(-above * 14);
            const rays = fbm(xn * 14 + age * 0.22 + j * 5, j * 7 + up * 0.12 - age * 0.03);
            const r = Math.min(1, Math.max(0, (rays - 0.14) / 0.48));
            const rs = r * r * (3 - 2 * r);
            const v =
              1.65 *
              curtain *
              (0.24 + 0.76 * rs) *
              (0.82 + 0.18 * Math.sin(xn * 9.5 - age * 2.1 + j * 1.3)) *
              (1 - 0.06 * j);
            if (v > swirl) {
              swirl = v;
              rise = Math.min(1, Math.max(0, above * 2.6));
            }
          }

          const s = Math.min(1, Math.max(0, swirl));
          const alpha = Math.min(1, 0.06 + 0.40 * s + 0.56 * Math.pow(s, 1.65));
          if (alpha < 0.035) continue;

          const t1 = Math.min(1, rise / 0.38);
          const t2 = Math.min(1, Math.max(0, (rise - 0.38) / 0.62)) * 0.72;
          let rr = (GOLD[0] * (1 - t1) + ACCENT[0] * t1) * (1 - t2) + ROSE[0] * t2;
          let gg = (GOLD[1] * (1 - t1) + ACCENT[1] * t1) * (1 - t2) + ROSE[1] * t2;
          let bb = (GOLD[2] * (1 - t1) + ACCENT[2] * t1) * (1 - t2) + ROSE[2] * t2;
          if (s > 0.72) {
            const hot = ((s - 0.72) / 0.28) * (1 - t1 * 0.7);
            rr = Math.min(255, rr + hot * 56);
            gg = Math.min(255, gg + hot * 42);
            bb = Math.min(255, bb + hot * 26);
          }

          dCtx!.fillStyle = `rgba(${rr | 0},${gg | 0},${bb | 0},${alpha.toFixed(3)})`;
          dCtx!.beginPath();
          dCtx!.arc(px, py, RADIUS, 0, PI2);
          dCtx!.fill();

          if (s > 0.45) {
            const ba = Math.min(0.26, Math.pow(s, 1.7) * 0.24);
            bCtx!.fillStyle = `rgba(${rr | 0},${gg | 0},${bb | 0},${ba.toFixed(3)})`;
            bCtx!.fillRect((px / W) * bw - 1.5, (py / H) * bh - 1.5, 3, 3);
          }
        }
      }
    }

    resize();
    drawFrame(2.1);

    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
    let raf = 0;
    let visible = true;
    const start = performance.now();
    let last = 0;

    function tick(now: number) {
      if (!visible || document.hidden) {
        raf = 0;
        return;
      }
      if (now - last >= 33) {
        last = now;
        drawFrame(2.1 + ((now - start) / 1000) * 0.85);
      }
      raf = requestAnimationFrame(tick);
    }

    function sync() {
      if (reduced) return;
      if (visible && !document.hidden) {
        if (!raf) raf = requestAnimationFrame(tick);
      } else if (raf) {
        cancelAnimationFrame(raf);
        raf = 0;
      }
    }

    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      sync();
    });
    observer.observe(stage);
    document.addEventListener('visibilitychange', sync);
    const onResize = () => { resize(); drawFrame(2.1); };
    window.addEventListener('resize', onResize);
    sync();

    return () => {
      if (raf) cancelAnimationFrame(raf);
      observer.disconnect();
      document.removeEventListener('visibilitychange', sync);
      window.removeEventListener('resize', onResize);
    };
  });
</script>

<div class="aurora-stage" aria-hidden="true">
  <canvas class="aurora-bloom" bind:this={bloomCanvas}></canvas>
  <canvas class="aurora-dots" bind:this={dotsCanvas}></canvas>
</div>
<div class="hero-scrim" aria-hidden="true"></div>

<section class="hero" aria-labelledby="headline">
  <h1 id="headline" class="hero-headline">
    The video editor that lets you do <em>anything</em>.
  </h1>

  <p class="hero-description">
    <span class="strong">Make it yours while you edit.</span> Ask an agent to build any effect, tool, or panel right inside the app.
  </p>

  <div class="hero-cta">
    <a class="pill light" href={download.href}>Download for Mac</a>
    <a class="pill glass" href={GITHUB} target="_blank" rel="noopener">
      Star on GitHub{#if stars.label} <span class="pill-count tabular">{stars.label}</span>{/if}
    </a>
  </div>
</section>
