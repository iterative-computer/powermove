import { createOnboardingSvgPlayer } from './svg-player.js';

async function start() {
  const svg = document.querySelector('#onboarding-svg');
  if (!(svg instanceof SVGSVGElement)) throw new Error('Onboarding SVG is unavailable');
  const response = await fetch('./scene.json');
  if (!response.ok) throw new Error(`Could not load onboarding scene (${response.status})`);
  const player = await createOnboardingSvgPlayer({
    svg,
    scene: await response.json(),
    baseURL: new URL('./', window.location.href),
    audio: true,
    autoplay: true,
    onComplete: () => window.onboarding.animationComplete(),
    onError: (error) => window.onboarding.animationFailed(error?.message ?? String(error))
  });
  const timeout = window.setTimeout(() => {
    window.onboarding.animationFailed('Onboarding playback timed out');
  }, (player.duration + 15) * 1000);
  window.addEventListener('pagehide', () => {
    window.clearTimeout(timeout);
    player.destroy();
  }, { once: true });
}

start().catch((error) => {
  window.onboarding.animationFailed(error?.message ?? String(error));
});
