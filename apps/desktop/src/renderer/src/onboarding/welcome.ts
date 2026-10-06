import { mount } from 'svelte';
import AgentStep from './AgentStep.svelte';
import TimelineStep from './TimelineStep.svelte';
import TranscriptionStep from './TranscriptionStep.svelte';
import WorkspaceStep from './WorkspaceStep.svelte';
import type { OnboardingAgentChoice, OnboardingTimelineMode } from '../../../shared/creative-workspace';
import '@powermove/tokens/tokens.css';
import './welcome.css';

const button = document.querySelector<HTMLButtonElement>('#begin');
const welcome = document.querySelector<HTMLElement>('.welcome-card');
const timelineStep = document.querySelector<HTMLElement>('#timeline-step');
const agentStep = document.querySelector<HTMLElement>('#agent-step');
const transcriptionStep = document.querySelector<HTMLElement>('#transcription-step');
const step = document.querySelector<HTMLElement>('#workspace-step');
let timelineMode: OnboardingTimelineMode = 'layers';
let agent: OnboardingAgentChoice | null = null;

/* Welcome → timeline style → agent → speech model → workspace suitcase. Only the last step begins. */
const show = (next: HTMLElement | null, focus: string) => {
  for (const view of [welcome, timelineStep, agentStep, transcriptionStep, step]) if (view) view.hidden = view !== next;
  next?.querySelector<HTMLElement>(focus)?.focus();
};

void window.onboarding?.appearance?.().then(theme => { document.documentElement.dataset.theme = theme; }).catch(() => undefined);

if (timelineStep) mount(TimelineStep, { target: timelineStep, props: {
  onback: () => show(welcome, '#begin'),
  oncontinue: mode => { timelineMode = mode; show(agentStep, '#agent-title'); }
} });
if (agentStep) mount(AgentStep, { target: agentStep, props: {
  onback: () => show(timelineStep, '#timeline-title'),
  oncontinue: choice => { agent = choice; show(transcriptionStep ?? step, transcriptionStep ? '#transcription-title' : '#workspace-title'); }
} });
/* Optional: choosing a model starts its download in main; it outlives onboarding. */
if (transcriptionStep) mount(TranscriptionStep, { target: transcriptionStep, props: {
  bridge: window.onboarding?.transcription ?? null,
  onback: () => show(agentStep, '#agent-title'),
  oncontinue: () => show(step, '#workspace-title')
} });
if (step) mount(WorkspaceStep, { target: step, props: {
  onback: () => show(transcriptionStep ?? agentStep, transcriptionStep ? '#transcription-title' : '#agent-title'),
  onbegin: choice => window.onboarding.begin({ ...choice, timelineMode, agent })
} });
const mark = document.querySelector<SVGPathElement>('.welcome-mark path');

const reportLogoTarget = () => {
  if (!mark) return;
  const { x, y, width, height } = mark.getBoundingClientRect();
  if (window.onboarding) window.onboarding.reportLogoTarget({ x, y, width, height });
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

button?.addEventListener('click', () => {
  if (!timelineStep || !welcome) return;
  show(timelineStep, '#timeline-title');
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
