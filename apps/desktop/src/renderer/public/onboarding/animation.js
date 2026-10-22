import { createOnboardingSvgPlayer } from './svg-player.js';

let skipped = false;
let player;
let endingFrame = 0;
let timeout;
const media = window.matchMedia('(prefers-reduced-motion: reduce)');
function skip() {
  if (skipped) return;
  skipped = true;
  window.clearTimeout(timeout);
  cancelAnimationFrame(endingFrame);
  player?.destroy();
  window.onboarding.animationComplete();
}
document.querySelector('#skip-intro')?.addEventListener('click', skip);
window.addEventListener('keydown', event => { if (event.key === 'Escape') { event.preventDefault(); skip(); } });
media.addEventListener('change', () => { if (media.matches) skip(); });

async function start() {
  if (media.matches) { skip(); return; }
  document.querySelector('#skip-intro')?.focus();
  const svg = document.querySelector('#onboarding-svg');
  if (!(svg instanceof SVGSVGElement)) throw new Error('Onboarding SVG is unavailable');
  const response = await fetch('./scene.json');
  if (!response.ok) throw new Error(`Could not load onboarding scene (${response.status})`);
  player = await createOnboardingSvgPlayer({
    svg,
    scene: await response.json(),
    baseURL: new URL('./', window.location.href),
    audio: true,
    autoplay: false,
    onComplete: () => window.onboarding.animationComplete(),
    onError: (error) => window.onboarding.animationFailed(error?.message ?? String(error))
  });
  if (skipped) { player.destroy(); return; }
  let endingSent = false;
  document.body.classList.add('onboarding-playing');
  player.play();
  const observeEnding = () => {
    if (!endingSent && player.currentTime >= 7) {
      endingSent = true;
      document.body.classList.add('onboarding-ending');
      window.onboarding.animationEnding();
    }
    endingFrame = requestAnimationFrame(observeEnding);
  };
  endingFrame = requestAnimationFrame(observeEnding);
  timeout = window.setTimeout(() => {
    window.onboarding.animationFailed('Onboarding playback timed out');
  }, (player.duration + 15) * 1000);
  window.addEventListener('pagehide', () => {
    window.clearTimeout(timeout);
    cancelAnimationFrame(endingFrame);
    player.destroy();
  }, { once: true });
}

start().catch((error) => {
  window.onboarding.animationFailed(error?.message ?? String(error));
});
