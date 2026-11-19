# @powermove/www

The Powermove marketing site. SvelteKit + Svelte 5 (runes), prerendered to
static HTML with `@sveltejs/adapter-static`.

## Commands

Run from the repo root (bun workspaces) or from this directory:

```sh
bun install                       # once, from the repo root
bun run --cwd apps/www dev    # http://localhost:5173
bun run --cwd apps/www build  # static output in apps/www/build/
bun run --cwd apps/www preview
bun run --cwd apps/www check  # svelte-check (alias: typecheck)
```

## Where things come from

- **Design tokens**: `@powermove/tokens/tokens.css` (`packages/tokens`), imported
  once in `src/routes/+layout.svelte`. The site does not keep its own copy.
- **Site styles**: `src/app.css`, a straight port of the old `globals.css` with
  the same class names. No Tailwind.
- **Font**: Geist variable, self-hosted at `static/fonts/geist-latin-variable.woff2`
  and declared via `@font-face` in the layout as `--font-geist-sans`.
- **Hero animation**: `src/lib/HeroAnimation.svelte` fetches `/hero/scene.json`
  and drives it through `$lib/player`. That module is a one-line re-export and
  is the only place the player implementation is referenced; today it points
  at the local copy in `src/lib/powermove/`, and will point at
  `@powermove/player/svg-player` once that package lands.

## Search and AI answer engines

- **Positioning and metadata**: `src/lib/site.ts` holds the headline ("The video
  editor you customize with AI"), the home title and description, the page list,
  and the JSON-LD builders. `src/lib/Seo.svelte` writes each page's title,
  description, canonical, Open Graph, and JSON-LD.
- **Generated files**: `sitemap.xml`, `llms.txt`, and `llms-full.txt` are
  prerendered routes built from the same data (`src/routes/*/+server.ts`), so a
  new page in `PAGES` or a new clone shows up everywhere. `static/robots.txt`
  allows crawlers and points at the sitemap.
- **FAQ**: `src/lib/faq.ts`. Keep answers short and self-contained; answer
  engines quote them one at a time.
- **Effect clones**: add an entry to `src/lib/clones.ts` and its short to
  `static/clones/<slug>.mp4` (720×1280 H.264, no audio, `+faststart`) with a
  poster and a 16:9 thumb. `/clones/<slug>` renders from that entry.

## Deployment

`bun run build` writes a fully prerendered site to `build/` (`index.html` plus
hashed assets under `_app/`). It works on any static host (Netlify, Vercel
static, GitHub Pages, S3, nginx...). If Cloudflare Pages is chosen,
`@sveltejs/adapter-cloudflare` is a drop-in replacement for `adapter-static`
in `svelte.config.js`; the prerender settings stay the same.

Cloudflare Web Analytics is included only when `VITE_CF_BEACON_TOKEN` is set
at build time. Put it in the gitignored `apps/www/.env`; the token comes from
`cf rum site-info list` on the Motioner account. Builds without it print a
warning and ship no beacon.
