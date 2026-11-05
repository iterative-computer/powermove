//! S079–S082. v1.2.0 shared/publish.ts, store/publish-form.ts,
//! main/cloud/publish.ts. Validation and a confirmation-bound worker protocol.
//! Scanner, compiler, upload, account and native confirmation are capabilities,
//! never inferred successes. No public network mutation runs in these models.
use super::{Flight, Ticket};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub path: String,
    pub line: u32,
}
impl Finding {
    pub fn key(&self) -> String {
        format!("{}:{}", self.path, self.line)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub local_id: String,
    pub coordinate: String,
    pub suggested_version: String,
    pub last_version: Option<String>,
    pub first_publish: bool,
    pub listing: Value,
    pub blocked_findings: Vec<Value>,
    pub permission_findings: Vec<Value>,
    pub waivable_findings: Vec<Finding>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Draft {
    pub version: String,
    pub notes: String,
    pub name: String,
    pub tagline: String,
    pub category: String,
    pub licence: String,
    pub visibility: String,
    pub icon_png: Option<String>,
    pub reasons: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Problem {
    VersionRequired,
    VersionInvalid,
    VersionNotHigher,
    NotesTooLong,
    NameRequired,
    NameTooLong,
    TaglineTooLong,
    WaiverTooShort,
    WaiverTooLong,
    BlockedFinding,
    PermissionFinding,
    CategoryInvalid,
    LicenceInvalid,
    VisibilityInvalid,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldProblem {
    pub field: String,
    pub problem: Problem,
}
fn version(s: &str) -> Option<[u32; 3]> {
    let parts: Vec<_> = s.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|s| s.is_empty() || s.len() > 6 || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}
fn chars(s: &str) -> usize {
    s.encode_utf16().count()
}
impl Draft {
    pub fn for_plan(plan: &Plan) -> Self {
        let field = |key| {
            plan.listing
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        Self {
            version: plan.suggested_version.clone(),
            name: field("name"),
            tagline: field("tagline"),
            category: field("category"),
            licence: "MIT".into(),
            visibility: "public".into(),
            ..Self::default()
        }
    }
    pub fn problems(&self, plan: &Plan) -> Vec<FieldProblem> {
        let mut result = Vec::new();
        let mut add = |field: &str, problem| {
            result.push(FieldProblem {
                field: field.into(),
                problem,
            })
        };
        let value = self.version.trim();
        if value.is_empty() {
            add("version", Problem::VersionRequired);
        } else if let Some(v) = version(value) {
            if plan
                .last_version
                .as_deref()
                .and_then(version)
                .is_some_and(|last| v <= last)
            {
                add("version", Problem::VersionNotHigher);
            }
        } else {
            add("version", Problem::VersionInvalid);
        }
        if chars(&self.notes) > 4000 {
            add("notes", Problem::NotesTooLong);
        }
        if plan.first_publish {
            if self.name.trim().is_empty() {
                add("name", Problem::NameRequired);
            } else if chars(self.name.trim()) > 80 {
                add("name", Problem::NameTooLong);
            }
            if chars(&self.tagline) > 160 {
                add("tagline", Problem::TaglineTooLong);
            }
            if ![
                "effects",
                "transitions",
                "panels",
                "layers",
                "themes",
                "commands",
                "tools",
            ]
            .contains(&self.category.as_str())
            {
                add("category", Problem::CategoryInvalid);
            }
            if self.licence != "MIT" {
                add("licence", Problem::LicenceInvalid);
            }
            if !["public", "unlisted"].contains(&self.visibility.as_str()) {
                add("visibility", Problem::VisibilityInvalid);
            }
        }
        if !plan.blocked_findings.is_empty() {
            add("findings", Problem::BlockedFinding);
        }
        if !plan.permission_findings.is_empty() {
            add("permissions", Problem::PermissionFinding);
        }
        for finding in &plan.waivable_findings {
            let key = finding.key();
            let n = chars(self.reasons.get(&key).map(|s| s.trim()).unwrap_or(""));
            if n < 3 {
                add(&key, Problem::WaiverTooShort);
            } else if n > 200 {
                add(&key, Problem::WaiverTooLong);
            }
        }
        result
    }
    pub fn form(&self, plan: &Plan) -> Result<Value, Vec<FieldProblem>> {
        let problems = self.problems(plan);
        if !problems.is_empty() {
            return Err(problems);
        }
        let mut form = json!({"version":self.version.trim(),"waivers":plan.waivable_findings.iter().map(|f| json!({"path":f.path,"line":f.line,"reason":self.reasons.get(&f.key()).map(|s|s.trim()).unwrap_or("")})).collect::<Vec<_>>()});
        if !self.notes.trim().is_empty() {
            form["notes"] = json!(self.notes.trim());
        }
        if plan.first_publish {
            form["listing"] = json!({"name":self.name.trim(),"tagline":self.tagline.trim(),"category":self.category,"licence":self.licence});
            form["visibility"] = json!(self.visibility);
            if let Some(icon) = self.icon_png.as_ref().filter(|s| !s.is_empty()) {
                form["iconPng"] = json!(icon);
            }
        }
        Ok(form)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconProblem {
    TooLarge,
    NotPng,
    NotSquare,
    InvalidDimensions,
}
/// Cheap preflight only. Host must fully decode PNG before upload. Unlike the
/// tagged 24-byte probe, zero-sized or non-IHDR input never appears accepted.
pub fn check_icon(bytes: &[u8]) -> Result<(), IconProblem> {
    if bytes.len() > 256 * 1024 {
        return Err(IconProblem::TooLarge);
    }
    if bytes.len() < 33
        || !bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10])
        || &bytes[12..16] != b"IHDR"
        || bytes[8..12] != 13u32.to_be_bytes()
    {
        return Err(IconProblem::NotPng);
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err(IconProblem::InvalidDimensions);
    }
    if width != height {
        return Err(IconProblem::NotSquare);
    }
    Ok(())
}

/// Worker owns the exact immutable frozen bytes identified by this descriptor.
/// UI never gets tokens/source bytes. The hash is not a scanner pass receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub local_id: String,
    pub coordinate: String,
    pub version: String,
    pub tree_sha: String,
    pub file_count: u32,
    pub size_bytes: u64,
    pub account_id: String,
    pub registry_origin: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Idle,
    Preparing,
    AwaitingConfirmation,
    Checking,
    Uploading { done: u32, total: u32 },
    Publishing,
    Published { release_id: String },
    Cancelled,
    Failed { code: String },
}
#[derive(Debug)]
pub struct Publish {
    pub phase: Phase,
    flight: Flight,
    snapshot: Option<Snapshot>,
}
impl Default for Publish {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            flight: Flight::default(),
            snapshot: None,
        }
    }
}
/// Constructed only by confirming the currently displayed frozen descriptor.
#[derive(Debug)]
pub struct Authorization {
    ticket: Ticket,
    snapshot: Snapshot,
}
impl Authorization {
    pub fn ticket(&self) -> Ticket {
        self.ticket
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}
impl Publish {
    pub fn prepare(&mut self) -> Option<Ticket> {
        if self.flight.current().is_some() {
            return None;
        }
        self.phase = Phase::Preparing;
        self.snapshot = None;
        Some(self.flight.start())
    }
    /// Worker calls only after fresh snapshot + scanner + permission/manifest
    /// checks; these capabilities remain mandatory outside this model.
    pub fn prepared(&mut self, ticket: Ticket, snapshot: Snapshot) -> bool {
        if !self.flight.accepts(ticket) || self.phase != Phase::Preparing {
            return false;
        }
        self.snapshot = Some(snapshot);
        self.phase = Phase::AwaitingConfirmation;
        true
    }
    pub fn confirm(
        &mut self,
        ticket: Ticket,
        shown: &Snapshot,
        account: &str,
        origin: &str,
        accepted: bool,
    ) -> Option<Authorization> {
        if !self.flight.accepts(ticket)
            || self.phase != Phase::AwaitingConfirmation
            || self.snapshot.as_ref() != Some(shown)
        {
            return None;
        }
        if !accepted {
            self.cancel();
            return None;
        }
        if shown.account_id != account || shown.registry_origin != origin {
            self.fail(ticket, "authority_changed");
            return None;
        }
        self.phase = Phase::Checking;
        Some(Authorization {
            ticket,
            snapshot: shown.clone(),
        })
    }
    pub fn progress(&mut self, ticket: Ticket, phase: Phase) -> bool {
        if !self.flight.accepts(ticket) {
            return false;
        }
        let allowed = match (&self.phase, &phase) {
            (Phase::Checking, Phase::Uploading { done, total }) => done <= total,
            (
                Phase::Uploading {
                    done: old,
                    total: old_total,
                },
                Phase::Uploading { done, total },
            ) => total == old_total && done >= old && done <= total,
            (Phase::Checking, Phase::Publishing) => true,
            (Phase::Uploading { done, total }, Phase::Publishing) => done == total,
            _ => false,
        };
        if allowed {
            self.phase = phase;
        }
        allowed
    }
    /// A response must name the same tree/version/coordinate; the worker also
    /// verifies the server commit hash before publishing durable provenance.
    pub fn published(
        &mut self,
        authorization: Authorization,
        returned: &Snapshot,
        release_id: String,
    ) -> bool {
        if self.phase != Phase::Publishing
            || !self.flight.accepts(authorization.ticket)
            || self.snapshot.as_ref() != Some(&authorization.snapshot)
            || returned != &authorization.snapshot
            || release_id.is_empty()
        {
            return false;
        }
        self.flight.finish(authorization.ticket);
        self.phase = Phase::Published { release_id };
        true
    }
    pub fn fail(&mut self, ticket: Ticket, code: &str) -> bool {
        if !self.flight.finish(ticket) {
            return false;
        }
        self.phase = Phase::Failed { code: code.into() };
        true
    }
    pub fn cancel(&mut self) -> Option<Ticket> {
        let ticket = self.flight.cancel();
        self.phase = Phase::Cancelled;
        ticket
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> Plan {
        Plan {
            local_id: "local".into(),
            coordinate: "author/tool".into(),
            suggested_version: "1.2.1".into(),
            last_version: Some("1.2.0".into()),
            first_publish: true,
            listing: json!({"name":"Tool","tagline":"","category":"tools"}),
            blocked_findings: vec![],
            permission_findings: vec![],
            waivable_findings: vec![],
        }
    }
    fn snapshot() -> Snapshot {
        Snapshot {
            local_id: "local".into(),
            coordinate: "author/tool".into(),
            version: "1.2.1".into(),
            tree_sha: "synthetic-tree".into(),
            file_count: 2,
            size_bytes: 20,
            account_id: "author".into(),
            registry_origin: "https://example.com".into(),
        }
    }
    #[test]
    fn tagged_version_grammar_includes_leading_zero_and_six_digits() {
        for v in ["01.02.000003", "999999.0.1", "1.2.1"] {
            assert!(version(v).is_some());
        }
        for v in ["1.2", "1.2.3-beta", "1000000.1.1", "v1.2.3", "1.2.-1"] {
            assert!(version(v).is_none());
        }
        let p = plan();
        let mut d = Draft::for_plan(&p);
        d.version = " 01.002.000000 ".into();
        assert!(
            d.problems(&p)
                .iter()
                .any(|p| p.problem == Problem::VersionNotHigher)
        );
    }
    #[test]
    fn update_omits_first_publish_fields_and_trims_exactly() {
        let mut p = plan();
        p.first_publish = false;
        let mut d = Draft::for_plan(&p);
        d.name.clear();
        d.category.clear();
        d.notes = " notes ".into();
        let f = d.form(&p).unwrap();
        assert_eq!(f, json!({"version":"1.2.1","notes":"notes","waivers":[]}));
    }
    #[test]
    fn utf16_limits_and_each_waiver_are_enforced() {
        let mut p = plan();
        p.waivable_findings = vec![Finding {
            path: "a.ts".into(),
            line: 2,
        }];
        let mut d = Draft::for_plan(&p);
        d.name = "😀".repeat(41);
        d.notes = "😀".repeat(2001);
        let problems = d.problems(&p);
        for problem in [
            Problem::NameTooLong,
            Problem::NotesTooLong,
            Problem::WaiverTooShort,
        ] {
            assert!(problems.iter().any(|p| p.problem == problem));
        }
        d.name = "Tool".into();
        d.notes.clear();
        d.reasons.insert("a.ts:2".into(), " safe fixture ".into());
        assert_eq!(d.form(&p).unwrap()["waivers"][0]["reason"], "safe fixture");
    }
    #[test]
    fn hard_and_permission_findings_cannot_be_waived() {
        let mut p = plan();
        let d = Draft::for_plan(&p);
        p.blocked_findings.push(json!({"kind":"fixture"}));
        p.permission_findings.push(json!({"needs":"network"}));
        let errors = d.form(&p).unwrap_err();
        assert!(errors.iter().any(|p| p.problem == Problem::BlockedFinding));
        assert!(
            errors
                .iter()
                .any(|p| p.problem == Problem::PermissionFinding)
        );
    }
    #[test]
    fn icon_preflight_rejects_tag_probe_false_positives() {
        assert_eq!(
            check_icon(&vec![0; 256 * 1024 + 1]),
            Err(IconProblem::TooLarge)
        );
        assert_eq!(check_icon(&[0; 24]), Err(IconProblem::NotPng));
        let mut bytes = vec![0; 33];
        bytes[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        bytes[8..12].copy_from_slice(&13u32.to_be_bytes());
        bytes[12..16].copy_from_slice(b"IHDR");
        assert_eq!(check_icon(&bytes), Err(IconProblem::InvalidDimensions));
        bytes[16..20].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(check_icon(&bytes), Err(IconProblem::InvalidDimensions));
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(check_icon(&bytes), Ok(()));
    }
    #[test]
    fn confirmation_binds_tree_account_and_single_publish() {
        let mut p = Publish::default();
        let t = p.prepare().unwrap();
        assert!(p.prepare().is_none());
        let s = snapshot();
        assert!(p.prepared(t, s.clone()));
        let mut wrong = s.clone();
        wrong.tree_sha = "other".into();
        assert!(
            p.confirm(t, &wrong, "author", "https://example.com", true)
                .is_none()
        );
        let auth = p
            .confirm(t, &s, "author", "https://example.com", true)
            .unwrap();
        assert!(
            p.confirm(t, &s, "author", "https://example.com", true)
                .is_none()
        );
        assert!(!p.progress(t, Phase::Uploading { done: 3, total: 2 }));
        assert!(p.progress(t, Phase::Uploading { done: 0, total: 2 }));
        assert!(!p.progress(t, Phase::Publishing));
        assert!(p.progress(t, Phase::Uploading { done: 2, total: 2 }));
        assert!(p.progress(t, Phase::Publishing));
        assert!(p.published(auth, &s, "release".into()));
        assert!(!p.fail(t, "late"));
    }
    #[test]
    fn cancel_and_changed_account_do_not_authorize_upload() {
        let mut p = Publish::default();
        let t = p.prepare().unwrap();
        let s = snapshot();
        p.prepared(t, s.clone());
        assert!(
            p.confirm(t, &s, "other", "https://example.com", true)
                .is_none()
        );
        assert!(matches!(p.phase, Phase::Failed { .. }));
        let t = p.prepare().unwrap();
        p.prepared(t, s.clone());
        assert!(
            p.confirm(t, &s, "author", "https://example.com", false)
                .is_none()
        );
        assert_eq!(p.phase, Phase::Cancelled);
        assert!(!p.prepared(t, s));
    }
}
