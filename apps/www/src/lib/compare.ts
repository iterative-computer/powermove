import type { Qa } from './site';

/* The Powermove vs After Effects page, as data so llms-full.txt can reuse it. */

export const VS_AE_TITLE = 'Powermove vs After Effects: the editor you customize with AI';
export const VS_AE_DESCRIPTION =
  'An honest comparison. After Effects has decades of plugins. Powermove is the video editor you customize with AI: free, open source, and extended by your agent.';

export const VS_AE_ROWS: { topic: string; powermove: string; ae: string }[] = [
  { topic: 'The idea', powermove: 'The video editor you customize with AI. A tiny kernel; everything else is an extension your agent can add to.', ae: 'A complete, fixed toolset from Adobe, extended with plugins and scripts.' },
  { topic: 'When a feature is missing', powermove: 'Ask Claude or Codex in the agent panel. It writes the extension.', ae: 'Buy a third-party plugin, find a script, or build it from expressions.' },
  { topic: 'Changing the interface', powermove: 'The agent restyles themes, panels, and keymaps on request.', ae: 'Rearrange panels and save workspaces.' },
  { topic: 'Effects', powermove: 'Built-in effects and transitions, plus effects and shaders your agent generates.', ae: 'A large built-in set and a deep third-party plugin market.' },
  { topic: 'Price', powermove: 'Free and open source (AGPL-3.0).', ae: 'Paid Creative Cloud subscription.' },
  { topic: 'Platforms', powermove: 'Macs with Apple silicon.', ae: 'macOS and Windows.' },
  { topic: 'Export', powermove: 'MP4, WebM, or ProRes with sound, stills, PNG sequences, or an editable project.', ae: 'Many formats through the render queue and Adobe Media Encoder.' },
  { topic: 'Maturity', powermove: 'New. A small but growing set of extensions.', ae: 'The industry standard since the 1990s, with templates, tutorials, and plugins for almost anything.' },
];

export const VS_AE_FAQ: Qa[] = [
  {
    q: 'Is Powermove a free alternative to After Effects?',
    a: 'For many motion graphics jobs, yes. Powermove is a free, open-source editor with layers, keyframes, curves, and effects on a timeline. It is the video editor you customize with AI: when it lacks a feature, a coding agent such as Claude or Codex adds it.',
  },
  {
    q: 'Can Powermove open After Effects projects or run After Effects plugins?',
    a: 'No. Powermove does not open .aep files or load After Effects plugins. Instead, you describe the effect you need and an agent builds it as a Powermove extension.',
  },
  {
    q: 'Will After Effects shortcuts feel familiar in Powermove?',
    a: 'Several do. Powermove’s default keymap follows After Effects for ⌘N (new composition), ⌥⌘N (new project), ⌥⌘Y (new adjustment layer), and ⇧⌘C (precompose).',
  },
  {
    q: 'Does Powermove run on Windows?',
    a: 'No. Powermove runs on Macs with Apple silicon. After Effects runs on macOS and Windows.',
  },
];
