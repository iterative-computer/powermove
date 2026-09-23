# Powermove Cloud

Hono Worker for identity and the extension registry. The cloud schema and initial migration are in `src/db/` and `drizzle/`. The P4 routes are desktop and email authentication, account settings, handle claim, account deletion, and account repos. Later units add store and publish routes.

## Local commands

From `apps/cloud`:

```sh
bun run types
bun run typecheck
bun run types:emit
bun test
bun run build
bun run dev
```

`bun test` uses an in-memory PGlite database and applies the checked-in SQL migration. Set `TEST_DATABASE_URL` to run against a disposable Postgres database instead; the test helper truncates tables before each test. The test files are serial because PGlite has one connection.

The generated `worker-configuration.d.ts` is ignored by Git. `bun run types` skips Wrangler's runtime type generation because that step starts a local listener and fails in restricted sandboxes. The TypeScript configs refer to the installed Cloudflare Workers type package in Bun's cache. Regenerate types after changing `wrangler.jsonc`. The Worker bundle aliases Better Auth's unused Kysely fallback to `src/kysely-unavailable.ts`; this Worker always provides the Drizzle adapter, and the alias prevents Kysely's optional PGlite dialect from entering the bundle.

## Publishing built-ins

The ten extensions in `apps/desktop/src/extensions/` are published under `powermove/<id>`. A changed tree requires a new `version` in its `manifest.json`. Run `bun run --cwd apps/desktop check:builtins` before a release and update `.builtin-versions.json` with `bun run --cwd apps/desktop check:builtins --update` when versions have been bumped. The pull request workflow checks this rule.

Run `bun run --cwd apps/desktop publish:builtins --dry-run` to inspect coordinates, versions, tree hashes and file counts without network access. For a real publish, set `POWERMOVE_REGISTRY_TOKEN` to a bearer token for the `powermove` publisher, optionally set `POWERMOVE_REGISTRY_URL` (default `https://cloud.trypowermove.com`), then run `bun run --cwd apps/desktop publish:builtins`. `--notes "text"` overrides the default release notes. The command can be run again; releases with the same version and tree are skipped.

PENDING(provision): Establish a manual process for issuing and delivering the `powermove` publisher token to the release engineer. There is no admin token-creation endpoint or supported keychain export flow yet. Do not put the token in this repository or a workflow secret until provisioning is defined.

## Provisioning

PENDING(provision): Create Neon `main` and `dev` databases, the R2 buckets `powermove-objects`, `powermove-tars`, and `powermove-icons`, OAuth apps for Google and GitHub, a Resend sender for `users.trypowermove.com`, and DNS for `cloud.trypowermove.com`. Apply `drizzle/0000_*.sql` to each database before deploying.

Set these Wrangler secrets per environment; never put them in `vars` or the repository:

- `DATABASE_URL`
- `BETTER_AUTH_SECRET` (at least 32 random characters)
- `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`
- `GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET`
- `RESEND_API_KEY`
- `ADMIN_TOKEN`

The OAuth providers are omitted when their IDs are absent so local boot can start. PENDING(provision): Without `RESEND_API_KEY`, the email OTP sender prints the code with a `PENDING(provision)` prefix for local testing. Configure a real sender before exposing email sign-in publicly.

PENDING(P5): The scheduled garbage collection handler only logs a placeholder. Unit P5 implements the state machine.

## Auth schema note

The Better Auth CLI could not run in this offline sandbox (`bunx` could not create its temporary files). `src/db/auth-schema.ts` was written from the installed Better Auth 1.7 core and username schema. The core tables are `user`, `session`, `account`, and `verification`; email OTP and one-time-token use `verification` and do not add tables. Regenerate and compare the schema with the CLI when it is available.
