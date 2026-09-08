import '../../../../css/tokens.css';
import './welcome.css';

const button = document.querySelector<HTMLButtonElement>('#begin');
const replay = document.querySelector<HTMLButtonElement>('#replay');
const status = document.querySelector<HTMLElement>('#begin-status');
const mark = document.querySelector<SVGPathElement>('.welcome-mark path');

const reportLogoTarget = () => {
  if (!mark) return;
  const { x, y, width, height } = mark.getBoundingClientRect();
  window.onboarding.reportLogoTarget({ x, y, width, height });
};

let scheduledReport = 0;
const scheduleLogoTarget = () => {
  if (scheduledReport) return;
  scheduledReport = requestAnimationFrame(() => {
    scheduledReport = 0;
    reportLogoTarget();
  });
};

void document.fonts.ready.then(scheduleLogoTarget);
if (mark) new ResizeObserver(scheduleLogoTarget).observe(mark);
window.addEventListener('resize', scheduleLogoTarget);

button?.addEventListener('click', async () => {
  button.disabled = true;
  if (status) status.textContent = 'Opening your workspace…';
  try {
    await window.onboarding.begin();
  } catch {
    button.disabled = false;
    if (status) status.textContent = 'Could not open Powermove. Try again.';
  }
});

button?.focus();

replay?.addEventListener('click', async () => {
  replay.disabled = true;
  if (status) status.textContent = 'Replaying…';
  try {
    await window.onboarding.replay();
  } catch {
    replay.disabled = false;
    if (status) status.textContent = 'Could not replay the animation. Try again.';
  }
});
