//! Aero on Powermove inference, and nothing else: the Powermove account's
//! session pays for every request, on the account's own API origin
//! (`docs/native/aero-inference.md` is the contract). Three calls under
//! `{origin}/v1/aero`: `GET /usage` (the plan and its credits), `GET /models`
//! (the curated catalogue) and `POST /chat/completions` (OpenAI Chat
//! Completions, streamed; the backend forwards it to the Vercel AI Gateway
//! with its own key). The session stays here and in the app's Keychain; the
//! core gets a provider description without it, and the fetch edge adds it
//! for the Aero origin only.
//!
//! Development builds with the development switch on may point Aero at a
//! local stand-in (`PM_AERO_ENDPOINT=http://127.0.0.1:<port>/<path>`, emulate
//! or the test fakes). Release builds never read it.
use crate::account::Model;
use crate::aero::Live;
use crate::edge::{Idle, Loopback, Net, NetRequest, STREAM_IDLE, fetch};
use crate::launch::Environment;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// Where Aero's calls live on the account's API origin.
pub const PATH: &str = "/v1/aero";
/// The development switch's stand-in for Aero's API (debug builds only).
pub const TEST_ENDPOINT: &str = "PM_AERO_ENDPOINT";
/// The development switch's demo model (`PM_AERO_FAKE_MODEL=1` lists it).
pub const FAKE_MODEL: &str = "fake/model";
/// How long a usage or catalogue read may stay silent.
const READ_IDLE: Duration = Duration::from_secs(15);
pub const SIGN_IN: &str = "Sign in to Powermove to use Aero.";
const SIGN_IN_AGAIN: &str = "Your Powermove session ended. Sign in to Powermove again to use Aero.";

/// The signed-in Powermove account (the app's Cloud session).
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    /// The API origin the session belongs to (`https://cloud.trypowermove.com`).
    pub origin: String,
    pub token: String,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
/// The app's Powermove account, as Aero reads it: the session, read each
/// time it is asked for (a sign-out or another account is seen at once).
pub trait PowermoveAccount: Send + Sync + std::fmt::Debug {
    fn session(&self) -> Option<Session>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub name: String,
}
/// The plan's credits (Aerolite: 1,000 a month) and when they come back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub plan: Plan,
    pub remaining: u64,
    pub included: u64,
    /// `resetsAt`'s calendar day (UTC).
    pub resets: (i32, u32, u32),
}
/// A plan the backend offers someone who has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub label: String,
    pub url: String,
}
/// What `GET /usage` said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageAnswer {
    SignedOut,
    /// Signed in, no plan: `offer` when the backend has one to show, none
    /// while the backend isn't there yet (404).
    NoPlan(Option<Offer>),
    Plan(Usage),
}
/// Where Aero stands for this person, as the picker shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Standing {
    #[default]
    Checking,
    SignedOut,
    NoPlan(Option<Offer>),
    Ready {
        usage: Usage,
        models: Vec<Model>,
    },
    /// Powermove couldn't be reached or answered something unreadable.
    Unreachable(String),
}

/// The test endpoint, when `development` (a debug build with the switch on)
/// and it names plain HTTP on 127.0.0.1 with a port, nothing else.
pub fn test_endpoint(vars: &BTreeMap<String, String>, development: bool) -> Option<String> {
    let raw = vars.get(TEST_ENDPOINT).filter(|_| development)?;
    let url = url::Url::parse(raw).ok()?;
    (url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| raw.trim_end_matches('/').to_owned())
}

/// Aero's API base for `session`, and the network that reaches it.
fn route(
    env: &Environment,
    session: &Session,
    idle: Duration,
) -> Result<(String, Arc<dyn Net>), String> {
    let development = cfg!(debug_assertions) && env.providers();
    if let Some(base) = test_endpoint(&env.vars, development) {
        return Ok((
            base,
            Arc::new(Idle {
                inner: Arc::new(Loopback),
                after: idle,
            }),
        ));
    }
    let origin = session.origin.trim_end_matches('/');
    if !origin.starts_with("https://") {
        return Err("This development account has no Aero endpoint.".into());
    }
    let net = env
        .net
        .clone()
        .ok_or("Powermove's network client is unavailable.")?;
    Ok((
        format!("{origin}{PATH}"),
        Arc::new(Idle {
            inner: net,
            after: idle,
        }),
    ))
}
fn client(env: &Environment) -> (String, String) {
    (
        "x-powermove-client".into(),
        format!("desktop/{}", env.app_version),
    )
}

/// Reads `GET /usage`, then (with a plan) `GET /models`. Blocks: call it on
/// a worker.
pub fn standing(env: &Environment) -> Standing {
    let Some(session) = env.powermove.as_ref().and_then(|a| a.session()) else {
        return Standing::SignedOut;
    };
    let (base, net) = match route(env, &session, READ_IDLE) {
        Ok(found) => found,
        Err(reason) => return Standing::Unreachable(reason),
    };
    let headers = [
        (
            "authorization".to_owned(),
            format!("Bearer {}", session.token),
        ),
        client(env),
    ];
    let get = |path: &str| {
        let url = format!("{base}{path}");
        fetch(
            net.as_ref(),
            &NetRequest {
                method: "GET",
                url: &url,
                headers: &headers,
                body: b"",
            },
        )
    };
    let unreachable = || Standing::Unreachable("Aero couldn’t reach Powermove. Try again.".into());
    let Ok((status, body)) = get("/usage") else {
        return unreachable();
    };
    match parse_usage(status, &body) {
        Ok(UsageAnswer::SignedOut) => Standing::SignedOut,
        Ok(UsageAnswer::NoPlan(offer)) => Standing::NoPlan(offer),
        Ok(UsageAnswer::Plan(usage)) => match get("/models") {
            Ok((200, body)) => match parse_models(&body) {
                Ok(models) => Standing::Ready { usage, models },
                Err(_) => {
                    Standing::Unreachable("Aero couldn’t read its models from Powermove.".into())
                }
            },
            Ok((401 | 403, _)) => Standing::SignedOut,
            _ => unreachable(),
        },
        Err(_) => Standing::Unreachable("Aero couldn’t read your plan from Powermove.".into()),
    }
}

/// `GET /usage`'s answer (the contract's usage section).
pub fn parse_usage(status: u16, body: &[u8]) -> Result<UsageAnswer, String> {
    match status {
        200 => {}
        401 | 403 => return Ok(UsageAnswer::SignedOut),
        // The route isn't there: the backend hasn't shipped.
        404 => return Ok(UsageAnswer::NoPlan(None)),
        _ => return Err(format!("usage answered HTTP {status}")),
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| "usage is not JSON")?;
    if !value.is_object() {
        return Err("usage is not an object".into());
    }
    let plan = &value["plan"];
    if plan.is_null() {
        return Ok(UsageAnswer::NoPlan(offer(&value["offer"])));
    }
    let text = |v: &Value, name: &str| {
        v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= 64)
            .map(str::to_owned)
            .ok_or(format!("usage is missing {name}"))
    };
    let count = |name: &str| {
        value["credits"][name]
            .as_u64()
            .ok_or(format!("usage is missing credits.{name}"))
    };
    let resets = value["resetsAt"]
        .as_str()
        .and_then(calendar_day)
        .ok_or("usage is missing resetsAt")?;
    Ok(UsageAnswer::Plan(Usage {
        plan: Plan {
            id: text(&plan["id"], "plan.id")?,
            name: text(&plan["name"], "plan.name")?,
        },
        remaining: count("remaining")?,
        included: count("included")?,
        resets,
    }))
}
/// An offer to show, when it is a short label and a Powermove https page.
fn offer(value: &Value) -> Option<Offer> {
    let label = value["label"].as_str()?.trim();
    let url = value["url"].as_str()?;
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let ours = host == "trypowermove.com" || host.ends_with(".trypowermove.com");
    (parsed.scheme() == "https" && ours && !label.is_empty() && label.chars().count() <= 40).then(
        || Offer {
            label: label.to_owned(),
            url: url.to_owned(),
        },
    )
}
/// `YYYY-MM-DD` at the start of an RFC 3339 time.
fn calendar_day(iso: &str) -> Option<(i32, u32, u32)> {
    let date = iso.get(..10)?;
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day) && iso.get(10..11) == Some("T"))
        .then_some((year, month, day))
}

/// `GET /models`'s answer: an OpenAI-shaped list (`data[].id`), each entry
/// optionally with Powermove's `name`, `efforts` and `defaultEffort`.
pub fn parse_models(body: &[u8]) -> Result<Vec<Model>, String> {
    let value: Value = serde_json::from_slice(body).map_err(|_| "models is not JSON")?;
    let level = |v: &Value| {
        v.as_str()
            .filter(|l| !l.is_empty() && l.len() <= 16 && l.bytes().all(|b| b.is_ascii_lowercase()))
            .map(str::to_owned)
    };
    let mut models: Vec<Model> = value["data"]
        .as_array()
        .ok_or("models has no data")?
        .iter()
        .filter_map(|m| {
            let id = m["id"]
                .as_str()
                .filter(|id| crate::launch::is_valid_model_id(id))?;
            let efforts: Vec<String> = m["efforts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(level)
                .collect();
            Some(Model {
                id: id.into(),
                label: m["name"]
                    .as_str()
                    .map(str::trim)
                    .filter(|n| !n.is_empty() && n.chars().count() <= 64)
                    .unwrap_or(id)
                    .to_owned(),
                default_effort: level(&m["defaultEffort"]).filter(|e| efforts.contains(e)),
                efforts,
                default: m["default"] == true,
                aliases: vec![],
            })
        })
        .collect();
    if models.is_empty() {
        return Err("models lists none".into());
    }
    if !models.iter().any(|m| m.default) {
        models[0].default = true;
    }
    Ok(models)
}

impl Usage {
    /// The usage monitor's lines, as the picker card shows them.
    pub fn facts(&self) -> Vec<(String, String)> {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let (_, month, day) = self.resets;
        vec![
            (
                "Credits".into(),
                format!(
                    "{} of {} left",
                    grouped(self.remaining),
                    grouped(self.included)
                ),
            ),
            ("Plan".into(), self.plan.name.clone()),
            (
                "Resets".into(),
                format!("{} {day}", MONTHS[month as usize - 1]),
            ),
        ]
    }
}
/// `1000` → `1,000`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The model edge for catalogue model `model` at `effort`, paid by the
/// Powermove account signed in now, or why it can't run.
pub fn live(env: &Environment, model: &str, effort: &str) -> Result<Live, String> {
    if !crate::launch::is_valid_model_id(model) {
        return Err("Choose a model for Aero.".into());
    }
    let account = env.powermove.clone().ok_or(SIGN_IN)?;
    let session = account.session().ok_or(SIGN_IN)?;
    let (base, net) = route(env, &session, STREAM_IDLE)?;
    let origin = pm_aero_core::origin_of(&base).ok_or("Aero's address is invalid.")?;
    let started = token_id(&session.token);
    let client = client(env);
    Ok(Live {
        provider: "aero".into(),
        configured: configured(&base, model, effort),
        model: model.into(),
        net,
        // Read for every request: a sign-out or another account is never
        // carried on with.
        credentials: Arc::new(move |rejected| {
            let session = account.session().ok_or(SIGN_IN)?;
            let id = token_id(&session.token);
            if id != started {
                return Err(
                    "The Powermove account changed during this turn. Send it again.".into(),
                );
            }
            // The backend refused this session: there is no other to try.
            if rejected.is_some_and(|r| token_id(r) == id) {
                return Err(SIGN_IN_AGAIN.into());
            }
            Ok((
                origin.clone(),
                vec![
                    ("authorization".into(), format!("Bearer {}", session.token)),
                    client.clone(),
                ],
            ))
        }),
    })
}

/// The core's connection to Aero's Chat Completions: the picked level as
/// `reasoning_effort` (none sent for no level). Tool choice is left to the
/// model: the catalogue's Claude models refuse a forced one.
fn configured(base: &str, model: &str, effort: &str) -> String {
    let mut entry = json!({"supports_tool_use": true, "supports_vision": true});
    if !effort.is_empty() {
        entry["reasoning_efforts"] = json!({effort: {"reasoning_effort": effort}});
    }
    json!({"aero": {"protocol":"openai-chat-completions","base_url":base,"auth":{"type":"none"},
        "tool_choice_mode":"omit","model_metadata":{model: entry}}})
    .to_string()
}

/// A session's identity for comparing, never the token itself.
fn token_id(token: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("session:{:x}", Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(endpoint: &str) -> BTreeMap<String, String> {
        BTreeMap::from([(TEST_ENDPOINT.to_owned(), endpoint.to_owned())])
    }

    #[test]
    fn the_usage_contract_reads_a_plan_with_its_credits_and_reset_day() {
        let body = br#"{"plan":{"id":"aerolite","name":"Aerolite"},
            "credits":{"remaining":812,"included":1000},"resetsAt":"2026-12-25T00:00:00Z"}"#;
        let Ok(UsageAnswer::Plan(usage)) = parse_usage(200, body) else {
            panic!("a plan")
        };
        assert_eq!(usage.plan.name, "Aerolite");
        assert_eq!((usage.remaining, usage.included), (812, 1000));
        assert_eq!(
            usage.facts(),
            [
                ("Credits".to_owned(), "812 of 1,000 left".to_owned()),
                ("Plan".to_owned(), "Aerolite".to_owned()),
                ("Resets".to_owned(), "Dec 25".to_owned()),
            ]
        );
    }

    #[test]
    fn the_usage_contract_refuses_an_answer_missing_a_field() {
        for (body, missing) in [
            (
                r#"{"plan":{"id":"aerolite","name":"Aerolite"},"credits":{"included":1000},"resetsAt":"2026-12-25T00:00:00Z"}"#,
                "credits.remaining",
            ),
            (
                r#"{"plan":{"id":"aerolite"},"credits":{"remaining":1,"included":1000},"resetsAt":"2026-12-25T00:00:00Z"}"#,
                "plan.name",
            ),
            (
                r#"{"plan":{"id":"aerolite","name":"Aerolite"},"credits":{"remaining":1,"included":1000}}"#,
                "resetsAt",
            ),
            (
                r#"{"plan":{"id":"aerolite","name":"Aerolite"},"credits":{"remaining":1,"included":1000},"resetsAt":"soon"}"#,
                "resetsAt",
            ),
        ] {
            let error = parse_usage(200, body.as_bytes()).unwrap_err();
            assert!(error.contains(missing), "{body}: {error}");
        }
        assert!(parse_usage(200, b"not json").is_err());
        assert!(parse_usage(200, b"[]").is_err());
    }

    #[test]
    fn the_usage_contract_reads_backend_errors_and_the_states_before_a_plan() {
        assert!(parse_usage(500, br#"{"error":"internal"}"#).is_err());
        assert!(parse_usage(429, b"").is_err());
        assert_eq!(parse_usage(401, b""), Ok(UsageAnswer::SignedOut));
        // Before the backend ships its route: no plan, nothing offered.
        assert_eq!(parse_usage(404, b"<html>"), Ok(UsageAnswer::NoPlan(None)));
        assert_eq!(
            parse_usage(200, br#"{"plan":null,"offer":{"label":"Get Aerolite","url":"https://trypowermove.com/aero"}}"#),
            Ok(UsageAnswer::NoPlan(Some(Offer {
                label: "Get Aerolite".into(),
                url: "https://trypowermove.com/aero".into()
            })))
        );
        // An offer anywhere but a Powermove https page is not shown.
        for url in [
            "http://trypowermove.com/aero",
            "https://trypowermove.com.example/aero",
        ] {
            let body = json!({"plan":null,"offer":{"label":"Get Aerolite","url":url}}).to_string();
            assert_eq!(
                parse_usage(200, body.as_bytes()),
                Ok(UsageAnswer::NoPlan(None)),
                "{url}"
            );
        }
    }

    #[test]
    fn the_catalogue_lists_its_models_with_their_levels() {
        let models = parse_models(br#"{"data":[
            {"id":"anthropic/claude-opus-5.5","name":"Claude Opus 5.5","efforts":["low","medium","high","xhigh","max"],"defaultEffort":"high"},
            {"id":"openai/gpt-6-sol","name":"GPT-6 Sol","efforts":["low","high"],"defaultEffort":"ultra"},
            {"id":"bad id"},
            {"id":"google/gemini-3-pro"}]}"#)
        .unwrap();
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "anthropic/claude-opus-5.5",
                "openai/gpt-6-sol",
                "google/gemini-3-pro"
            ]
        );
        assert!(models[0].default);
        assert_eq!(models[0].default_effort.as_deref(), Some("high"));
        assert_eq!(
            models[1].default_effort, None,
            "a level the model doesn't list"
        );
        assert_eq!(models[2].label, "google/gemini-3-pro");
        assert!(parse_models(br#"{"data":[]}"#).is_err());
    }

    /// The test endpoint is a development build's only, loopback only.
    #[test]
    fn a_release_build_has_no_test_endpoint() {
        let local = vars("http://127.0.0.1:4014/v1/aero");
        assert_eq!(test_endpoint(&local, false), None, "release config");
        assert_eq!(
            test_endpoint(&local, true).as_deref(),
            Some("http://127.0.0.1:4014/v1/aero")
        );
        for elsewhere in [
            "https://127.0.0.1:4014/v1",
            "http://localhost:4014/v1",
            "http://127.0.0.1/v1",
            "http://evil.example:4014/v1",
            "http://user:pw@127.0.0.1:4014/v1",
        ] {
            assert_eq!(test_endpoint(&vars(elsewhere), true), None, "{elsewhere}");
        }
    }

    /// Run in a release test build (`cargo test --release`): with the switch on
    /// and the endpoint set, Aero still goes only to the account's origin.
    #[cfg(not(debug_assertions))]
    #[test]
    fn this_release_build_sends_aero_only_to_the_account_origin() {
        #[derive(Debug)]
        struct Never;
        impl Net for Never {
            fn stream(
                &self,
                _: &NetRequest<'_>,
                _: &std::sync::atomic::AtomicBool,
                _: &mut dyn FnMut(crate::edge::Chunk<'_>) -> Result<(), String>,
            ) -> Result<(), String> {
                Err("unused".into())
            }
        }
        let env = Environment {
            vars: vars("http://127.0.0.1:4014/v1/aero"),
            net: Some(Arc::new(Never)),
            aero: Some(crate::aero::Launcher(Arc::new(|_, _, _| {
                Err("unused".into())
            }))),
            ..Environment::default()
        };
        let session = Session {
            origin: "https://cloud.trypowermove.com".into(),
            token: "t".into(),
        };
        let (base, _) = route(&env, &session, READ_IDLE).unwrap();
        assert_eq!(base, "https://cloud.trypowermove.com/v1/aero");
    }

    #[test]
    fn each_picker_level_is_sent_as_reasoning_effort_and_none_sends_none() {
        let with: Value = serde_json::from_str(&configured(
            "https://a.example/v1/aero",
            "openai/gpt-6-sol",
            "xhigh",
        ))
        .unwrap();
        assert_eq!(
            with["aero"]["model_metadata"]["openai/gpt-6-sol"]["reasoning_efforts"]["xhigh"],
            json!({"reasoning_effort":"xhigh"})
        );
        assert_eq!(with["aero"]["protocol"], "openai-chat-completions");
        let without: Value =
            serde_json::from_str(&configured("https://a.example/v1/aero", "m", "")).unwrap();
        assert!(
            without["aero"]["model_metadata"]["m"]
                .get("reasoning_efforts")
                .is_none()
        );
    }
}
