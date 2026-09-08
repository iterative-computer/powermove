import { createPlayer } from './player.js';

export const EXPECTED_DURATION = 9.766666666666667;

export function hollowGlowScene(scene) {
  const glow = scene.effects?.find((effect) => effect.id === 'glow');
  if (!glow || typeof glow.frag !== 'string' || glow.keepOrig !== true) {
    throw new Error('The onboarding glow effect is missing its original-alpha mask');
  }
  const originalOutput = /o = u_behindLayer > \.5\s*\? original \+ coloredGlow \* \(1\. - original\.a\)\s*: original \+ coloredGlow;/;
  if (!originalOutput.test(glow.frag)) {
    throw new Error('The onboarding glow output did not match the supplied export');
  }
  glow.frag = glow.frag.replace(originalOutput, 'o = coloredGlow * (1. - original.a);');
  return scene;
}

async function start() {
  const canvas = document.querySelector('#onboarding-canvas');
  if (!(canvas instanceof HTMLCanvasElement)) throw new Error('Onboarding canvas is unavailable');

  const response = await fetch('./scene.json');
  if (!response.ok) throw new Error(`Could not load onboarding scene (${response.status})`);
  const scene = hollowGlowScene(await response.json());
  if (Math.abs(scene.project.dur - EXPECTED_DURATION) > 1e-9) {
    throw new Error('The supplied onboarding timing has changed');
  }

  let player;
  let complete = false;
  let observedPlaying = false;
  let frame = 0;
  const finish = () => {
    if (complete) return;
    complete = true;
    cancelAnimationFrame(frame);
    player?.destroy();
    window.onboarding.animationComplete();
  };
  player = await createPlayer({
    canvas,
    scene,
    baseURL: new URL('./', window.location.href),
    transparent: true,
    audio: true,
    autoplay: false,
    loop: false,
    onError: (error) => window.onboarding.animationFailed(error?.message ?? String(error))
  });
  player.play();
  const observePlayback = () => {
    observedPlaying ||= player.playing;
    const finalFrame = player.duration - 1 / scene.project.fps;
    if (observedPlaying && !player.playing && player.currentTime >= finalFrame - 1e-4) finish();
    else frame = requestAnimationFrame(observePlayback);
  };
  frame = requestAnimationFrame(observePlayback);
  // The native flow has its own longer watchdog. This renderer fallback keeps
  // a failed player state from hanging even if its error callback is missed.
  window.setTimeout(() => {
    if (!complete) window.onboarding.animationFailed('Onboarding playback timed out');
  }, (EXPECTED_DURATION + 15) * 1000);
  window.addEventListener('pagehide', () => {
    cancelAnimationFrame(frame);
    player?.destroy();
  }, { once: true });
}

start().catch((error) => {
  window.onboarding.animationFailed(error?.message ?? String(error));
});
