# Aero on Powermove inference: the client contract

Aero, Powermove's own harness, runs only on Powermove inference: the Aero and Aerolite plans, paid by the person's
Powermove account. It has no bring-your-own keys and no custom endpoint. Claude Code and Codex keep their own sign-ins and
keys (Settings › Providers › Anthropic (Claude Code) and OpenAI (Codex)).

This is what the desktop app sends and reads (`apps/native/crates/agents/src/inference.rs`). The backend doesn't exist
yet; until it does, the app says so honestly (below).

## Where and how

- **Base:** `{origin}/v1/aero`, where `origin` is the API origin of the signed-in Powermove account
  (`https://cloud.trypowermove.com`, the Store's origin; `pm_compat_runner::store_install::ORIGIN`).
- **Auth:** `Authorization: Bearer <session>`, the Powermove account's session (the app's Cloud sign-in, kept in the
  Keychain). Also `X-Powermove-Client: desktop/<version>`. No other credential is ever sent, and the session goes only
  to the Aero base's origin.
- **Protocol:** OpenAI Chat Completions. The backend forwards each request to the Vercel AI Gateway's OpenAI-compatible
  Chat Completions (`https://ai-gateway.vercel.sh/v1/chat/completions`) with its own gateway key, which never reaches
  the app. The client uses fx's `openai-chat-completions` connection through the host transport, not fx's native AI
  Gateway protocol: the host transport (where Rust adds the credential) carries only Chat Completions and Anthropic
  Messages, the backend meters credits and holds the gateway key, and emulate can stand in for it in development.

## `GET /v1/aero/usage`

The plan and its credits. Read when the account changes, after each turn, and when the app starts.

| Answer | Meaning | The app shows |
|---|---|---|
| `200 {"plan": {...}, "credits": {...}, "resetsAt": "..."}` | A plan | Aero's models, and the usage monitor |
| `200 {"plan": null, "offer": {"label", "url"}}` | No plan; a plan is offered | "Aero inference is coming soon." and the offer as an action |
| `200 {"plan": null}` | No plan | "Aero inference is coming soon." |
| `404` | The route isn't there (before the backend ships) | "Aero inference is coming soon." |
| `401`, `403` | The session ended | "Sign in to Powermove to use Aero." |
| other status, unreadable body, a missing field | — | No monitor; "Aero couldn't read your plan from Powermove." |

```json
{
  "plan": { "id": "aerolite", "name": "Aerolite" },
  "credits": { "remaining": 812, "included": 1000 },
  "resetsAt": "2026-12-25T00:00:00Z"
}
```

- `plan.id`, `plan.name`: non-empty strings of at most 64 characters. Aerolite is $9.99 for 1,000 credits a month.
- `credits.remaining`, `credits.included`: unsigned integers.
- `resetsAt`: an RFC 3339 time in UTC. The app shows its calendar day.
- `plan` is required: no plan is `plan: null`, never a missing field.
- `offer` (only with `plan: null`): `label` at most 40 plain characters (no control or bidi characters), `url` an https
  page on `trypowermove.com` or one of its subdomains, with no userinfo and no other port. Any other offer, or one that
  repeats the credential, isn't shown. Only its canonical form is opened, natively; the panel gets the label alone.

The usage monitor, at the bottom of Aero's picker card: **Credits** `812 of 1,000 left`, **Plan** `Aerolite`,
**Resets** `Dec 25`. It shows nothing until the backend answers.

## `GET /v1/aero/models`

The curated catalogue (Opus, GPT, Gemini and others; tens, not hundreds), read only with a plan. An OpenAI-shaped list:

```json
{ "data": [
  { "id": "anthropic/claude-opus-5.5", "name": "Claude Opus 5.5",
    "efforts": ["low", "medium", "high", "xhigh", "max"], "defaultEffort": "high", "default": true },
  { "id": "openai/gpt-6-sol", "name": "GPT-6 Sol", "efforts": ["none", "low", "medium", "high", "xhigh"] },
  { "id": "google/gemini-3-pro" }
] }
```

- `id` (required): the gateway's model id (`provider/model`), at most 128 characters of `A-Za-z0-9._:/[]-`; the app
  sends it back as `model`. Entries with an invalid id are skipped; a list with none is an error. The app keeps it as
  `pm/<id>`: a thread saved before Aero ran only on Powermove (an OpenAI-key, Anthropic-key, ChatGPT or Custom model)
  names no such id, so it sends nothing, queued or resumed, until the person picks a model from this catalogue.
- `name`: the label (defaults to the id). `efforts`: the levels the picker offers, in order (lowercase words).
  `defaultEffort`: one of them. `default`: the model chosen when none is (else the first). `context`: the context
  window in tokens (the card shows it). The picker marks each row by the id's provider (Claude's logo for
  `anthropic/`, OpenAI's blossom for `openai/`, a neutral sparkle for the rest).

## `POST /v1/aero/chat/completions`

OpenAI Chat Completions with `stream: true`, the catalogue `id` as `model`, the picked level as `reasoning_effort`
(omitted for no level), function tools, and no `tool_choice`.

| Answer | The app shows |
|---|---|
| `200` SSE (`data:` chunks, `data: [DONE]`) | The turn |
| `401` | "Your Powermove session ended. Sign in to Powermove again to use Aero." |
| `402 {"error": {"code": "credits_exhausted"}}` | "You're out of Aero credits. They come back when your plan resets." |
| `429` | The rate-limit words |
| an `error` event in the stream | Powermove's own words for its category, never the backend's text |

The backend's own error text never reaches the panel, the trace or the session's files. A turn is fenced to the
account it started on: a sign-out or another account stops its open stream, discards that reply and runs no more of
its tools.

## Before the backend exists

- Signed out: Aero shows **Sign in to Powermove** (the picker, the pill, Settings › Providers, onboarding).
- Signed in, no plan (today `/usage` answers 404): **Aero inference is coming soon**, or the offer when there is one.

## Development

Debug builds with the development switch on (`PM_AERO_DEV=1` or the profile's `aero-dev` file) read
`PM_AERO_ENDPOINT=http://127.0.0.1:<port>/<path>`: Aero's base, for emulate or the test fakes
(`crates/agents/tests/fakes/fake_backends.py` serves `/aero/v1/aero/*`). It is sent a development credential
(`pm-aero-development`), never the person's session; Aero's real traffic never goes through the development fakes
that reroute other providers (`PM_AGENT_FAKES`). The fake model (`PM_AERO_FAKE_MODEL=1`) is listed and runs in debug
builds only. Release builds never read the endpoint
(`inference::tests::a_release_build_has_no_test_endpoint`, and in `cargo test --release`
`this_release_build_sends_aero_only_to_the_account_origin`). A development review account
(`PM_NATIVE_REVIEW_CLOUD=1`) has no inference origin of its own, so it needs the test endpoint.
