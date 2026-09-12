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
    autoplay: false,
    onComplete: () => window.onboarding.animationComplete(),
    onError: (error) => window.onboarding.animationFailed(error?.message ?? String(error))
  });
  let endingSent = false;
  document.body.classList.add('onboarding-playing');
  player.play();
  let endingFrame = 0;
  const observeEnding = () => {
    if (!endingSent && player.currentTime >= 7) {
      endingSent = true;
      document.body.classList.add('onboarding-ending');
      window.onboarding.animationEnding();
    }
    endingFrame = requestAnimationFrame(observeEnding);
  };
  endingFrame = requestAnimationFrame(observeEnding);
  const timeout = window.setTimeout(() => {
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
