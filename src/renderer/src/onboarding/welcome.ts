import '../../../../css/tokens.css';
import './welcome.css';

const button = document.querySelector<HTMLButtonElement>('#begin');
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
document.querySelector('.welcome-card')?.addEventListener('animationend', scheduleLogoTarget);

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

let entranceStarted = false;
const startEntrance = async () => {
  if (entranceStarted) return;
  entranceStarted = true;
  await document.fonts.ready;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    document.body.classList.add('welcome-entering');
    scheduleLogoTarget();
  }));
};
window.addEventListener('focus', startEntrance, { once: true });
if (document.hasFocus()) void startEntrance();
