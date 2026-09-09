'use client';

import { useEffect, useRef, useState } from 'react';

export function HeroAnimation() {
  const svg = useRef<SVGSVGElement>(null);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let cancelled = false, visible = true, frame = 0, last = 0, elapsed = 0;
    let player: {duration: number; seek(time: number): void; destroy(): void} | undefined;
    const abort = new AbortController();
    const motion = matchMedia('(prefers-reduced-motion: reduce)');
    const root = svg.current!;
    setReady(false);
    function fail(error: unknown) {
      if (cancelled) return;
      cancelAnimationFrame(frame);
      player?.destroy(); player = undefined;
      delete root.dataset.ready;
      setReady(false);
      console.error('[Powermove hero]', error);
    }
    function tick(now: number) {
      if (!player) return;
      if (last) elapsed = (elapsed + (now - last) / 1000) % player.duration;
      last = now;
      try { player.seek(elapsed); frame = requestAnimationFrame(tick); }
      catch (error) { fail(error); }
    }
    function sync() {
      cancelAnimationFrame(frame); last = 0;
      if (!player || cancelled) return;
      if (motion.matches) player.seek(Math.max(0, player.duration - 1));
      else if (visible && !document.hidden) frame = requestAnimationFrame(tick);
    }
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; sync(); });
    observer.observe(root);
    motion.addEventListener('change', sync);
    document.fonts.addEventListener('loadingdone', sync);
    document.addEventListener('visibilitychange', sync);
    void (async () => {
      try {
        const [{ createSvgPlayer }, response] = await Promise.all([
          import('@/lib/powermove/svg-player'), fetch('/hero/scene.json', {signal: abort.signal}),
        ]);
        if (!response.ok) throw new Error('Scene unavailable');
        const scene = await response.json();
        // A slow font must not prevent playback. Each seek remeasures glyphs,
        // so a font that arrives later is picked up on the next frame.
        let fontTimeout: ReturnType<typeof setTimeout> | undefined;
        try {
          await Promise.race([
            document.fonts.load('400 128px Geist').catch(() => []),
            new Promise(resolve => { fontTimeout = setTimeout(resolve, 1500); }),
          ]);
        } finally { clearTimeout(fontTimeout); }
        if (cancelled) return;
        player = createSvgPlayer(root, scene);
        player.seek(0); root.dataset.ready = 'true'; sync(); setReady(true);
      } catch (error) { fail(error); }
    })();
    return () => {
      cancelled = true; abort.abort(); cancelAnimationFrame(frame); observer.disconnect();
      motion.removeEventListener('change', sync); document.removeEventListener('visibilitychange', sync);
      document.fonts.removeEventListener('loadingdone', sync);
      player?.destroy();
    };
  }, []);
  return <div className="hero-art" role="group" aria-label="Made in Powermove: animated typography showcasing a gradient editor, 3D system, and shader">
    {/* Still frame from scene.json at 14s, present even before hydration. */}
    {!ready && <svg className="hero-vector hero-poster" viewBox="0 0 1920 1080" aria-hidden="true">
      <g fill="#1B1D23" fontFamily="Geist, sans-serif" fontSize="128" letterSpacing="-2" style={{fontVariationSettings: "'wght' 398.334"}}>
        <text x="960" y="575.13599" textAnchor="middle">Shape your video editor</text>
      </g>
    </svg>}
    <svg ref={svg} className="hero-vector" viewBox="0 0 1920 1080" aria-hidden="true" style={{visibility: ready ? 'visible' : 'hidden'}}/>
  </div>;
}
