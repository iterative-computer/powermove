'use client';

import { useEffect, useRef, useState } from 'react';

export function HeroAnimation() {
  const svg = useRef<SVGSVGElement>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let cancelled = false, visible = true, frame = 0, last = 0, elapsed = 0;
    let player: {duration: number; seek(time: number): void; destroy(): void} | undefined;
    const abort = new AbortController();
    const motion = matchMedia('(prefers-reduced-motion: reduce)');
    const root = svg.current!;
    function tick(now: number) {
      if (!player) return;
      if (last) elapsed = (elapsed + (now - last) / 1000) % player.duration;
      last = now; player.seek(elapsed); frame = requestAnimationFrame(tick);
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
    document.addEventListener('visibilitychange', sync);
    void (async () => {
      try {
        const [{ createSvgPlayer }, response] = await Promise.all([
          import('@/lib/powermove/svg-player'), fetch('/hero/scene.json', {signal: abort.signal}),
        ]);
        if (!response.ok) throw new Error('Scene unavailable');
        const scene = await response.json();
        await document.fonts.load('400 128px Geist');
        await document.fonts.ready;
        if (cancelled) return;
        player = createSvgPlayer(root, scene);
        player.seek(0); root.dataset.ready = 'true'; sync();
      } catch { if (!cancelled) setFailed(true); }
    })();
    return () => {
      cancelled = true; abort.abort(); cancelAnimationFrame(frame); observer.disconnect();
      motion.removeEventListener('change', sync); document.removeEventListener('visibilitychange', sync);
      player?.destroy();
    };
  }, []);
  return <div className="hero-art" role="group" aria-label="Made in Powermove: animated typography showcasing a gradient editor, 3D system, and shader">
    <svg ref={svg} className="hero-vector" viewBox="0 0 1920 1080" aria-hidden="true"/>
    {failed && <div className="animation-fallback">Make a Powermove.</div>}
  </div>;
}
