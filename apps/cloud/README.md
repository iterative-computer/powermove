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
  it to Neon (or local Postgres with `LOCAL_POSTGRES=1`) and exports `fetch` + `scheduled` (GC).
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
bun run dev:emulate  # seeded Google OAuth emulator on http://localhost:4002
```

`bun test` runs on an in-memory PGlite database with the checked-in
migrations. Set `TEST_DATABASE_URL` to run the same suite with node-postgres against a
disposable Postgres; the helper truncates tables between tests. Files run
serially because PGlite has one connection, so the "two racing first
publishes" invariant is exercised through a test hook rather than real
concurrency.

`worker-configuration.d.ts` is generated and git-ignored. The bundle aliases
Better Auth's unused Kysely fallback to `src/kysely-unavailable.ts` so the
optional PGlite dialect never enters the Worker.

## Local stack

Requires Bun and OrbStack or Docker Desktop with Docker Compose running. From
the repository root, run these commands in order:

```sh
cd apps/cloud
bun run local:up
bun run dev:local
```

Leave the Worker running. In a second terminal:

```sh
cd apps/cloud
bun run local:seed --publish
```

This prints a `POWERMOVE_REGISTRY_TOKEN` for Jude and a
`MARA_REGISTRY_TOKEN` for Mara. It publishes the ten built-ins and the four
sample extensions to the local Store. The publisher needs the Worker on port
8787. In a third terminal:

```sh
cd apps/desktop
bun run dev:local
```

The desktop's Store now uses `http://localhost:8787`. Settings › Advanced
shows the Registry URL marked “(from environment)”. The local override is
active only in the unpackaged development app; it does not replace the stored
Registry URL. `normalizeOrigin` accepts this HTTP localhost origin.

`local:up` creates `.dev.vars` from `.dev.vars.example` only when missing.
Local Postgres is at `localhost:54329`; the three R2 buckets, rate limits,
scheduled handler and Email Sending are simulated by Wrangler. R2 objects
live under `apps/cloud/.wrangler/state/`. With the example's `DEV_LOG_OTP=1`,
email sign-in codes appear in the `dev:local` terminal as `OTP for …: …`.
When `DEV_LOG_OTP` is disabled, Wrangler logs simulated email and prints the
saved text file path (typically `/tmp/miniflare-…/files/email-text/<id>.txt`).

For Google sign-in instead, run `bun run dev:emulate` from `apps/cloud` in
another terminal, uncomment the three Google variables in `.dev.vars`, then
restart `dev:local`. The emulator offers Jude and Mara accounts. The OAuth
callback returns to the development desktop via `powermove://` because
`POWERMOVE_DEV_PROTOCOL=1` is set by `dev:local`.

Smoke checklist:

1. Sign in on the desktop with a fresh `@localhost` email and the code printed
   by Wrangler.
2. Claim a handle. The seed has already assigned `powermove` to Jude and
   `mara` to Mara.
3. Browse the Store and see the ten `powermove/*` built-ins.
4. Install one built-in.
5. Publish a local extension under the signed-in handle.
6. Sign in as `mara@localhost`, then fork and publish someone else's extension.

To delete local Postgres and all local R2 state, stop the Worker and run
`bun run local:down` from `apps/cloud`. This removes the Compose named volume
and `.wrangler/state/`. `.dev.vars` is kept so local settings survive a reset.

### Sample extensions

`bun run local:seed --publish` publishes `mara/glass-tint@1.0.0`,
`mara/ease-lab@1.0.0`, `mara/colour-match@1.0.0`, and
`mara/wipe-set@1.0.0`. To publish just the samples again, from `apps/cloud`:

```sh
bun run local:samples
```

The command uses `MARA_REGISTRY_TOKEN` when set, or reads Mara's latest
unexpired local session from Postgres. Existing identical versions are skipped.
Keep the Worker running at `http://localhost:8787`.

Try these flows in the desktop Store:

1. Open `mara/glass-tint` and install 1.0.0. It appears in Library; apply its
   effect from Effects & Presets. Then run `bun run local:samples --update`
   from `apps/cloud`. The command publishes 1.1.0 with an Edge control and a
   What's new note. Return to Library, check for updates, and choose Update.
2. Install `mara/colour-match`. Install and set up asks for the required OpenAI
   API key and offers the optional palette size. Run “Check Colour Match
   connection” from the command palette to see an HTTP status or an offline
   error toast. The sample does not modify the project.
3. Install `mara/ease-lab`, open its Ease lab panel, choose a curve, and press
   Apply. The panel remembers the preset. Alt+Shift+E runs the same command.
   Install `mara/wipe-set` to try its three layer transitions.
4. To make a fork under `jude`, sign in with a fresh `@localhost` account and
   claim the `jude` handle (seeded `jude@localhost` owns `powermove`). Install
   `mara/ease-lab`, reveal its folder from Library, edit a source file, then
   choose Publish. The Store shows “Forked from mara/ease-lab” on the new
   listing. Open your published version's detail page and choose Withdraw…;
   the installed copy remains available locally. Choose Uninstall… in Library
   when you are finished.

The Store detail page lists older versions but currently offers installation
of the latest only, so `--update` publishes Glass Tint 1.1.0 after you install
1.0.0.

## Provisioning (PENDING(provision))

Nothing below has been created yet. Do these once, in order.

1. **Neon.** Project with branches `main` (production) and `dev`. Apply the
   migrations in `drizzle/*.sql` in order to each (`bunx drizzle-kit migrate`
   with `DATABASE_URL` set, or `psql -f`). CI may create a branch per run.
2. **R2.** Buckets `powermove-objects`, `powermove-tars`, `powermove-icons`
   (names in `wrangler.jsonc`). No public access; the Worker serves them.
3. **OAuth app.** Google, redirect URI
   `https://cloud.trypowermove.com/v1/auth/oauth2/callback/google`.
4. **Cloudflare Email Service.** Onboard the sending domain
   `trypowermove.com` (PENDING(provision)). The `send_email` binding allows
   `sign-in@trypowermove.com` as its sender and requires Workers Paid.
5. **DNS.** `cloud.trypowermove.com` as the Worker's custom domain.
6. **Workers Paid plan** (interactive Postgres transactions and the Rate
   Limiting binding). Rate limiters are declared as `unsafe.bindings` in
   `wrangler.jsonc`; confirm the account has them enabled.
7. **Secrets** (`bunx wrangler secret put <NAME>`), never in `vars` or git:
   `DATABASE_URL`, `BETTER_AUTH_SECRET` (≥ 32 random chars),
   `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `ADMIN_TOKEN` (moderation
   endpoint; unset means moderation is disabled).
8. **Deploy** `bun run deploy`, then `curl https://cloud.trypowermove.com/health`.
9. **The `powermove` publisher.** Sign in once from the desktop app with the
   account that will own the built-ins (skip the handle step), find its user
   id in the `user` table, then seed the reserved handle with the admin token:
   `curl -X POST https://cloud.trypowermove.com/v1/admin/publishers -H "X-Admin-Token: …" -H "content-type: application/json" -d '{"handle":"powermove","userId":"<id>"}'`.
   Reserved handles (`src/handles.ts`) can only be claimed this way.
10. **Built-ins.** See below.

Without `GOOGLE_CLIENT_ID` Google sign-in is omitted so local boot works.
The Email Sending binding delivers sign-in codes. Local `wrangler dev`
simulates delivery by logging messages and writing them to local files.
`DEV_LOG_OTP=1` prints codes to the console even when the binding is present.
Never set that in production.

## Local sign-in with the Google emulator

Run `bun run dev:emulate` alongside `bun run dev`. The emulator seeds the
OAuth client `powermove-local` and two users, `jude@example.com` and
`mara@example.com`. Its consent page offers a button for each seeded user;
clicking one posts the user's email and OAuth request fields to
`/o/oauth2/v2/auth/callback`.

Put these values in local `.dev.vars`:

```dotenv
GOOGLE_CLIENT_ID=powermove-local
GOOGLE_CLIENT_SECRET=powermove-local-secret
GOOGLE_DISCOVERY_URL=http://localhost:4002/.well-known/openid-configuration
APP_ORIGIN=http://localhost:8787
DEV_LOG_OTP=1
```

`GOOGLE_DISCOVERY_URL` is optional and defaults to Google's production OIDC
discovery URL. The emulator documents local and self-hosted `--base-url`
setups; its docs do not describe a hosted `emulators.dev` instance. A
deployed development Worker needs an HTTPS-reachable emulator and a matching
registered callback URI.

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
