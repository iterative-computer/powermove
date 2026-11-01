use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

pub const WORKFLOWS: &str = include_str!("../../../../../docs/native/w1-workflows.md");
pub const KEYMAP: &str = include_str!("../../../../../docs/native/w1-keymap.md");
pub const TRIAGE: &str = include_str!("../../../../../docs/native/e2e-triage.csv");

#[derive(Clone, Debug, Serialize)]
pub struct Contract {
    pub id: String,
    pub expected: String,
    pub source: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Workflow {
    pub id: String,
    pub title: String,
    pub contract: String,
    pub cases: Vec<Contract>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Binding {
    pub id: String,
    pub chord: String,
    pub command: String,
    pub args: Value,
    pub priority: i64,
    pub in_fields: bool,
    pub repeat: bool,
    pub loose: bool,
    pub workflow: String,
    pub source: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct LegacySpec {
    pub file: String,
    pub in_370_roster: bool,
    pub in_tag: bool,
    pub in_main: bool,
    pub revision: String,
    pub workflows: Vec<String>,
    pub classification: String,
    pub disposition: String,
    pub evidence: String,
    pub conditional_or_skip: bool,
    pub coverage_limit: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Catalog {
    pub workflows: Vec<Workflow>,
    pub bindings: Vec<Binding>,
    pub specs: Vec<LegacySpec>,
}
fn cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split(" | ")
        .map(str::trim)
        .collect()
}
fn stable_id(s: &str, prefixes: &[u8], digits: usize) -> bool {
    let b = s.as_bytes();
    b.len() == digits + 1 && prefixes.contains(&b[0]) && b[1..].iter().all(u8::is_ascii_digit)
}
fn boolean(s: &str) -> Result<bool, String> {
    match s {
        "yes" | "true" => Ok(true),
        "no" | "false" => Ok(false),
        _ => Err(format!("invalid boolean {s:?}")),
    }
}
impl Catalog {
    pub fn embedded() -> Result<Self, String> {
        Self::parse(WORKFLOWS, KEYMAP, TRIAGE)
    }
    pub fn parse(workflows: &str, keymap: &str, triage: &str) -> Result<Self, String> {
        let mut groups = Vec::<Workflow>::new();
        let mut in_contracts = false;
        for line in workflows.lines() {
            if line == "## Workflow contracts" {
                in_contracts = true;
            } else if in_contracts && line.starts_with("## ") {
                break; // Appendix claims are not shipped acceptance variants.
            } else if in_contracts {
                if let Some(heading) = line.strip_prefix("### ") {
                    let (id, title) = heading.split_once(" — ").ok_or("invalid W heading")?;
                    if !stable_id(id, b"W", 3) {
                        return Err("invalid workflow id".into());
                    }
                    groups.push(Workflow {
                        id: id.into(),
                        title: title.into(),
                        contract: String::new(),
                        cases: vec![],
                    });
                } else if let Some(group) = groups.last_mut() {
                    group.contract.push_str(line);
                    group.contract.push('\n');
                    if line.starts_with("| ") {
                        let row = cells(line);
                        if stable_id(row[0], b"AB", 3) {
                            if row.len() != 3 {
                                return Err(format!("invalid case row {}", row[0]));
                            }
                            group.cases.push(Contract {
                                id: row[0].into(),
                                expected: row[1].into(),
                                source: row[2].into(),
                            });
                        }
                    }
                }
            }
        }
        let mut bindings = vec![];
        for line in keymap.lines().filter(|line| line.starts_with("| K")) {
            let r = cells(line);
            if !stable_id(r[0], b"K", 3) {
                continue;
            }
            if r.len() != 10 || !stable_id(r[0], b"K", 3) {
                return Err("invalid key row".into());
            }
            bindings.push(Binding {
                id: r[0].into(),
                chord: r[1].replace("&#96;", "`"),
                command: r[2].into(),
                args: serde_json::from_str(r[3]).map_err(|e| e.to_string())?,
                priority: r[4].parse::<i64>().map_err(|e| e.to_string())?,
                in_fields: boolean(r[5])?,
                repeat: boolean(r[6])?,
                loose: boolean(r[7])?,
                workflow: r[8].into(),
                source: r[9].into(),
            });
        }
        let rows = csv_rows(triage)?;
        let header = [
            "spec_file",
            "in_370_roster",
            "in_v1_2_0",
            "in_main",
            "evidence_revision",
            "workflow_ids",
            "classification",
            "disposition",
            "evidence",
            "conditional_or_skip_present",
            "coverage_limit",
        ];
        if rows
            .first()
            .map(|r| r.iter().map(String::as_str).collect::<Vec<_>>())
            != Some(header.to_vec())
        {
            return Err("unexpected triage header".into());
        }
        let mut specs = vec![];
        for r in rows.into_iter().skip(1) {
            if r.len() != 11 {
                return Err("invalid triage row length".into());
            }
            specs.push(LegacySpec {
                file: r[0].clone(),
                in_370_roster: boolean(&r[1])?,
                in_tag: boolean(&r[2])?,
                in_main: boolean(&r[3])?,
                revision: r[4].clone(),
                workflows: r[5].split(';').map(str::to_owned).collect(),
                classification: r[6].clone(),
                disposition: r[7].clone(),
                evidence: r[8].clone(),
                conditional_or_skip: boolean(&r[9])?,
                coverage_limit: r[10].clone(),
            });
        }
        let catalog = Self {
            workflows: groups,
            bindings,
            specs,
        };
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn validate(&self) -> Result<(), String> {
        let ids = self
            .workflows
            .iter()
            .map(|w| w.id.as_str())
            .collect::<BTreeSet<_>>();
        if ids.len() != self.workflows.len() {
            return Err("duplicate workflow".into());
        }
        let mut cases = BTreeSet::new();
        for w in &self.workflows {
            for c in &w.cases {
                if !cases.insert(&c.id) {
                    return Err(format!("duplicate case {}", c.id));
                }
            }
        }
        let mut keys = BTreeSet::new();
        for k in &self.bindings {
            if !keys.insert(&k.id) || !ids.contains(k.workflow.as_str()) {
                return Err(format!("invalid binding {}", k.id));
            }
        }
        let mut files = BTreeSet::new();
        for s in &self.specs {
            if !files.insert(&s.file)
                || s.workflows.is_empty()
                || s.workflows.iter().any(|w| !ids.contains(w.as_str()))
            {
                return Err(format!("invalid spec mapping {}", s.file));
            }
        }
        Ok(())
    }
}

/// RFC4180 fields, including embedded commas, CRLF, quotes and newlines. Reject
/// malformed quoting rather than quietly dropping a legacy spec's provenance.
pub fn csv_rows(input: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut closed = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if field.is_empty() && !closed => quoted = true,
                ',' => {
                    row.push(std::mem::take(&mut field));
                    closed = false;
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    closed = false;
                }
                '\r' if chars.peek() == Some(&'\n') => {}
                '"' => return Err("quote in unquoted CSV field".into()),
                _ if closed => return Err("characters after CSV quote".into()),
                _ => field.push(c),
            }
        }
    }
    if quoted {
        return Err("unterminated CSV quote".into());
    }
    if !field.is_empty() || !row.is_empty() || closed {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}
