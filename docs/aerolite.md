# Aerolite

Aerolite is Powermove's first paid plan ($9/month). Subscribers get Claude and
OpenAI models built in (each with its own allowance), about 1 GB of cloud
storage, and one read-only share link per week. Aero ($19/month) is modelled
in the plan table but not sold yet.

## Where things live

| Piece | Code |
| --- | --- |
| Tiers, entitlements, model prices, tunables | `apps/cloud/src/billing/plans.ts` |
| Quota decisions (models, shares, storage), pure | `apps/cloud/src/billing/quotas.ts` |
| Plan, usage and storage reads | `apps/cloud/src/billing/account.ts` |
| Model proxy `/v1/ai/*` | `apps/cloud/src/routes/ai.ts`, `src/billing/meter.ts` |
| Billing `/v1/billing/*` (Stripe, test mode only) | `apps/cloud/src/routes/billing.ts`, `src/billing/stripe.ts` |
| Share links `/v1/shares`, public pages `/s/:id` | `apps/cloud/src/routes/shares.ts`, `src/share-page.ts` |
| Migration | `apps/cloud/drizzle/0009_aerolite.sql` |
| Desktop: hosted runner, cloud calls | `apps/desktop/src/main/compatible-provider.ts` (`hosted`), `src/main/cloud/aerolite.ts` |
| Desktop: UI | `src/renderer/src/cloud/{plan.ts,UpgradeSheet.svelte}`, `src/renderer/src/settings/PlanSettings.svelte`, the agent's `ConnectionGate.svelte` |
| Local end-to-end stack, no accounts | `apps/cloud/scripts/aerolite-dev.ts` |

Every entitlement check asks `entitlementsFor(tier)`; nothing compares tier
names. Adding Aero means filling in its row (already present) and adding it to
`PURCHASABLE`.

## How the built-in models work

The desktop's existing "API / local model" runner speaks OpenAI-style
`/chat/completions`. Aerolite is a second instance of that runner whose base
URL is `<cloud>/v1/ai` and whose key is the person's Powermove session token.
The proxy:

1. resolves the plan and refuses models the plan doesn't include (403);
2. sums this period's and today's spend for that provider and refuses at the
   cap (429, `X-Powermove-Limit-Resets`);
3. caps `max_tokens` at 16k, forces `stream_options.include_usage`, swaps in
   our provider key and forwards to Anthropic's or OpenAI's chat-completions
   endpoint;
4. meters from its own copy of the stream (a client that stops reading early
   or presses Stop is still billed for what was generated) and records one
   `usage_events` row with the list-price cost in micro-dollars.

Limits are in dollars of list-price inference, so they stay fair across
models (Haiku lasts 20× longer than Sonnet). The app only ever shows
percentages, never dollars.

## Limits and the cost math

Defaults in `CONFIG` (`plans.ts`):

| | Claude / month | Claude / day | OpenAI / month | OpenAI / day | Storage |
| --- | --- | --- | --- | --- | --- |
| Aerolite | $2.50 | $0.50 | $1.50 | $0.30 | 1 GB |
| Aero (later) | $7.00 | $1.40 | $4.00 | $0.80 | 50 GB |

Per Aerolite subscriber, per month:

| | |
| --- | --- |
| Price | $9.00 |
| Stripe (2.9% + $0.30) | −$0.56 |
| Inference, if both allowances are fully used | −$4.00 |
| R2 storage, 1 GB full | −$0.02 |
| Workers + Neon, rough | −$0.10 |
| **Worst-case margin** | **≈ $4.30 (48%)** |

Subscription products typically see a minority of users near the cap; at 35%
average use inference is ≈ $1.40 and the margin ≈ $6.90 (77%). The daily caps
(20% of the month) stop one long session from draining a month and bound
abuse from a stolen session.

What $2.50 of Claude buys: an agent turn sends roughly 12k input tokens
(instructions, tools, project context) and gets about 800 back. On Sonnet 5.5
($2 / $10 per M) that's about $0.03 per turn uncached, so roughly 80 Sonnet
turns a month, or about 1,600 on Haiku 5.5. $1.50 of OpenAI is about 50
GPT-6.2 Sol turns. That is enough for regular use, not for all-day agent
sessions; those belong in Aero or the bring-your-own-agent path. Prompt
caching would roughly double the turns (see open decisions).

## Share links

Free: one link ever. Aerolite: one per rolling 7 days. Aero: unlimited and
(later) interactive. Revoked links still count, so deleting one never refunds
quota. A link stores facts about the project (size, frame rate, length,
compositions, layer names and timing) plus a JPEG poster of the current
frame. It never stores the project file or media. The page is static HTML
with a strict CSP and `noindex`. Share bytes count against cloud storage.

## Going live: what's needed

Nothing here charges money today: `stripeKey()` refuses any key that isn't
`sk_test_`/`rk_test_`, and webhooks with `livemode: true` are ignored.
Turning on real billing is a deliberate one-line change in `stripe.ts`.

Worker secrets (`wrangler secret put`, never in `wrangler.jsonc`):

| Variable | What |
| --- | --- |
| `AEROLITE_ANTHROPIC_API_KEY` | Anthropic API key for hosted Claude (its own workspace, with a spend limit) |
| `AEROLITE_OPENAI_API_KEY` | OpenAI API key for hosted models (its own project, with a spend limit) |
| `STRIPE_SECRET_KEY` | `sk_test_…` (or a restricted `rk_test_…`) |
| `STRIPE_WEBHOOK_SECRET` | `whsec_…` from the webhook endpoint below |
| `STRIPE_PRICE_AEROLITE` | Price id of the $9/month recurring price |
| `STRIPE_PRICE_AERO` | Later, for Aero |
| `AEROLITE_ANTHROPIC_BASE_URL`, `AEROLITE_OPENAI_BASE_URL` | Optional overrides (gateway, regional endpoint) |

Accounts and setup:

1. Stripe (test mode): product "Powermove Aerolite", $9/month price; customer
   portal enabled (cancel, update card); webhook endpoint
   `https://cloud.trypowermove.com/v1/billing/webhook` for
   `checkout.session.completed` and `customer.subscription.created|updated|deleted`.
2. Anthropic and OpenAI organisations with billing, a dedicated key each, and
   provider-side monthly spend limits as a backstop.
3. Cloudflare: R2 bucket `powermove-shares` (binding `SHARES`, already in
   `wrangler.jsonc`).
4. Neon: apply migration `0009_aerolite.sql`.

## Trying it locally

```sh
cd apps/cloud && bun scripts/aerolite-dev.ts          # real app, PGlite, simulated Stripe + models
cd apps/desktop && POWERMOVE_REGISTRY_URL=http://localhost:8787 bun run dev
```

Sign in with any `@example.com` address. The code is printed by the dev
server and also served at `/__dev/otp?email=…`. Upgrade from the agent panel. Checkout
opens a simulated URL; `curl -X POST localhost:8787/__dev/complete-checkout`
plays the Stripe webhook. Export `AEROLITE_ANTHROPIC_API_KEY` /
`AEROLITE_OPENAI_API_KEY` before starting the dev server to hit the real
providers.

## Open decisions

- **Storage size.** 1 GB is a placeholder (`CONFIG.aeroliteStorageBytes`).
  Today only share pages use it; free accounts get 25 MB so their one share fits.
- **Which models.** Aerolite: Claude Sonnet 5.5 + Haiku 5.5, GPT-6.2 Sol +
  GPT-6 Luna. Opus 5.5 is Aero-only. GPT-6.2 Sol is priced at the published
  6.1 Sol rate ($2 / $10) until OpenAI confirms; check before launch.
- **Prompt caching / native Messages API.** The proxy forwards OpenAI-style
  chat completions, including to Anthropic's compatibility endpoint. A native
  `/v1/messages` route would give Claude prompt caching (≈2× more turns for
  the same budget), better tool streaming and extended thinking. Recommended
  before or soon after launch.
- **Limits.** The table above is a starting point; revisit with real usage
  from `usage_events`.
- **Share quota semantics.** Rolling 7 days vs calendar week; whether
  revoking should refund.
- **Running out mid-month.** Today the model is refused until reset. Options:
  fall back to Haiku/Luna, or sell top-ups.
- **Free trial / promo codes.** Checkout allows promotion codes; no trial is
  configured.

## Native (GPUI) app

The server side is client-agnostic; the Electron UI is not. To build Aerolite
in the native app:

**Auth.** The existing Powermove session token, as `Authorization: Bearer`.

**Plan & usage.** `GET /v1/billing/plan` returns:

```jsonc
{
  "plan": { "tier": "free|aerolite|aero", "label": "Aerolite", "priceCents": 900, "storageBytes": 1073741824,
            "shares": { "kind": "lifetime|weekly", "count": 1 } /* or { "kind": "unlimited" } */,
            "interactiveShares": false,
            "models": [{ "id": "claude-sonnet-5-5", "provider": "anthropic", "label": "Claude Sonnet 5.5" }] },
  "subscription": { "status": "active", "renewsAt": "2026-12-18T…Z", "cancelAtPeriodEnd": false } /* or null */,
  "usage": { "anthropic": { "period": 0.12, "day": 0.4, "periodResetsAt": "…", "dayResetsAt": "…" },
             "openai": { … } } /* null on free */,
  "shares": { "used": 3, "usedThisWeek": 1, "canCreate": false, "nextAvailableAt": "…" },
  "storage": { "usedBytes": 2048, "limitBytes": 1073741824 },
  "upgrades": [ /* plan summaries the person can buy */ ],
  "billingConfigured": true
}
```

The desktop wire schema is `apps/desktop/src/shared/aerolite.ts` (`PlanDto`).

**Billing.** `POST /v1/billing/checkout {"tier":"aerolite"}` and
`POST /v1/billing/portal` return `{ url }`. Open the URL only if its host is
`stripe.com` or a subdomain. Re-fetch the plan when the app regains focus.

**Models.** `GET /v1/ai/models` (OpenAI list format) and
`POST /v1/ai/chat/completions` (OpenAI format, `stream: true`). Errors are
`{ error, detail }`: show `detail` as written. 403 means the model isn't in
the plan; 429 with `quota_exceeded` means the allowance is used, and
`X-Powermove-Limit-Resets` says when it refills.

**Shares.** `POST /v1/shares { snapshot, poster? }` → `{ id, url, interactive }`;
`GET /v1/shares`; `DELETE /v1/shares/:id`. The snapshot schema is
`CreateShare` in `apps/cloud/src/routes/shares.ts`. The poster is a base64
PNG or JPEG, 2 MB at most.

**UI to rebuild.** (Screens and copy are in the Electron implementation.)
1. Agent connection gate: an "Aerolite" provider. Signed out: "Sign in to
   Powermove". Free: "Get Aerolite", which opens the paywall. Paid: the
   plan's models appear in the model picker.
2. Paywall sheet: what's included, "Upgrade for $9/month", then "Finish
   checkout in your browser" with Check Again, then "You're on Aerolite" with
   Use Aerolite Models.
3. Settings › Plan & Usage: plan row (Upgrade / Manage Billing), one meter per
   provider showing percent used and refill date, share links with Copy and
   Remove, cloud storage meter.
