import { DISCORD, GITHUB } from './links';
import type { Download } from './releases';

/* One place for the words search engines and AI answer engines quote. The
   headline leads every title, description, and structured-data block; the
   pages, sitemap.xml, llms.txt, and llms-full.txt all read from here. */

export const SITE_URL = 'https://trypowermove.com';
export const HEADLINE = 'The video editor you customize with AI';

export const HOME_TITLE = `${HEADLINE} · Powermove for Mac`;
export const HOME_DESCRIPTION =
  'The video editor you customize with AI. Connect Claude or Codex to Powermove to add extensions, restyle the interface, and generate effects. Free, open source.';

/** The plain-language definition of "malleable", reused wherever we explain it. */
export const MALLEABLE =
  'Powermove is a malleable video editor: you connect a coding agent such as Claude or Codex, and it adds extensions, restyles the interface, and generates effects and shaders while you work.';

export const OG_IMAGE = `${SITE_URL}/og.png?v=3`;
export const OG_IMAGE_ALT = 'Powermove, with the editor over a desert night background.';

export const ORG_ID = `${SITE_URL}/#organization`;
export const APP_ID = `${SITE_URL}/#app`;
export const SITE_ID = `${SITE_URL}/#website`;

export const absolute = (path: string) => (path.startsWith('http') ? path : SITE_URL + path);

/** Every indexable page, in sitemap order. */
export const PAGES = [
  { path: '/', title: HOME_TITLE },
  { path: '/privacy', title: 'Privacy Policy' },
  { path: '/terms', title: 'Terms of Service' },
] as const;

type Json = Record<string, unknown>;

export function organization(): Json {
  return {
    '@type': 'Organization',
    '@id': ORG_ID,
    name: 'Iterative Computer',
    url: 'https://iterative.computer',
    email: 'hello@iterative.computer',
    logo: absolute('/powermove-logo.png'),
    sameAs: ['https://github.com/iterative-computer', DISCORD],
  };
}

export function website(): Json {
  return { '@type': 'WebSite', '@id': SITE_ID, url: SITE_URL + '/', name: 'Powermove', description: HOME_DESCRIPTION, publisher: { '@id': ORG_ID } };
}

export function softwareApplication(download: Download): Json {
  return {
    '@type': 'SoftwareApplication',
    '@id': APP_ID,
    name: 'Powermove',
    alternateName: HEADLINE,
    description: `${HEADLINE}. ${MALLEABLE}`,
    url: SITE_URL + '/',
    applicationCategory: 'MultimediaApplication',
    applicationSubCategory: 'Video editor',
    operatingSystem: 'macOS (Apple silicon)',
    ...(download.version ? { softwareVersion: download.version } : {}),
    downloadUrl: download.href,
    installUrl: download.href,
    isAccessibleForFree: true,
    license: 'https://www.gnu.org/licenses/agpl-3.0.html',
    offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
    screenshot: absolute('/editor-window.png'),
    image: OG_IMAGE,
    featureList: [
      'Connect a coding agent (Claude, Codex, or an OpenAI-compatible endpoint) to customize the editor',
      'Agent-built extensions, interface themes, effects, and shaders',
      'Layers, keyframes, curves, and effects on a timeline',
      'Export MP4, WebM, or ProRes with sound, stills, PNG sequences, or an editable project',
      'Free and open source (AGPL-3.0)',
    ],
    sameAs: [GITHUB],
    publisher: { '@id': ORG_ID },
    author: { '@id': ORG_ID },
  };
}

export type Qa = { q: string; a: string };

export function faqPage(items: Qa[], path: string): Json {
  return {
    '@type': 'FAQPage',
    '@id': `${SITE_URL}${path === '/' ? '/' : path}#faq`,
    mainEntity: items.map(({ q, a }) => ({ '@type': 'Question', name: q, acceptedAnswer: { '@type': 'Answer', text: a } })),
  };
}

/** Serialize a JSON-LD graph for a <script> tag; `<` is escaped so text can never close it. */
export function jsonLd(nodes: Json[]): string {
  return JSON.stringify({ '@context': 'https://schema.org', '@graph': nodes }).replace(/</g, '\\u003c');
}
