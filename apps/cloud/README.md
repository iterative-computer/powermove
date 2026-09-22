# @powermove/cloud

The hosted backend for Powermove. Extension registry first, collaboration later. AGPL-3.0-or-later like the rest of the repo, and self-hostable.

## Stack

- Hono on Cloudflare Workers, run and deployed with wrangler. Docs: https://hono.dev
- Request validation with `@hono/zod-validator`. Errors are thrown as `HTTPException` and mapped in one `onError`.
- The desktop imports `AppType` and calls the API through `hc` from `hono/client`, so routes are chained in `src/index.ts` to keep the type complete.
- Postgres on Neon with Drizzle, R2 for extension tarballs. Not wired yet.

## Commands

```
bun run --cwd apps/cloud dev        # wrangler dev on http://localhost:8787
bun run --cwd apps/cloud typecheck  # wrangler types + tsc
bun run --cwd apps/cloud test       # bun test, uses app.request()
bun run --cwd apps/cloud deploy
```

## Layout

```
src/index.ts      app, notFound, onError, route mounting, AppType
src/env.ts        binding types (generated CloudflareBindings)
src/routes/*.ts   one Hono instance per resource, handlers inline
```
