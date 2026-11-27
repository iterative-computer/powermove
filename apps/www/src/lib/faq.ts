import type { Qa } from './site';

/* Short, self-contained answers: each one should still make sense when an
   answer engine quotes it on its own, so every answer names Powermove. */
export const HOME_FAQ: Qa[] = [
  {
    q: 'What is Powermove?',
    a: 'Powermove is the video editor you customize with AI. It is a free, open-source motion and video editor for Macs with Apple silicon. You connect a coding agent such as Claude or Codex, and it adds extensions, restyles the interface, and generates effects and shaders.',
  },
  {
    q: 'What is a malleable video editor?',
    a: 'A malleable video editor is one you can change while you use it. In Powermove, almost everything above a tiny kernel is an extension, including panels, effects, transitions, themes, and keymaps. When the editor is missing something, you ask a coding agent and it builds the extension.',
  },
  {
    q: 'Can I make motion graphics with Claude or Codex?',
    a: 'Yes. Powermove connects to Claude Code, Codex, or an OpenAI-compatible endpoint. Describe what you want in the agent panel, and the agent edits layers, keyframes, and effects through the same undoable tools you use. If an effect does not exist yet, it can write one.',
  },
  {
    q: 'Is Powermove free?',
    a: 'Yes. Powermove is free and open source under the AGPL-3.0 license, and the source code is on GitHub. The agent features use the AI provider you choose with your own account.',
  },
  {
    q: 'Is Powermove an After Effects alternative?',
    a: 'For many motion graphics jobs, yes: Powermove has layers, keyframes, curves, and effects on a timeline. It does not open After Effects projects or run After Effects plugins. Instead, you ask an agent to build the effect you need.',
  },
  {
    q: 'What can Powermove export?',
    a: 'Powermove renders MP4, WebM, or ProRes with sound, a still image, or a PNG sequence. It can also save an editable project with every layer and its media.',
  },
  {
    q: 'Which computers does Powermove run on?',
    a: 'Powermove runs on Macs with Apple silicon. There is no Windows or Linux version today.',
  },
  {
    q: 'Do my projects leave my Mac?',
    a: 'No. Powermove stores projects, media, and renders on your Mac. Prompts go only to the AI provider you choose, and with a local model nothing leaves your machine.',
  },
];
