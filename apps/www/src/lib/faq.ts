import type { Qa } from './site';

/* Short, self-contained answers: each one should still make sense when an
   answer engine quotes it on its own, so every answer names Powermove. */
export const HOME_FAQ: Qa[] = [
  {
    q: 'What is Powermove?',
    a: 'Powermove is a free, open-source video and motion editor for Apple silicon Macs that you can customize with AI. Use Aerolite’s built-in agent, or connect your own Claude Code or Codex account, to add extensions, restyle the workspace, and build custom effects.',
  },
  {
    q: 'What is a malleable video editor?',
    a: 'A malleable editor is one you can shape while you work. In Powermove, panels, effects, transitions, themes, and keymaps are all extensions built on a small core. If the editor is missing a tool you need, ask the agent and it builds it on the spot.',
  },
  {
    q: 'Can I make motion graphics with Claude or Codex?',
    a: 'Yes. Powermove works with its built-in Aerolite agent as well as Claude Code, Codex, or any OpenAI-compatible model. Describe what you want in the agent panel and it edits layers, keyframes, and effects through the same undoable tools you use — and if an effect doesn’t exist yet, it can build one.',
  },
  {
    q: 'Is Powermove free?',
    a: 'Yes. The entire editor is free and open source under AGPL-3.0, with the source code on GitHub. You can bring your own Claude Code or Codex account at no extra cost, or subscribe to Aerolite ($9.99/month) for built-in AI with 1,000 monthly credits.',
  },
  {
    q: 'Is Powermove an After Effects alternative?',
    a: 'For most motion graphics work, yes — Powermove gives you layers, keyframes, easing curves, and GPU effects on a native timeline. It doesn’t open After Effects project files or run After Effects plugins; instead, you can install community extensions or ask the agent to build the effect you need.',
  },
  {
    q: 'What can Powermove export?',
    a: 'Powermove exports MP4, WebM, and ProRes video with audio, as well as still frames and PNG sequences. You can also save a self-contained project with every layer and its media.',
  },
  {
    q: 'Which computers does Powermove run on?',
    a: 'Powermove runs natively on Apple silicon Macs (M1 and newer). Windows and Linux aren’t supported yet.',
  },
  {
    q: 'Do my projects leave my Mac?',
    a: 'No. Your projects, media, and renders stay local on your Mac. Only your prompts are sent to the AI provider you choose — and with a local model, nothing leaves your machine.',
  },
];
