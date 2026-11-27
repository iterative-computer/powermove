# Aero inference API

Aero, Powermove's own agent, runs only on Powermove inference and is paid for by
the person's Powermove plan. There are no bring-your-own keys and no custom
endpoint. Claude Code and Codex keep their own sign-ins and keys.

The backend is `apps/cloud`. Everything is under `{origin}/v2`, with
`Authorization: Bearer <Powermove session>`. `/v1/aero/*` answers 404.

## Plans and credits

| Plan | Credits a month |
|---|---|
| Free | none |
| Aerolite | 1,000 |
| Aero | 2,500 |

- Credits are granted when a billing period's invoice is paid and reset every
  month: they don't roll over. An upgrade mid-period tops the period up to the
  new plan.
- Each request spends credits according to the model and the tokens it uses.
  Cached input uses fewer credits than fresh input.
- Checkout is Stripe Checkout; managing, cancelling and changing plan happen in
  the Stripe Customer Portal. Plan changes are prorated.

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

- No plan: `plan: null`, `status: null` (or `"canceled"` after a plan ended),
  credits all 0, `period: null`, `daily: []`.
- `status` is `active`, `past_due` (a failed renewal: no new credits) or
  `canceled`.
- `daily` is every UTC day of the current period up to today, zero days included.

`POST /v2/billing/checkout {"plan": "aerolite" | "aero"}` → `{"url"}` (Stripe
Checkout). `400 invalid_plan` for any other plan; `409 has_plan` when the account
already has a live plan (use the portal).

`POST /v2/billing/portal` → `{"url"}`; `409 no_billing_account` before the first
checkout. Open both URLs in the default browser.

Both send the browser back to `{origin}/v2/billing/return?status=success|cancel|portal`,
which opens `powermove://billing/return?status=…`. On that link, read
`/v2/billing` again: the plan turns active when Stripe's webhook arrives, usually
before the page loads.

## Models

`GET /v2/aero/models` →
`{"data": [{"id", "name", "efforts", "defaultEffort", "context", "default"?}]}`.
Ids look like `openai/<model>` or `anthropic/<model>`; send the id back as
`model`.

## Chat completions

`POST /v2/aero/chat/completions`: OpenAI Chat Completions with `stream: true`, the
model id, `reasoning_effort` (one of the model's `efforts`; omitted means
`defaultEffort`), function tools, `tool_choice`, `parallel_tool_calls` and
`max_completion_tokens`. Images only as `data:` URLs. Optional headers:

- `X-Powermove-Request-Id` (8–128 of `A-Za-z0-9._:-`): the request's id. Each id
  is metered once; a repeat answers `409 duplicate_request` and is not run.
- `X-Powermove-Thread` (or the body's `prompt_cache_key`): names the thread for
  prompt caching. Without it, the thread's opening (system prompt and first user
  turn) names it.

The answer is Chat Completions SSE: one choice at index 0; `role`, `content`,
`reasoning_content`, `reasoning_details` and `tool_calls` deltas; a
`finish_reason` chunk (`tool_calls` exactly when the reply calls tools); a chunk
with `choices: []` and `usage` (`prompt_tokens` including cached,
`prompt_tokens_details.cached_tokens` and `cache_write_tokens`,
`completion_tokens_details.reasoning_tokens`); then `data: [DONE]`.

`reasoning_details` items (`{"type": "reasoning.aero", "provider", "text", "metadata"}`)
carry the provider's reasoning continuity data. Send them back unchanged on the
assistant message so a tool loop keeps its reasoning.

| Answer | Meaning |
|---|---|
| `200` SSE | The turn |
| `400 invalid_request`, `unknown_model` | A request the backend won't send |
| `401 unauthorized` | Signed out, or the session ended |
| `402 credits_exhausted` | No plan, or no credits left this period |
| `409 duplicate_request` | That request id was already used |
| `429 rate_limited` (`Retry-After`) | Too many requests for this account |
| `503 inference_unavailable` | Inference is not configured on this backend |
| `data: {"error": {"code": "upstream_error"}}` in the stream | The provider failed mid-turn |

Every error body is `{"error": {"code": "…"}}`; the backend's own text never
reaches the client.

Credits are metered when the stream ends, or when the client goes away (which
stops the provider call). A stream cut off before the provider reported usage is
estimated.

## Prompt caching

Always on, for both providers, scoped to the account and keyed by thread.

## Development

`bun run local:billing` in `apps/cloud` runs the whole stack locally (Postgres,
the Worker, a fake Stripe with Checkout and Portal pages, and an AI simulator for
both providers) and seeds one Free and one Aerolite account; see
[`apps/cloud/README.md`](../../apps/cloud/README.md) › Local stack. Point a debug
build's development origin at that Worker.
