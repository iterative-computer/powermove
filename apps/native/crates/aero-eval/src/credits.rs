//! Credit-burn measurement for `aero-eval`, using the exact `AERO_PRICING`
//! schema and integer nano-USD math from `apps/cloud/src/aero/pricing.ts`.
//!
//! Converts observed run usage (`run.json`), recorded model streams
//! (`replay/*.json`), or representative workload profiles into USD cost, Aero
//! credits, and estimated edits per plan (1,000 credits for Aerolite; 2,500
//! credits for Aero).
use crate::record::{RunRecord, Usage};
use crate::replay::Recording;
use crate::stats::median_interval;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const AEROLITE_CREDITS: u64 = 1_000;
pub const AERO_CREDITS: u64 = 2_500;

/// Exact mirror of `apps/cloud/src/aero/pricing.ts`'s `AERO_PRICING` JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pricing {
    pub version: String,
    pub credit_nano_usd: u64,
    pub models: BTreeMap<String, ModelPrice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Price {
    pub input: f64,
    pub cached_input: f64,
    pub cache_write: f64,
    pub cache_write_long: f64,
    pub output: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongTier {
    pub above: u64,
    pub input: f64,
    pub cached_input: f64,
    pub cache_write: f64,
    pub cache_write_long: f64,
    pub output: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPrice {
    pub input: f64,
    pub cached_input: f64,
    pub cache_write: f64,
    pub cache_write_long: f64,
    pub output: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long: Option<LongTier>,
}

/// Token counts for one request or one summed run, matching `apps/cloud`'s `Metered`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeteredTokens {
    /// Total input tokens (fresh + cache_read + cache_write).
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cache_write_long: u64,
    /// Output tokens (reasoning included).
    pub output: u64,
}

impl From<Usage> for MeteredTokens {
    fn from(u: Usage) -> Self {
        Self {
            input: u.input,
            cache_read: u.cached,
            cache_write: 0,
            cache_write_long: 0,
            output: u.output,
        }
    }
}

impl MeteredTokens {
    pub fn add(&mut self, other: Self) {
        self.input += other.input;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cache_write_long += other.cache_write_long;
        self.output += other.output;
    }
}

fn nano_rate(usd_per_million: f64) -> Result<u128, String> {
    if !usd_per_million.is_finite() || usd_per_million < 0.0 {
        return Err(format!("invalid price {usd_per_million}"));
    }
    let scaled = usd_per_million * 1000.0;
    let rounded = scaled.round();
    if (scaled - rounded).abs() >= 1e-9 {
        return Err(format!(
            "price {usd_per_million} is not a whole number of nano-USD per token"
        ));
    }
    Ok(rounded as u128)
}

impl ModelPrice {
    fn tier(&self, input: u64) -> Price {
        if let Some(long) = &self.long
            && input > long.above
        {
            return Price {
                input: long.input,
                cached_input: long.cached_input,
                cache_write: long.cache_write,
                cache_write_long: long.cache_write_long,
                output: long.output,
            };
        }
        Price {
            input: self.input,
            cached_input: self.cached_input,
            cache_write: self.cache_write,
            cache_write_long: self.cache_write_long,
            output: self.output,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for v in [
            self.input,
            self.cached_input,
            self.cache_write,
            self.cache_write_long,
            self.output,
        ] {
            nano_rate(v)?;
        }
        if let Some(long) = &self.long {
            if long.above == 0 {
                return Err("long.above must be positive".into());
            }
            for v in [
                long.input,
                long.cached_input,
                long.cache_write,
                long.cache_write_long,
                long.output,
            ] {
                nano_rate(v)?;
            }
        }
        Ok(())
    }

    /// Exact integer nano-USD cost for `usage`, mirroring `apps/cloud/src/aero/pricing.ts:cost`.
    pub fn cost_nano_usd(&self, usage: MeteredTokens) -> u128 {
        let cache_read = usage.cache_read.min(usage.input);
        let cache_write = usage.cache_write.min(usage.input - cache_read);
        let write_long = usage.cache_write_long.min(cache_write);
        let fresh = usage.input - cache_read - cache_write;
        let p = self.tier(usage.input);
        let r = |v: f64| (v * 1000.0).round() as u128;
        u128::from(fresh) * r(p.input)
            + u128::from(cache_read) * r(p.cached_input)
            + u128::from(cache_write - write_long) * r(p.cache_write)
            + u128::from(write_long) * r(p.cache_write_long)
            + u128::from(usage.output) * r(p.output)
    }
}

impl Pricing {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let p: Self = serde_json::from_slice(bytes).map_err(|e| format!("invalid pricing JSON: {e}"))?;
        if p.version.trim().is_empty() {
            return Err("pricing.version is empty".into());
        }
        if p.credit_nano_usd == 0 {
            return Err("pricing.creditNanoUsd must be positive".into());
        }
        if p.models.is_empty() {
            return Err("pricing.models is empty".into());
        }
        for (id, model) in &p.models {
            model
                .validate()
                .map_err(|e| format!("pricing.models[{id}]: {e}"))?;
        }
        Ok(p)
    }

    /// Resolve a model name written as `openai/gpt-6.2-sol`, `pm/openai/gpt-6.2-sol`,
    /// or bare `gpt-6.2-sol`.
    pub fn resolve_model(&self, model: &str) -> Option<(&str, &ModelPrice)> {
        let stripped = model.strip_prefix("pm/").unwrap_or(model);
        if let Some((k, v)) = self.models.get_key_value(stripped) {
            return Some((k.as_str(), v));
        }
        self.models
            .iter()
            .find(|(k, _)| k.split_once('/').is_some_and(|(_, tail)| tail == stripped))
            .map(|(k, v)| (k.as_str(), v))
    }

    pub fn cost_nano_usd(&self, model: &str, usage: MeteredTokens) -> Option<u128> {
        let (_, price) = self.resolve_model(model)?;
        Some(price.cost_nano_usd(usage))
    }

    pub fn credits(&self, nano_usd: u128) -> f64 {
        nano_usd as f64 / self.credit_nano_usd as f64
    }

    pub fn usd(nano_usd: u128) -> f64 {
        nano_usd as f64 / 1_000_000_000.0
    }
}

pub fn edits_per_budget(credits_per_edit: f64, budget_credits: u64) -> Option<f64> {
    (credits_per_edit > 0.0 && credits_per_edit.is_finite())
        .then(|| budget_credits as f64 / credits_per_edit)
}

/// Extract per-exchange token usage from a recorded model stream (OpenAI Responses
/// or Anthropic Messages).
pub fn recording_exchanges(rec: &Recording) -> Vec<MeteredTokens> {
    let mut out = Vec::new();
    for ex in &rec.exchanges {
        let mut openai: Option<MeteredTokens> = None;
        let mut anth_start: Option<(u64, u64, u64, u64)> = None;
        let mut anth_output: u64 = 0;
        let mut anth_seen = false;
        for ev in &ex.response.events {
            let Some(d) = ev.data.as_ref() else {
                continue;
            };
            let ty = d["type"]
                .as_str()
                .or(ev.event.as_deref())
                .unwrap_or("");
            if matches!(ty, "response.completed" | "response.incomplete")
                && let Some(u) = d.pointer("/response/usage")
            {
                openai = Some(parse_openai_usage(u));
            } else if ty == "message_start"
                && let Some(u) = d.pointer("/message/usage")
            {
                let fresh = u["input_tokens"].as_u64().unwrap_or(0);
                let write = u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                let read = u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                let write_long = u
                    .pointer("/cache_creation/ephemeral_1h_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                anth_output = u["output_tokens"].as_u64().unwrap_or(0);
                anth_start = Some((fresh, write, read, write_long));
                anth_seen = true;
            } else if ty == "message_delta"
                && let Some(u) = d.get("usage")
                && let Some(o) = u["output_tokens"].as_u64()
            {
                anth_output = o;
                anth_seen = true;
            }
        }
        if let Some(u) = openai {
            out.push(u);
        } else if anth_seen {
            let (fresh, write, read, write_long) = anth_start.unwrap_or_default();
            out.push(MeteredTokens {
                input: fresh + write + read,
                cache_read: read,
                cache_write: write,
                cache_write_long: write_long,
                output: anth_output,
            });
        } else if let Some(body) = &ex.response.body
            && let Some(u) = body.get("usage")
        {
            out.push(parse_openai_usage(u));
        }
    }
    out
}

fn parse_openai_usage(u: &Value) -> MeteredTokens {
    let details = &u["input_tokens_details"];
    MeteredTokens {
        input: u["input_tokens"].as_u64().unwrap_or(0),
        cache_read: details["cached_tokens"].as_u64().unwrap_or(0),
        cache_write: details["cache_write_tokens"].as_u64().unwrap_or(0),
        cache_write_long: details
            .pointer("/cache_write_tokens_details/ttl_12h_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output: u["output_tokens"].as_u64().unwrap_or(0),
    }
}

/// One row of a credit-burn report for observed `aero-eval` runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunCreditGroup {
    pub harness: String,
    pub model: String,
    pub effort: String,
    pub mode: String,
    pub task: String,
    pub priced_runs: u32,
    pub unpriced_runs: u32,
    pub median_usd: Option<f64>,
    pub mean_usd: Option<f64>,
    pub median_credits: Option<f64>,
    pub mean_credits: Option<f64>,
    pub edits_per_1k: Option<f64>,
    pub edits_per_2500: Option<f64>,
}

/// Summarise credit burn for observed runs (`RunRecord`s) using `pricing`.
pub fn summarize_runs(runs: &[RunRecord], suite_version: u32, pricing: &Pricing) -> Vec<RunCreditGroup> {
    let mut by: BTreeMap<(String, String, String, String, String), Vec<&RunRecord>> = BTreeMap::new();
    for r in runs {
        if r.suite_version != suite_version {
            continue;
        }
        by.entry((
            r.harness.clone(),
            r.model.clone(),
            r.effort.clone(),
            r.mode.clone(),
            "all".into(),
        ))
        .or_default()
        .push(r);
        by.entry((
            r.harness.clone(),
            r.model.clone(),
            r.effort.clone(),
            r.mode.clone(),
            r.task.clone(),
        ))
        .or_default()
        .push(r);
    }
    by.into_iter()
        .map(|((harness, model, effort, mode, task), group)| {
            let mut nanos = Vec::new();
            let mut unpriced = 0u32;
            for r in &group {
                match r.tokens.and_then(|u| pricing.cost_nano_usd(&r.model, u.into())) {
                    Some(n) => nanos.push(n),
                    None => unpriced += 1,
                }
            }
            let usds: Vec<f64> = nanos.iter().map(|&n| Pricing::usd(n)).collect();
            let creds: Vec<f64> = nanos.iter().map(|&n| pricing.credits(n)).collect();
            let median_usd = median_interval(&usds, 0.90).map(|m| m.estimate);
            let mean_usd = (!usds.is_empty()).then(|| usds.iter().sum::<f64>() / usds.len() as f64);
            let median_credits = median_interval(&creds, 0.90).map(|m| m.estimate);
            let mean_credits =
                (!creds.is_empty()).then(|| creds.iter().sum::<f64>() / creds.len() as f64);
            RunCreditGroup {
                harness,
                model,
                effort,
                mode,
                task,
                priced_runs: nanos.len() as u32,
                unpriced_runs: unpriced,
                median_usd,
                mean_usd,
                median_credits,
                mean_credits,
                edits_per_1k: median_credits.and_then(|c| edits_per_budget(c, AEROLITE_CREDITS)),
                edits_per_2500: median_credits.and_then(|c| edits_per_budget(c, AERO_CREDITS)),
            }
        })
        .collect()
}

pub fn render_run_credits(groups: &[RunCreditGroup], pricing: &Pricing) -> String {
    let mut out = format!(
        "credit burn — pricing \"{}\" (1 credit = ${:.5}, 1,000 credits = ${:.2})\n",
        pricing.version,
        Pricing::usd(u128::from(pricing.credit_nano_usd)),
        Pricing::usd(u128::from(pricing.credit_nano_usd) * u128::from(AEROLITE_CREDITS)),
    );
    out.push_str(
        "harness       model                       effort  task                          runs   med USD   med cred  edits/1k  edits/2.5k\n",
    );
    for g in groups {
        let fmt_opt = |v: Option<f64>, f: fn(f64) -> String| {
            v.map(f).unwrap_or_else(|| "n/a".into())
        };
        out.push_str(&format!(
            "{:<13} {:<27} {:<7} {:<29} {:>4}  {:>8}  {:>9}  {:>8}  {:>10}\n",
            g.harness,
            g.model,
            g.effort,
            g.task,
            g.priced_runs,
            fmt_opt(g.median_usd, |u| format!("${u:.4}")),
            fmt_opt(g.median_credits, |c| format!("{c:.2}")),
            fmt_opt(g.edits_per_1k, |e| format!("{e:.0}")),
            fmt_opt(g.edits_per_2500, |e| format!("{e:.0}")),
        ));
    }
    out
}

/// Representative multi-turn Powermove agent edit profiles (per-turn token
/// shapes with prompt caching across tool calls), used to project credit burn
/// across every model in the Aero catalogue before a live batch is approved.
#[derive(Clone, Debug)]
pub struct WorkloadProfile {
    pub name: &'static str,
    pub description: &'static str,
    pub turns: &'static [MeteredTokens],
}

/// Three realistic Powermove agent edit envelopes (with prompt caching on):
/// - `quick-edit`: 3 turns (inspect timeline, apply cut/grade, confirm)
/// - `typical-edit`: 6 turns (inspect, read frame/props, edit, check render, tweak)
/// - `heavy-extension`: 12 turns (build/fix custom effect or panel + verify renders)
pub const WORKLOAD_PROFILES: &[WorkloadProfile] = &[
    WorkloadProfile {
        name: "quick-edit",
        description: "3 turns (cut/trim/grade, ~30k cached prompt + 1.5k output)",
        turns: &[
            MeteredTokens { input: 10_000, cache_read: 0, cache_write: 8_000, cache_write_long: 0, output: 450 },
            MeteredTokens { input: 11_500, cache_read: 9_500, cache_write: 1_500, cache_write_long: 0, output: 600 },
            MeteredTokens { input: 12_500, cache_read: 11_000, cache_write: 1_000, cache_write_long: 0, output: 450 },
        ],
    },
    WorkloadProfile {
        name: "typical-edit",
        description: "6 turns (multi-step edit + frame check, ~110k input mostly cached + 4.5k output)",
        turns: &[
            MeteredTokens { input: 12_000, cache_read: 0, cache_write: 10_000, cache_write_long: 0, output: 600 },
            MeteredTokens { input: 15_000, cache_read: 11_500, cache_write: 3_000, cache_write_long: 0, output: 800 },
            MeteredTokens { input: 18_000, cache_read: 14_500, cache_write: 3_000, cache_write_long: 0, output: 900 },
            MeteredTokens { input: 20_500, cache_read: 17_500, cache_write: 2_500, cache_write_long: 0, output: 800 },
            MeteredTokens { input: 22_000, cache_read: 19_500, cache_write: 2_000, cache_write_long: 0, output: 800 },
            MeteredTokens { input: 23_500, cache_read: 21_500, cache_write: 1_500, cache_write_long: 0, output: 600 },
        ],
    },
    WorkloadProfile {
        name: "heavy-extension",
        description: "12 turns (build/debug custom effect or panel + render checks, ~360k input mostly cached + 14k output)",
        turns: &[
            MeteredTokens { input: 14_000, cache_read: 0, cache_write: 12_000, cache_write_long: 0, output: 900 },
            MeteredTokens { input: 18_000, cache_read: 13_500, cache_write: 4_000, cache_write_long: 0, output: 1_200 },
            MeteredTokens { input: 22_000, cache_read: 17_500, cache_write: 4_000, cache_write_long: 0, output: 1_400 },
            MeteredTokens { input: 25_000, cache_read: 21_500, cache_write: 3_000, cache_write_long: 0, output: 1_400 },
            MeteredTokens { input: 28_000, cache_read: 24_500, cache_write: 3_000, cache_write_long: 0, output: 1_400 },
            MeteredTokens { input: 31_000, cache_read: 27_500, cache_write: 3_000, cache_write_long: 0, output: 1_200 },
            MeteredTokens { input: 33_000, cache_read: 30_500, cache_write: 2_000, cache_write_long: 0, output: 1_200 },
            MeteredTokens { input: 35_000, cache_read: 32_500, cache_write: 2_000, cache_write_long: 0, output: 1_200 },
            MeteredTokens { input: 37_000, cache_read: 34_500, cache_write: 2_000, cache_write_long: 0, output: 1_100 },
            MeteredTokens { input: 39_000, cache_read: 36_500, cache_write: 2_000, cache_write_long: 0, output: 1_100 },
            MeteredTokens { input: 40_500, cache_read: 38_500, cache_write: 1_500, cache_write_long: 0, output: 1_000 },
            MeteredTokens { input: 42_000, cache_read: 40_000, cache_write: 1_500, cache_write_long: 0, output: 900 },
        ],
    },
];

/// Render a model-by-model credit table for a sequence of exchanges (e.g. a
/// recording or a workload profile).
pub fn render_exchanges_table(
    title: &str,
    exchanges: &[MeteredTokens],
    pricing: &Pricing,
) -> String {
    let mut totals = MeteredTokens::default();
    for &u in exchanges {
        totals.add(u);
    }
    let mut out = format!(
        "{title} — {} exchanges (input {} [cached {}, write {}], output {})\n",
        exchanges.len(),
        totals.input,
        totals.cache_read,
        totals.cache_write,
        totals.output,
    );
    out.push_str(
        "model                         USD/edit   cred/edit  edits/1k (Aerolite)  edits/2.5k (Aero)\n",
    );
    for (id, mp) in &pricing.models {
        let nano: u128 = exchanges.iter().map(|&u| mp.cost_nano_usd(u)).sum();
        let usd = Pricing::usd(nano);
        let cred = pricing.credits(nano);
        let e1k = edits_per_budget(cred, AEROLITE_CREDITS)
            .map(|e| format!("{e:.0}"))
            .unwrap_or_else(|| "n/a".into());
        let e25 = edits_per_budget(cred, AERO_CREDITS)
            .map(|e| format!("{e:.0}"))
            .unwrap_or_else(|| "n/a".into());
        out.push_str(&format!(
            "{:<29} ${:<9.4} {:>9.2}  {:>19}  {:>17}\n",
            id, usd, cred, e1k, e25
        ));
    }
    out
}

pub fn render_profiles(pricing: &Pricing) -> String {
    let mut out = format!(
        "Aero credit projection — pricing \"{}\" (1 credit = ${:.5}, 1,000 credits = ${:.2})\n\n",
        pricing.version,
        Pricing::usd(u128::from(pricing.credit_nano_usd)),
        Pricing::usd(u128::from(pricing.credit_nano_usd) * u128::from(AEROLITE_CREDITS)),
    );
    for p in WORKLOAD_PROFILES {
        out.push_str(&render_exchanges_table(
            &format!("{} ({})", p.name, p.description),
            p.turns,
            pricing,
        ));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_pricing() -> Pricing {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../cloud/test/fixtures/pricing.json");
        Pricing::load(&path).expect("cloud fixture pricing.json")
    }

    #[test]
    fn nano_usd_cost_matches_cloud_pricing_tests_exactly() {
        let p = fixture_pricing();
        assert_eq!(p.credit_nano_usd, 1_000_000);
        // Mirrors apps/cloud/test/aero.test.ts lines 36-50.
        let sol = &p.models["openai/gpt-6.2-sol"];
        assert_eq!(
            sol.cost_nano_usd(MeteredTokens {
                input: 200_000,
                ..Default::default()
            }),
            400_000_000
        );
        assert_eq!(
            sol.cost_nano_usd(MeteredTokens {
                input: 200_000,
                cache_read: 200_000,
                ..Default::default()
            }),
            20_000_000
        );
        assert_eq!(
            sol.cost_nano_usd(MeteredTokens {
                input: 1,
                ..Default::default()
            }),
            2_000
        );
        assert_eq!(
            sol.cost_nano_usd(MeteredTokens {
                input: 272_000,
                output: 1_000,
                ..Default::default()
            }),
            272_000 * 2_000 + 1_000 * 10_000
        );
        assert_eq!(
            sol.cost_nano_usd(MeteredTokens {
                input: 272_001,
                output: 1_000,
                ..Default::default()
            }),
            272_001 * 4_000 + 1_000 * 15_000
        );
        assert_eq!(
            sol.cost_nano_usd(MeteredTokens {
                input: 300_000,
                cache_read: 290_000,
                ..Default::default()
            }),
            10_000 * 4_000 + 290_000 * 200
        );
        let opus = &p.models["anthropic/claude-opus-5-5"];
        assert_eq!(
            opus.cost_nano_usd(MeteredTokens {
                input: 1_000_000,
                cache_write: 1_000_000,
                ..Default::default()
            }),
            5_000 * 1_000_000
        );
        assert_eq!(
            opus.cost_nano_usd(MeteredTokens {
                input: 1_000_000,
                cache_write: 1_000_000,
                cache_write_long: 1_000_000,
                ..Default::default()
            }),
            8_000_000_000
        );
    }

    #[test]
    fn resolves_prefixed_and_bare_model_names() {
        let p = fixture_pricing();
        assert_eq!(
            p.resolve_model("pm/openai/gpt-6.2-sol").map(|(k, _)| k),
            Some("openai/gpt-6.2-sol")
        );
        assert_eq!(
            p.resolve_model("claude-opus-5-5").map(|(k, _)| k),
            Some("anthropic/claude-opus-5-5")
        );
    }

    #[test]
    fn extracts_usage_from_synthetic_toy_recording() {
        let path = crate::data_root().join("replay/synthetic-toy-cut.json");
        let rec = Recording::load(&path).unwrap();
        let ex = recording_exchanges(&rec);
        assert_eq!(ex.len(), 3);
        assert_eq!(
            ex[0],
            MeteredTokens {
                input: 900,
                cache_read: 600,
                cache_write: 0,
                cache_write_long: 0,
                output: 40,
            }
        );
    }
}
