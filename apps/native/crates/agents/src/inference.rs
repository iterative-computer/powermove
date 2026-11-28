//! Aero on Powermove inference, and nothing else: the Powermove account's
//! session pays for every request, on the account's own API origin
//! (`docs/native/aero-inference.md` is the contract). Three calls on the
//! account's origin: `GET /v2/billing` (the plan, its credits this period
//! and what can be bought), `GET /v2/aero/models` (the curated catalogue)
//! and `POST /v2/aero/chat/completions` (OpenAI Chat Completions, streamed;
//! the backend runs the providers through the AI SDK with its own keys). The
//! session stays here and in the app's Keychain; the core gets a provider
//! description without it, and the fetch edge adds it for the Aero origin
//! only.
//!
//! Development builds with the development switch on may point Aero at a
//! local stand-in (`PM_AERO_ENDPOINT=http://127.0.0.1:<port>/<path>`, the
//! test fakes), which gets a development credential, never the person's
//! session. An account on a local development origin (`PM_CLOUD_ORIGIN`,
//! debug builds and the `dev-cloud-origin` feature only) runs on that origin
//! with its own session. Release builds never read either, and Aero never
//! goes through the development fakes that reroute other providers.
//!
//! Aero's model ids are namespaced (`pm/<gateway id>`): a thread saved
//! before Aero ran only on Powermove names no such id, so it never runs
//! until the person picks a model from the catalogue.
use crate::account::Model;
use crate::aero::Live;
use crate::edge::{Idle, Loopback, Net, NetRequest, STREAM_IDLE, fetch};
use crate::launch::Environment;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// Where Aero's calls live on the account's API origin.
pub const PATH: &str = "/v2/aero";
/// The account's billing state on its API origin.
pub const BILLING: &str = "/v2/billing";
/// The development switch's stand-in for Aero's API (debug builds only).
pub const TEST_ENDPOINT: &str = "PM_AERO_ENDPOINT";
/// The development switch's demo model (`PM_AERO_FAKE_MODEL=1` lists it in a
/// debug build; a release build never lists or runs it).
pub const FAKE_MODEL: &str = "fake/model";
/// The prefix of every Aero model id: the catalogue's, never a saved one
/// from before Aero ran only on Powermove inference.
pub const CATALOGUE: &str = "pm/";
/// What the test endpoint is sent instead of the person's session.
pub const DEVELOPMENT_CREDENTIAL: &str = "pm-aero-development";
/// How long a billing or catalogue read may stay silent.
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
    /// Powermove refused this session (401): the app forgets it, as Sign
    /// Out does, and shows signed out everywhere.
    fn ended(&self) {}
}

/// A calendar day (UTC): year, month, day.
pub type Day = (i32, u32, u32);
/// The plan the account is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub price_cents: u64,
}
/// A plan that can be bought (Settings › Account's Subscribe).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanOffer {
    pub id: String,
    pub name: String,
    pub price_cents: u64,
    pub credits: u64,
}
/// The subscription's state, as Stripe has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    PastDue,
    Canceled,
}
/// `GET /v2/billing`: the plan, its credits this period and what can be
/// bought. Credits don't roll over: `included` is this period's grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Billing {
    pub plan: Option<Plan>,
    pub status: Option<Status>,
    pub cancel_at_period_end: bool,
    pub included: u64,
    pub used: u64,
    pub remaining: u64,
    /// The billing period's first and last calendar days (`period.end` is
    /// when the credits reset).
    pub period: Option<(Day, Day)>,
    /// Credits spent each day of the period so far, zero days included.
    pub daily: Vec<(Day, u64)>,
    pub plans: Vec<PlanOffer>,
}
/// What `GET /v2/billing` said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BillingAnswer {
    SignedOut,
    /// The route isn't there (a backend before billing shipped).
    Missing,
    Billing(Billing),
}
/// Where Aero stands for this person, as the picker shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Standing {
    #[default]
    Checking,
    SignedOut,
    /// Signed in on Free (or a cancelled plan): Aero needs Aerolite or Aero.
    Free(Billing),
    Ready {
        billing: Billing,
        models: Vec<Model>,
    },
    /// Powermove couldn't be reached or answered something unreadable.
    Unreachable(String),
}
/// What the picker and the panel say on Free.
pub const NEEDS_PLAN: &str = "Subscribe to Aerolite.";

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

/// The fake model is listed: a debug build whose development switch names
/// it (`PM_AERO_FAKE_MODEL=1`). `development` is `cfg!(debug_assertions)`.
pub fn fake_model_listed(vars: &BTreeMap<String, String>, development: bool) -> bool {
    development && vars.get("PM_AERO_FAKE_MODEL").is_some_and(|v| v == "1")
}
/// `model` runs on the fake: a debug build only.
pub fn runs_fake(model: &str) -> bool {
    cfg!(debug_assertions) && model == FAKE_MODEL
}
/// The catalogue's id for gateway model `id` (`pm/<id>`).
pub fn catalogue_id(id: &str) -> String {
    format!("{CATALOGUE}{id}")
}
/// The gateway id an Aero model id names, when it is a catalogue id.
pub fn gateway_id(model: &str) -> Option<&str> {
    model
        .strip_prefix(CATALOGUE)
        .filter(|id| crate::launch::is_valid_model_id(id))
}

/// Where Aero's calls go for `session`: the API origin (`/v2/...` follows
/// it), the network that reaches it and the bearer it is sent (the session,
/// or the development credential for the test endpoint).
struct Route {
    origin: String,
    net: Arc<dyn Net>,
    bearer: String,
}
/// A build that may run on a local development origin: debug builds, and
/// the Sandbox's `dev-cloud-origin` feature. Never a shipped release.
pub const LOCAL_ORIGINS: bool = cfg!(debug_assertions) || cfg!(feature = "dev-cloud-origin");
/// `origin` is a local development stack's (`http://127.0.0.1:8787`): plain
/// HTTP on this Mac with a port, nothing else.
pub fn local_origin(origin: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
}
fn route(env: &Environment, session: &Session, idle: Duration) -> Result<Route, String> {
    let development = cfg!(debug_assertions) && env.providers();
    let local = |origin: String, bearer: String| Route {
        origin,
        net: Arc::new(Idle {
            inner: Arc::new(Loopback),
            after: idle,
        }),
        bearer,
    };
    if let Some(base) = test_endpoint(&env.vars, development) {
        return Ok(local(base, DEVELOPMENT_CREDENTIAL.into()));
    }
    let origin = session.origin.trim_end_matches('/');
    // The account lives on a local development stack: its own session.
    if LOCAL_ORIGINS && local_origin(origin) {
        return Ok(local(origin.to_owned(), session.token.clone()));
    }
    if !origin.starts_with("https://") {
        return Err("This development account has no Aero endpoint.".into());
    }
    let net = env
        .net
        .clone()
        .ok_or("Powermove's network client is unavailable.")?;
    // The person's session never goes to a development fake.
    if net.rerouted(&format!("{origin}{PATH}")) || net.rerouted(&format!("{origin}{BILLING}")) {
        return Err("Aero never goes through the development fakes. Use PM_AERO_ENDPOINT.".into());
    }
    Ok(Route {
        origin: origin.to_owned(),
        net: Arc::new(Idle {
            inner: net,
            after: idle,
        }),
        bearer: session.token.clone(),
    })
}
fn client(env: &Environment) -> (String, String) {
    (
        "x-powermove-client".into(),
        format!("desktop/{}", env.app_version),
    )
}

/// Reads `GET /v2/billing`, then (on a plan) `GET /v2/aero/models`.
/// Blocks: call it on a worker.
pub fn standing(env: &Environment) -> Standing {
    let Some(session) = env.powermove.as_ref().and_then(|a| a.session()) else {
        return Standing::SignedOut;
    };
    let Route {
        origin,
        net,
        bearer,
    } = match route(env, &session, READ_IDLE) {
        Ok(found) => found,
        Err(reason) => return Standing::Unreachable(reason),
    };
    let headers = [
        ("authorization".to_owned(), format!("Bearer {bearer}")),
        client(env),
    ];
    let get = |path: &str| {
        let url = format!("{origin}{path}");
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
    let Ok((status, body)) = get(BILLING) else {
        return unreachable();
    };
    let ended = || {
        if let Some(account) = env.powermove.as_ref() {
            account.ended();
        }
        Standing::SignedOut
    };
    match parse_billing(status, &body) {
        Ok(BillingAnswer::SignedOut) => ended(),
        Ok(BillingAnswer::Missing) => {
            Standing::Unreachable("Aero isn’t available on Powermove yet.".into())
        }
        Ok(BillingAnswer::Billing(billing)) if !billing.subscribed() => Standing::Free(billing),
        Ok(BillingAnswer::Billing(billing)) => match get(&format!("{PATH}/models")) {
            Ok((200, body)) => match parse_models(&body) {
                Ok(models) => Standing::Ready { billing, models },
                Err(_) => {
                    Standing::Unreachable("Aero couldn’t read its models from Powermove.".into())
                }
            },
            Ok((401, _)) => ended(),
            Ok((403, _)) => Standing::SignedOut,
            _ => unreachable(),
        },
        Err(_) => Standing::Unreachable("Aero couldn’t read your plan from Powermove.".into()),
    }
}

/// `GET /v2/billing`'s answer (the contract's billing section). Every
/// field is checked; an answer that leaves one out is refused, never
/// filled with a default.
pub fn parse_billing(status: u16, body: &[u8]) -> Result<BillingAnswer, String> {
    match status {
        200 => {}
        401 | 403 => return Ok(BillingAnswer::SignedOut),
        404 => return Ok(BillingAnswer::Missing),
        _ => return Err(format!("billing answered HTTP {status}")),
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| "billing is not JSON")?;
    if !value.is_object() {
        return Err("billing is not an object".into());
    }
    let text = |v: &Value, name: &str| {
        v.as_str()
            .map(str::trim)
            .filter(|s| {
                !s.is_empty()
                    && s.chars().count() <= 64
                    && !s.chars().any(|c| {
                        c.is_control()
                            || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                    })
            })
            .map(str::to_owned)
            .ok_or(format!("billing is missing {name}"))
    };
    let id = |v: &Value, name: &str| {
        v.as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 32
                    && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
            })
            .map(str::to_owned)
            .ok_or(format!("billing is missing {name}"))
    };
    let count = |v: &Value, name: &str| v.as_u64().ok_or(format!("billing is missing {name}"));
    let plan = match value.get("plan").ok_or("billing is missing plan")? {
        Value::Null => None,
        plan => Some(Plan {
            id: id(&plan["id"], "plan.id")?,
            name: text(&plan["name"], "plan.name")?,
            price_cents: count(&plan["priceCents"], "plan.priceCents")?,
        }),
    };
    let status = match value.get("status").ok_or("billing is missing status")? {
        Value::Null => None,
        status => Some(match status.as_str() {
            Some("active" | "trialing") => Status::Active,
            Some("past_due" | "unpaid") => Status::PastDue,
            Some("canceled") => Status::Canceled,
            _ => return Err("billing has an unknown status".into()),
        }),
    };
    let credits = &value["credits"];
    let period = match &value["period"] {
        Value::Null => None,
        period => Some((
            period["start"]
                .as_str()
                .and_then(calendar_day)
                .ok_or("billing is missing period.start")?,
            period["end"]
                .as_str()
                .and_then(calendar_day)
                .ok_or("billing is missing period.end")?,
        )),
    };
    let daily = match &value["daily"] {
        Value::Null => vec![],
        daily => daily
            .as_array()
            .ok_or("billing's daily is not a list")?
            .iter()
            .map(|d| {
                Ok((
                    d["date"]
                        .as_str()
                        .and_then(date)
                        .ok_or("billing has a day without a date")?,
                    count(&d["credits"], "daily.credits")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    let plans = value["plans"]
        .as_array()
        .ok_or("billing is missing plans")?
        .iter()
        .map(|p| {
            Ok(PlanOffer {
                id: id(&p["id"], "plans.id")?,
                name: text(&p["name"], "plans.name")?,
                price_cents: count(&p["priceCents"], "plans.priceCents")?,
                credits: count(&p["credits"], "plans.credits")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(BillingAnswer::Billing(Billing {
        plan,
        status,
        cancel_at_period_end: value["cancelAtPeriodEnd"] == true,
        included: count(&credits["included"], "credits.included")?,
        used: count(&credits["used"], "credits.used")?,
        remaining: count(&credits["remaining"], "credits.remaining")?,
        period,
        daily,
        plans,
    }))
}
/// `YYYY-MM-DD`.
fn date(s: &str) -> Option<Day> {
    if s.len() != 10 {
        return None;
    }
    let mut parts = s.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some((year, month, day))
}
/// `YYYY-MM-DD` at the start of an RFC 3339 time.
fn calendar_day(iso: &str) -> Option<Day> {
    (iso.get(10..11) == Some("T")).then_some(())?;
    date(iso.get(..10)?)
}

/// `GET /models`'s answer: an OpenAI-shaped list (`data[].id`), each entry
/// optionally with Powermove's `name`, `efforts`, `defaultEffort` and
/// `context`. Ids come back in the catalogue's namespace (`pm/<id>`).
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
            let name = m["name"]
                .as_str()
                .map(str::trim)
                .filter(|n| {
                    !n.is_empty() && n.chars().count() <= 64 && !n.chars().any(char::is_control)
                })
                .unwrap_or(id);
            // The context window, when the catalogue gives it, as the
            // picker reads it from a label ("(200K context)").
            let label = match m["context"].as_u64().filter(|c| *c >= 1000) {
                Some(c) if c % 1_000_000 == 0 => format!("{name} ({}M context)", c / 1_000_000),
                Some(c) => format!("{name} ({}K context)", c / 1000),
                None => name.to_owned(),
            };
            Some(Model {
                id: catalogue_id(id),
                label,
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

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// `Dec 25`.
pub fn short_day((_, month, day): Day) -> String {
    format!("{} {day}", MONTHS[month as usize - 1])
}
/// Low-credit threshold crossed in the current billing period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreditWarningLevel {
    /// 80%–94% of the period's credits have been spent.
    Warning,
    /// 95%+ of the period's credits have been spent (still above 0).
    Critical,
}
/// When an account is running low on credits (80%+ used, before hitting 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LowCredits {
    pub remaining: u64,
    pub included: u64,
    pub percent_used: u8,
    pub level: CreditWarningLevel,
}
impl Billing {
    /// On a plan that runs Aero: bought and not cancelled (a plan whose
    /// payment failed still runs on the credits it has).
    pub fn subscribed(&self) -> bool {
        self.plan.is_some() && self.status != Some(Status::Canceled)
    }
    /// Low-credit warning at 80% (`Warning`) and 95% (`Critical`) used.
    /// Returns `None` when no credits are included, below 80%, or already at 0
    /// (zero remaining is shown as out-of-credits instead).
    pub fn low_credits(&self) -> Option<LowCredits> {
        if self.included == 0 || self.remaining == 0 {
            return None;
        }
        let used = self.included.saturating_sub(self.remaining);
        let used_100 = u128::from(used) * 100;
        let included = u128::from(self.included);
        let level = if used_100 >= included * 95 {
            CreditWarningLevel::Critical
        } else if used_100 >= included * 80 {
            CreditWarningLevel::Warning
        } else {
            return None;
        };
        let percent_used = ((used_100 / included).min(99)) as u8;
        Some(LowCredits {
            remaining: self.remaining,
            included: self.included,
            percent_used,
            level,
        })
    }
    /// When this period's credits are replaced by the next grant.
    pub fn resets(&self) -> Option<Day> {
        self.period.map(|(_, end)| end)
    }
    /// `812 of 1,000 left`.
    pub fn credits_left(&self) -> String {
        format!(
            "{} of {} left",
            grouped(self.remaining),
            grouped(self.included)
        )
    }
    /// The usage monitor's lines, as the picker card shows them.
    pub fn facts(&self) -> Vec<(String, String)> {
        let mut facts = vec![("Credits".to_owned(), self.credits_left())];
        if let Some(plan) = &self.plan {
            facts.push(("Plan".into(), plan.name.clone()));
        }
        if let Some(day) = self.resets() {
            facts.push(("Resets".into(), short_day(day)));
        }
        facts
    }
    /// A bigger plan to move to (Aerolite → Aero), when there is one.
    pub fn upgrade(&self) -> Option<&PlanOffer> {
        let included = self.included;
        self.plans
            .iter()
            .filter(|p| p.credits > included && Some(&p.id) != self.plan.as_ref().map(|p| &p.id))
            .max_by_key(|p| p.credits)
    }
}
/// `1000` → `1,000`.
pub fn grouped(n: u64) -> String {
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
    live_in(env, model, effort, None)
}
/// [`live`] for Aero session `thread`: every request names it
/// (`X-Powermove-Thread`, the backend's stable prompt-cache key) and carries
/// its own id (`X-Powermove-Request-Id`, so a repeat is never billed twice).
pub fn live_in(
    env: &Environment,
    model: &str,
    effort: &str,
    thread: Option<&str>,
) -> Result<Live, String> {
    let thread = thread
        .filter(|t| request_header_value(t))
        .map(|t| ("x-powermove-thread".to_owned(), t.to_owned()));
    let model = gateway_id(model).ok_or(CHOOSE)?;
    let account = env.powermove.clone().ok_or(SIGN_IN)?;
    let session = account.session().ok_or(SIGN_IN)?;
    let Route {
        origin: api,
        net,
        bearer,
    } = route(env, &session, STREAM_IDLE)?;
    let base = format!("{api}{PATH}");
    let origin = pm_aero_core::origin_of(&base).ok_or("Aero's address is invalid.")?;
    let started = token_id(&session.token);
    let client = client(env);
    let watched = account.clone();
    let fence = started.clone();
    Ok(Live {
        provider: "aero".into(),
        configured: configured(&base, model, effort),
        model: model.into(),
        net,
        // The turn's fence: the account it started on is still signed in.
        // An open stream stops, and no tool runs, once it is not.
        current: Arc::new(move || {
            watched
                .session()
                .is_some_and(|s| token_id(&s.token) == fence)
        }),
        // Read for every request: a sign-out or another account is never
        // carried on with.
        credentials: Arc::new(move |rejected| {
            let session = account.session().ok_or(SIGN_IN)?;
            let id = token_id(&session.token);
            if id != started {
                return Err(CHANGED.into());
            }
            // The backend refused this bearer: there is no other to try,
            // and the app forgets it.
            if rejected.is_some_and(|r| r == bearer) {
                account.ended();
                return Err(SIGN_IN_AGAIN.into());
            }
            let mut headers = vec![
                ("authorization".into(), format!("Bearer {bearer}")),
                client.clone(),
                ("x-powermove-request-id".into(), request_id()),
            ];
            headers.extend(thread.clone());
            Ok((origin.clone(), headers))
        }),
    })
}
/// Why a thread's model can't run: it names none of the catalogue's.
pub const CHOOSE: &str = "Choose a model for Aero from Powermove’s catalogue.";
pub const CHANGED: &str = "The Powermove account changed during this turn. Send it again.";

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

/// The contract's request and thread ids: 8–128 of `A-Za-z0-9._:-`.
fn request_header_value(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}
/// A fresh id for one request (`aero-` and 32 random hex digits).
fn request_id() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    let random = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if random.is_err() {
        // Never a repeat: the time and a counter stand in.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        bytes[..8].copy_from_slice(&now.to_le_bytes());
        bytes[8..].copy_from_slice(&n.to_le_bytes());
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("aero-{hex}")
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

    const AEROLITE: &str = r#"{"plan":{"id":"aerolite","name":"Aerolite","priceCents":999,"interval":"month"},
        "status":"active","cancelAtPeriodEnd":false,
        "credits":{"included":1000,"used":188,"remaining":812},
        "period":{"start":"2026-11-25T00:00:00Z","end":"2026-12-25T00:00:00Z"},
        "daily":[{"date":"2026-11-25","credits":40},{"date":"2026-11-26","credits":0},{"date":"2026-11-27","credits":148}],
        "plans":[{"id":"aerolite","name":"Aerolite","priceCents":999,"credits":1000},
                 {"id":"aero","name":"Aero","priceCents":1999,"credits":2500}]}"#;
    fn billing(body: &str) -> Billing {
        match parse_billing(200, body.as_bytes()) {
            Ok(BillingAnswer::Billing(b)) => b,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_billing_contract_reads_a_plan_its_credits_days_and_reset() {
        let b = billing(AEROLITE);
        assert_eq!(b.plan.as_ref().unwrap().name, "Aerolite");
        assert_eq!(b.plan.as_ref().unwrap().price_cents, 999);
        assert_eq!(b.status, Some(Status::Active));
        assert!(b.subscribed());
        assert_eq!((b.included, b.used, b.remaining), (1000, 188, 812));
        assert_eq!(b.period, Some(((2026, 11, 25), (2026, 12, 25))));
        assert_eq!(b.daily.len(), 3);
        assert_eq!(b.daily[2], ((2026, 11, 27), 148));
        assert_eq!(b.plans[1].credits, 2500);
        assert_eq!(b.upgrade().map(|p| p.id.as_str()), Some("aero"));
        assert_eq!(
            b.facts(),
            [
                ("Credits".to_owned(), "812 of 1,000 left".to_owned()),
                ("Plan".to_owned(), "Aerolite".to_owned()),
                ("Resets".to_owned(), "Dec 25".to_owned()),
            ]
        );
    }

    #[test]
    fn free_has_no_credits_period_or_days_and_a_cancelled_plan_is_free() {
        let free = billing(
            r#"{"plan":null,"status":null,"cancelAtPeriodEnd":false,
            "credits":{"included":0,"used":0,"remaining":0},"period":null,"daily":[],
            "plans":[{"id":"aerolite","name":"Aerolite","priceCents":999,"credits":1000}]}"#,
        );
        assert!(!free.subscribed());
        assert_eq!(free.resets(), None);
        assert!(free.daily.is_empty());
        assert_eq!(
            free.facts(),
            [("Credits".to_owned(), "0 of 0 left".to_owned())]
        );
        let cancelled =
            billing(&AEROLITE.replace(r#""status":"active""#, r#""status":"canceled""#));
        assert!(!cancelled.subscribed());
        let past_due = billing(&AEROLITE.replace(r#""status":"active""#, r#""status":"past_due""#));
        assert_eq!(past_due.status, Some(Status::PastDue));
        assert!(
            past_due.subscribed(),
            "a failed payment still runs on its credits"
        );
        let aero = billing(
            &AEROLITE
                .replace(
                    r#""id":"aerolite","name":"Aerolite","priceCents":999,"interval""#,
                    r#""id":"aero","name":"Aero","priceCents":1999,"interval""#,
                )
                .replace(r#""included":1000"#, r#""included":2500"#),
        );
        assert_eq!(aero.upgrade(), None, "Aero is the biggest plan");
    }

    #[test]
    fn the_billing_contract_refuses_an_answer_missing_a_field() {
        for (from, to, missing) in [
            (r#""remaining":812"#, r#""left":812"#, "credits.remaining"),
            (
                r#""name":"Aerolite","priceCents":999,"interval""#,
                r#""priceCents":999,"interval""#,
                "plan.name",
            ),
            (
                r#""priceCents":999,"interval""#,
                r#""interval""#,
                "plan.priceCents",
            ),
            (
                r#""end":"2026-12-25T00:00:00Z""#,
                r#""end":"soon""#,
                "period.end",
            ),
            (r#""status":"active""#, r#""status":"weird""#, "status"),
            (r#""status":"active","#, "", "status"),
            (
                r#"{"date":"2026-11-25","credits":40}"#,
                r#"{"date":"25/11","credits":40}"#,
                "date",
            ),
            (
                r#""credits":2500}"#,
                r#""credits":"lots"}"#,
                "plans.credits",
            ),
            (
                r#""name":"Aero","#,
                "\"name\":\"Aero\u{202e}\",",
                "plans.name",
            ),
        ] {
            let body = AEROLITE.replace(from, to);
            assert_ne!(body, AEROLITE, "{from}");
            let error = parse_billing(200, body.as_bytes()).unwrap_err();
            assert!(error.contains(missing), "{from}: {error}");
        }
        assert!(parse_billing(200, b"not json").is_err());
        assert!(parse_billing(200, b"[]").is_err());
        // No plan is said explicitly (`plan: null`), never by leaving it out.
        assert!(parse_billing(200, b"{}").unwrap_err().contains("plan"));
    }

    #[test]
    fn the_billing_contract_reads_backend_errors_and_signed_out() {
        assert!(parse_billing(500, br#"{"error":"internal"}"#).is_err());
        assert!(parse_billing(429, b"").is_err());
        assert_eq!(parse_billing(401, b""), Ok(BillingAnswer::SignedOut));
        assert_eq!(parse_billing(404, b"<html>"), Ok(BillingAnswer::Missing));
    }

    #[test]
    fn only_a_local_stack_on_this_mac_is_a_local_origin() {
        assert!(local_origin("http://127.0.0.1:8787"));
        assert!(local_origin("http://localhost:8787/"));
        for elsewhere in [
            "https://127.0.0.1:8787",
            "http://127.0.0.1",
            "http://127.0.0.1:8787/v2",
            "http://evil.example:8787",
            "http://user@127.0.0.1:8787",
            "http://127.0.0.1:8787/?x=1",
            "https://cloud.trypowermove.com",
        ] {
            assert!(!local_origin(elsewhere), "{elsewhere}");
        }
    }

    /// P1 (Sol, PM-750): a release build never lists or runs the fake model.
    #[test]
    fn a_release_build_never_lists_the_fake_model() {
        let named = BTreeMap::from([("PM_AERO_FAKE_MODEL".to_owned(), "1".to_owned())]);
        assert!(!fake_model_listed(&named, false), "release config");
        assert!(fake_model_listed(&named, true));
        assert_eq!(runs_fake(FAKE_MODEL), cfg!(debug_assertions));
        assert!(!runs_fake(&catalogue_id("anthropic/claude-opus-5.5")));
    }
    #[cfg(not(debug_assertions))]
    #[test]
    fn this_release_build_never_runs_the_fake_model() {
        assert!(!runs_fake(FAKE_MODEL));
        let env = Environment::default();
        assert_eq!(live(&env, FAKE_MODEL, "").err().as_deref(), Some(CHOOSE));
    }

    /// P1 (Sol, PM-750): a model id saved before Aero ran only on Powermove
    /// (an old OpenAI, Anthropic, ChatGPT or Custom route) never runs; only
    /// the catalogue's namespaced ids do.
    #[test]
    fn a_saved_byo_model_id_never_runs_on_aero() {
        #[derive(Debug)]
        struct Signed;
        impl PowermoveAccount for Signed {
            fn session(&self) -> Option<Session> {
                Some(Session {
                    origin: "https://cloud.trypowermove.com".into(),
                    token: "t".into(),
                })
            }
        }
        let env = Environment {
            powermove: Some(Arc::new(Signed)),
            net: Some(Arc::new(crate::edge::Loopback)),
            ..Environment::default()
        };
        for saved in [
            "openai/gpt-6-sol",
            "chatgpt/gpt-6-sol",
            "anthropic/claude-opus-5-5",
            "custom/llama-4",
            "pm/",
        ] {
            assert_eq!(
                live(&env, saved, "high").err().as_deref(),
                Some(CHOOSE),
                "{saved}"
            );
        }
        let live = live(&env, "pm/openai/gpt-6-sol", "high").unwrap();
        assert_eq!(live.model, "openai/gpt-6-sol");
    }

    /// Every request names its Aero thread (the backend's prompt-cache key)
    /// and carries an id of its own, never the same twice.
    #[test]
    fn each_request_names_its_thread_and_carries_a_fresh_request_id() {
        #[derive(Debug)]
        struct Signed;
        impl PowermoveAccount for Signed {
            fn session(&self) -> Option<Session> {
                Some(Session {
                    origin: "https://cloud.trypowermove.com".into(),
                    token: "t".into(),
                })
            }
        }
        let env = Environment {
            powermove: Some(Arc::new(Signed)),
            net: Some(Arc::new(crate::edge::Loopback)),
            ..Environment::default()
        };
        let thread = "4f2a9c0d1e3b5a7c";
        let live = live_in(&env, "pm/openai/gpt-6.2-sol", "", Some(thread)).unwrap();
        let header = |headers: &[(String, String)], name: &str| {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        let (_, first) = (live.credentials)(None).unwrap();
        let (_, second) = (live.credentials)(None).unwrap();
        assert_eq!(
            header(&first, "x-powermove-thread").as_deref(),
            Some(thread)
        );
        let (a, b) = (
            header(&first, "x-powermove-request-id").unwrap(),
            header(&second, "x-powermove-request-id").unwrap(),
        );
        assert_ne!(a, b);
        assert!(request_header_value(&a), "{a}");
        // No thread, or one the contract would refuse: none is named.
        let bare = live_in(&env, "pm/openai/gpt-6.2-sol", "", Some("bad id!")).unwrap();
        let (_, headers) = (bare.credentials)(None).unwrap();
        assert_eq!(header(&headers, "x-powermove-thread"), None);
    }

    /// P1 (Sol, PM-750): Aero's traffic never goes through the development
    /// fakes that reroute other providers' origins (they keep the headers).
    /// Debug builds only: a release build has no fakes.
    #[cfg(debug_assertions)]
    #[test]
    fn aero_never_goes_through_the_development_fakes() {
        #[derive(Debug)]
        struct Signed;
        impl PowermoveAccount for Signed {
            fn session(&self) -> Option<Session> {
                Some(Session {
                    origin: "https://cloud.trypowermove.com".into(),
                    token: "real-session".into(),
                })
            }
        }
        let vars = BTreeMap::from([(
            "PM_AGENT_FAKES".to_owned(),
            "https://cloud.trypowermove.com=http://127.0.0.1:9".to_owned(),
        )]);
        let env = Environment {
            powermove: Some(Arc::new(Signed)),
            net: Some(crate::edge::Reroute::from_env(
                &vars,
                Arc::new(crate::edge::Loopback),
            )),
            ..Environment::default()
        };
        assert!(live(&env, "pm/openai/gpt-6-sol", "").is_err());
        assert!(matches!(standing(&env), Standing::Unreachable(_)));
    }

    #[test]
    fn the_catalogue_lists_its_models_with_their_levels() {
        let models = parse_models(br#"{"data":[
            {"id":"anthropic/claude-opus-5.5","name":"Claude Opus 5.5","efforts":["low","medium","high","xhigh","max"],"defaultEffort":"high","context":200000},
            {"id":"openai/gpt-6-sol","name":"GPT-6 Sol","efforts":["low","high"],"defaultEffort":"ultra"},
            {"id":"bad id"},
            {"id":"google/gemini-3-pro"}]}"#)
        .unwrap();
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "pm/anthropic/claude-opus-5.5",
                "pm/openai/gpt-6-sol",
                "pm/google/gemini-3-pro"
            ]
        );
        assert!(models[0].default);
        assert_eq!(models[0].default_effort.as_deref(), Some("high"));
        assert_eq!(
            models[1].default_effort, None,
            "a level the model doesn't list"
        );
        assert_eq!(models[0].label, "Claude Opus 5.5 (200K context)");
        assert_eq!(models[2].label, "google/gemini-3-pro");
        assert!(parse_models(br#"{"data":[]}"#).is_err());
    }

    /// The test endpoint is a development build's only, loopback only.
    #[test]
    fn a_release_build_has_no_test_endpoint() {
        let local = vars("http://127.0.0.1:4014/aero");
        assert_eq!(test_endpoint(&local, false), None, "release config");
        assert_eq!(
            test_endpoint(&local, true).as_deref(),
            Some("http://127.0.0.1:4014/aero")
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
            vars: vars("http://127.0.0.1:4014/aero"),
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
        let found = route(&env, &session, READ_IDLE).unwrap();
        assert_eq!(found.origin, "https://cloud.trypowermove.com");
        assert_eq!(found.bearer, "t");
        // A session on a local stack never runs in a shipped build.
        let local = Session {
            origin: "http://127.0.0.1:8787".into(),
            token: "t".into(),
        };
        assert_eq!(LOCAL_ORIGINS, cfg!(feature = "dev-cloud-origin"));
        if !LOCAL_ORIGINS {
            assert!(route(&env, &local, READ_IDLE).is_err());
        }
    }

    #[test]
    fn each_picker_level_is_sent_as_reasoning_effort_and_none_sends_none() {
        let with: Value = serde_json::from_str(&configured(
            "https://a.example/v2/aero",
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
            serde_json::from_str(&configured("https://a.example/v2/aero", "m", "")).unwrap();
        assert!(
            without["aero"]["model_metadata"]["m"]
                .get("reasoning_efforts")
                .is_none()
        );
    }

    #[test]
    fn low_credits_triggers_at_80_and_95_percent_and_yields_to_zero() {
        let billing = |included, remaining| Billing {
            plan: Some(Plan {
                id: "aerolite".into(),
                name: "Aerolite".into(),
                price_cents: 999,
            }),
            status: Some(Status::Active),
            cancel_at_period_end: false,
            included,
            used: included.saturating_sub(remaining),
            remaining,
            period: Some(((2026, 11, 25), (2026, 12, 25))),
            daily: vec![],
            plans: vec![],
        };
        assert_eq!(billing(0, 0).low_credits(), None);
        assert_eq!(billing(1000, 1000).low_credits(), None);
        assert_eq!(billing(1000, 201).low_credits(), None);
        assert_eq!(
            billing(1000, 200).low_credits(),
            Some(LowCredits {
                remaining: 200,
                included: 1000,
                percent_used: 80,
                level: CreditWarningLevel::Warning,
            })
        );
        assert_eq!(
            billing(1000, 51).low_credits(),
            Some(LowCredits {
                remaining: 51,
                included: 1000,
                percent_used: 94,
                level: CreditWarningLevel::Warning,
            })
        );
        assert_eq!(
            billing(1000, 50).low_credits(),
            Some(LowCredits {
                remaining: 50,
                included: 1000,
                percent_used: 95,
                level: CreditWarningLevel::Critical,
            })
        );
        assert_eq!(
            billing(1000, 1).low_credits(),
            Some(LowCredits {
                remaining: 1,
                included: 1000,
                percent_used: 99,
                level: CreditWarningLevel::Critical,
            })
        );
        // Zero remaining is handled by outOfCredits instead of lowCredits.
        assert_eq!(billing(1000, 0).low_credits(), None);
    }
}
