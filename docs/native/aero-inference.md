# Aero on Powermove inference: billing and inference v2

Aero, Powermove's own harness, runs only on Powermove inference, paid by the person's Powermove plan. No
bring-your-own keys, no custom endpoint. Claude Code and Codex keep their own sign-ins and keys.

The backend is `apps/cloud` (`src/routes/billing.ts`, `stripe-webhook.ts`, `aero.ts`). Everything is under
`{origin}/v2`, with `Authorization: Bearer <Powermove session>` as in v1. `/v1/aero/*` answers 404.

## Plans and credits

| Plan | Price | Credits a month |
|---|---|---|
| Free | $0 | none (new accounts get none) |
| Aerolite | $9.99 | 1,000 |
| Aero | $19.99 | 2,500 |

- Credits are granted when a billing period's invoice is paid and **don't roll over**: only the current period's
  grant and the spends inside the period count. An upgrade mid-period tops the period up to the new plan.
- **1 credit = $0.005 of provider list price**: `credits = ceil(cost_usd * 200)`. Each model has an input, cached
  input, cache write and output price per million tokens (`src/aero/catalogue.ts`). Cached input costs the cached
  rate, so caching saves credits.
- Checkout is Stripe Checkout (Stripe Tax on, billing address required, tax IDs collected). Manage, cancel and change
  plan are the Stripe Customer Portal. Plan changes are prorated.

## Billing

`GET /v2/billing`

```json
{ "plan": {"id": "aerolite", "name": "Aerolite", "priceCents": 999, "interval": "month"},
  "status": "active", "cancelAtPeriodEnd": false,
  "credits": {"included": 1000, "used": 188, "remaining": 812},
  "period": {"start": "2026-11-25T00:00:00.000Z", "end": "2026-12-25T00:00:00.000Z"},
  "daily": [{"date": "2026-11-25", "credits": 40}],
  "plans": [{"id": "aerolite", "name": "Aerolite", "priceCents": 999, "credits": 1000},
            {"id": "aero", "name": "Aero", "priceCents": 1999, "credits": 2500}] }
```

- No plan: `plan: null`, `status: null` (or `"canceled"` after a plan ended), credits all 0, `period: null`,
  `daily: []`. `status` is `active`, `past_due` (a failed renewal: no new credits) or `canceled`.
- `daily` is every UTC day of the current period up to today, zero days included.

`POST /v2/billing/checkout {"plan": "aerolite" | "aero"}` → `{"url"}` (Stripe Checkout); `400 invalid_plan` for any other plan, `409 has_plan` when the
account has a live plan: use the portal. `POST /v2/billing/portal` → `{"url"}`; `409 no_billing_account` before the
first checkout. Open both in the default browser.

Both send the browser back to `{origin}/v2/billing/return?status=success|cancel|portal`, which says "You can return to
Powermove" and opens `powermove://billing/return?status=…`. On that link, read `/v2/billing` again: the plan turns
active when Stripe's webhook arrives, usually before the page loads.

## Models

`GET /v2/aero/models` → `{"data": [{"id", "name", "efforts", "defaultEffort", "context", "default"?}]}`. Ids are
`openai/<model>` and `anthropic/<model>` (GPT-6.2 Sol (the default), GPT-6.2 Astra, GPT-6 Luna, Claude Opus 5.5,
Claude Sonnet 5.5); send the id back as `model`.

## Chat completions

`POST /v2/aero/chat/completions`: OpenAI Chat Completions with `stream: true`, the model id, `reasoning_effort` (one
of the model's `efforts`; omitted means `defaultEffort`), function tools, `tool_choice`, `parallel_tool_calls`,
`max_completion_tokens`. Images only as `data:` URLs. Optional headers:

- `X-Powermove-Request-Id` (8–128 of `A-Za-z0-9._:-`): the request's id. Each id is metered once; a repeat answers
  `409 duplicate_request` and is not run.
- `X-Powermove-Thread` (or the body's `prompt_cache_key`): names the thread for OpenAI's prompt cache. Without it the
  thread's opening (system prompt and first user turn) names it.

The answer is Chat Completions SSE, in the subset fx's `openai-chat-completions` connection parses: one choice at
index 0; `role`, `content`, `reasoning_content`, `reasoning_details` and `tool_calls` deltas; a `finish_reason`
chunk (`tool_calls` exactly when the reply calls tools); a chunk with `choices: []` and `usage` (`prompt_tokens`
including cached, `prompt_tokens_details.cached_tokens` and `cache_write_tokens`,
`completion_tokens_details.reasoning_tokens`); then `data: [DONE]`.

`reasoning_details` items (`{"type": "reasoning.aero", "provider", "text", "metadata"}`) carry the provider's
continuity data (Anthropic's thinking signature, OpenAI's encrypted reasoning). Send them back unchanged on the
assistant message, as fx does, so a tool loop keeps its reasoning.

| Answer | Meaning |
|---|---|
| `200` SSE | The turn |
| `400 invalid_request`, `unknown_model` | A request the backend won't send |
| `401 unauthorized` | Signed out or the session ended |
| `402 credits_exhausted` | No plan, or no credits left this period |
| `409 duplicate_request` | That request id was already used |
| `429 rate_limited` (`Retry-After`) | 60 requests a minute per account |
| `503 inference_unavailable` | The backend has no provider key |
| `data: {"error": {"code": "upstream_error"}}` in the stream | The provider failed mid-turn |

Every error body is `{"error": {"code": "…"}}`; the backend's own text never reaches the client.

After the stream ends, or the client goes away (which stops the provider call), the request's one ledger row holds
the model, tokens in, cached, cache written and out, and the credits. A stream cut off before the provider reported
usage is estimated (four characters a token) and marked so.

## Prompt caching

Always on. Anthropic: cache breakpoints on the system prompt, the last tool definition, the newest message and the
previous request's last message. OpenAI: a stable `prompt_cache_key` per thread, scoped to the account; the Responses
API with `store: false` and encrypted reasoning. Both meter cache reads and writes from the provider's usage.

## Development

`bun run local:billing` in `apps/cloud` runs the whole stack locally (Postgres, the Worker, a fake Stripe with
Checkout and Portal pages, an AI simulator for both providers) and seeds Jude on Free and Mara on Aerolite; see
`apps/cloud/README.md` › Local stack. Point a debug or Sandbox build's development origin at that Worker.
