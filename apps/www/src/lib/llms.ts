import { HOME_FAQ } from './faq';
import { DISCORD, GITHUB, RELEASES } from './links';
import { HEADLINE, MALLEABLE, absolute } from './site';

/* llms.txt (https://llmstxt.org) and its long form, built from the same data
   as the pages so the two never drift apart. */

const SUMMARY = `${HEADLINE}. ${MALLEABLE} Powermove is free and open source (AGPL-3.0), for Macs with Apple silicon.`;

const FACTS = [
  'Powermove is a native Rust and Metal video and motion editor for Apple silicon Macs, with layers, keyframes, easing curves, and GPU effects on a timeline.',
  'Works with Powermove’s built-in Aerolite agent ($9.99/month, 1,000 AI credits monthly) as well as your own Claude Code, Codex, or OpenAI-compatible model.',
  'A deliberately tiny kernel (project store, typed edits, undo, compositor). Panels, effects, transitions, themes, and keymaps are extensions, and the agent can write new ones on the spot.',
  'Extensions can be turned off in Settings and back on when needed. Examples include motion tracking and audio-reactive visuals.',
  'Export MP4, WebM, or ProRes video with audio, still frames, PNG sequences, or a self-contained editable project with every layer and its media.',
  'Projects, media, and renders stay local on your Mac. Prompts go only to the AI provider you choose; with a local model nothing leaves your machine.',
  'Free and open source under AGPL-3.0. Runs natively on Apple silicon Macs (M1 and newer).',
];

export function llmsTxt(downloadHref: string): string {
  return `# Powermove

> ${SUMMARY}

${FACTS.map((fact) => `- ${fact}`).join('\n')}

## Pages

- [Home](${absolute('/')}): ${HEADLINE}. What Powermove is, what it does, and FAQs.

## Get Powermove

- [Download for Mac](${downloadHref}): Latest release (.dmg, Apple silicon).
- [Source code on GitHub](${GITHUB}): AGPL-3.0.
- [Releases](${RELEASES})
- [Discord](${DISCORD})

## Optional

- [Full text for language models](${absolute('/llms-full.txt')})
- [Privacy policy](${absolute('/privacy')})
- [Terms of service](${absolute('/terms')})
`;
}

export function llmsFullTxt(downloadHref: string): string {
  const qa = (items: { q: string; a: string }[]) => items.map(({ q, a }) => `### ${q}\n\n${a}`).join('\n\n');
  return `# Powermove

> ${SUMMARY}

Website: ${absolute('/')}
Download: ${downloadHref}
Source: ${GITHUB}

## What Powermove is

${FACTS.map((fact) => `- ${fact}`).join('\n')}

## Frequently asked questions

${qa(HOME_FAQ)}
`;
}
