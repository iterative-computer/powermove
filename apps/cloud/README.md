# Powermove Cloud

The hosted backend for the desktop app: a Hono Worker on Cloudflare that runs
identity and the extension Store. Postgres on Neon through Drizzle, R2 for
git objects, release tarballs and icons, Better Auth for accounts.

The contract with the desktop lives in `packages/registry` (`wire/` schemas,
manifest parser, snapshot hashing, git codec, tar, scanner). Every request
body is validated with those schemas here and every response is parsed with
them on the desktop.

## Layout

- `src/app.ts` builds the app from a data-layer factory; `src/index.ts` binds
  it to Neon and exports `fetch` + `scheduled` (GC).
- `src/routes/` one file per resource: `auth-desktop` (the PKCE hand-off),
  `auth-email`, `auth-signout`, `me`, `objects`, `publish`, `repo-management`
  (yank, patch, tombstone, moderation, reports, icons), `store` (public
  reads, versions batch), `installs`, `health`.
- `src/lifecycle.ts` is the state table (visibility × moderation × tombstone
  × yank). Every read decides through it.
- `src/objects/` presence, references (the one reachability predicate shared
  by presence and GC), the streaming multipart parser.
- `src/db/` Drizzle schema (`schema.ts`, `auth-schema.ts`) and `client.ts`
  (the `Data` type: HTTP reads plus a WebSocket pool for transactions).
- `drizzle/` generated migrations, applied in order. Never hand-edit.
- `test/` bun:test suites on PGlite plus an in-memory R2 fake.

## Local commands

From `apps/cloud`:

```sh
bun run types        # regenerate worker-configuration.d.ts from wrangler.jsonc
bun run typecheck
bun run types:emit   # emits dist/types (AppType) for the desktop's typed client
bun test
bun run build        # wrangler deploy --dry-run
bun run dev          # wrangler dev on http://localhost:8787 (500s without DATABASE_URL)
```

`bun test` runs on an in-memory PGlite database with the checked-in
migrations. Set `TEST_DATABASE_URL` to run the same suite against a
disposable Postgres; the helper truncates tables between tests. Files run
serially because PGlite has one connection, so the "two racing first
publishes" invariant is exercised through a test hook rather than real
concurrency.

`worker-configuration.d.ts` is generated and git-ignored. The bundle aliases
Better Auth's unused Kysely fallback to `src/kysely-unavailable.ts` so the
optional PGlite dialect never enters the Worker.

## Provisioning (PENDING(provision))

Nothing below has been created yet. Do these once, in order.

1. **Neon.** Project with branches `main` (production) and `dev`. Apply the
   migrations in `drizzle/*.sql` in order to each (`bunx drizzle-kit migrate`
   with `DATABASE_URL` set, or `psql -f`). CI may create a branch per run.
2. **R2.** Buckets `powermove-objects`, `powermove-tars`, `powermove-icons`
   (names in `wrangler.jsonc`). No public access; the Worker serves them.
3. **OAuth apps.** Google and GitHub, redirect URI
   `https://cloud.trypowermove.com/v1/auth/callback/<provider>`.
4. **Resend.** Verified sending domain `users.trypowermove.com` (also the
   handle mail domain used in commit identities).
5. **DNS.** `cloud.trypowermove.com` as the Worker's custom domain.
6. **Workers Paid plan** (interactive Postgres transactions and the Rate
   Limiting binding). Rate limiters are declared as `unsafe.bindings` in
   `wrangler.jsonc`; confirm the account has them enabled.
7. **Secrets** (`bunx wrangler secret put <NAME>`), never in `vars` or git:
   `DATABASE_URL`, `BETTER_AUTH_SECRET` (≥ 32 random chars),
   `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GITHUB_CLIENT_ID`,
   `GITHUB_CLIENT_SECRET`, `RESEND_API_KEY`, `ADMIN_TOKEN` (moderation
   endpoint; unset means moderation is disabled).
8. **Deploy** `bun run deploy`, then `curl https://cloud.trypowermove.com/health`.
9. **The `powermove` publisher.** Sign in once from the desktop app with the
   account that will own the built-ins (skip the handle step), find its user
   id in the `user` table, then seed the reserved handle with the admin token:
   `curl -X POST https://cloud.trypowermove.com/v1/admin/publishers -H "X-Admin-Token: …" -H "content-type: application/json" -d '{"handle":"powermove","userId":"<id>"}'`.
   Reserved handles (`src/handles.ts`) can only be claimed this way.
10. **Built-ins.** See below.

Without `GOOGLE_*`/`GITHUB_*` the social providers are omitted so local boot
works. Without `RESEND_API_KEY` email sign-in fails with `bad_request`; set
`DEV_LOG_OTP=1` in a local `.dev.vars` to print codes to the console
instead. Never set that in production.

## Publishing built-ins

The ten extensions in `apps/desktop/src/extensions/` are published under
`powermove/<id>` by `bun run --cwd apps/desktop publish:builtins`, which
drives the same objects + publish protocol as the desktop. A changed tree
must bump `version` in its `manifest.json`; `bun run check` at the repo root
(or `check:builtins` in `apps/desktop`) compares each tree hash with
`src/extensions/.builtin-versions.json` and fails otherwise. After bumping,
refresh the file with `check:builtins --update`. A pull-request workflow
runs the check; the release workflow publishes when `POWERMOVE_REGISTRY_TOKEN`
is configured and skips otherwise.

`--dry-run` prints coordinates, versions, tree hashes and file counts with no
network. Re-running is safe: an unchanged tree at an existing version is a
skip. PENDING(provision): issuing the `powermove` bearer token to the release
engineer is a manual step (sign in as that account, or add an admin
token-minting route later); do not commit it anywhere.

## Operations

- **GC** runs on the cron (`0 */6 * * *`): claim unreferenced, unleased
  objects idle for 24 h → recheck → mark `deleting` → delete the R2 key →
  delete the row. It also expires leases and `desktop_auth` rows.
- **Moderation**: `POST /v1/admin/repos/:repoId/moderation` with header
  `X-Admin-Token`; actions `hide`, `unhide`, `remove`. Each writes
  `moderation_log`. Reports arrive at `POST /v1/store/x/:handle/:slug/report`.
- **Lifecycle** (what each state means for browse, detail, files, updates,
  installs and forks) is the table in `docs-private/store-plan.md` §2.6 and
  the code in `src/lifecycle.ts`.

## Auth schema note

`src/db/auth-schema.ts` was written by hand (the Better Auth CLI could not run
in the build sandbox) and later compared column for column with
`@better-auth/cli generate` output for core + username + email-otp +
one-time-token: identical. Regenerate and diff again after a Better Auth
upgrade.
