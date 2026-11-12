//! plan_check / draft / revise / finalize. Pure: bytes in, bytes out, no I/O.
use crate::bind::{bind, diagnostic_counts, hex_sha256, slot_style, BoundCard, BoundTarget};
use crate::card::{Card, Defaults, Kind, Severity};
use crate::error::{Error, Result};
use crate::expr::*;
use oh_document::{parse_contract, Package};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

// ---- plan ----
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// Project record key (D11). The host loads the record into `facts`; mapped slots that
    /// are not given are filled from it with a `memory:` source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub documents: Vec<DocPlan>,
    /// Facts of the project record, filled by the host (never part of the JSON contract).
    #[serde(skip)]
    pub facts: BTreeMap<String, Fact>,
}
/// One confirmed project fact and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub value: String,
    pub source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocPlan {
    pub card: String,
    #[serde(default)]
    pub slots: BTreeMap<String, PlanSlot>,
    #[serde(default)]
    pub overrides: Vec<Override>,
    /// Bytes of `file` slots, filled by the host (never part of the JSON contract).
    #[serde(skip)]
    pub files: BTreeMap<String, Vec<u8>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSlot {
    pub value: Value,
    pub source: Option<String>,
}
/// A user decision to proceed despite a failed rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub rule: String,
    pub decision: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    /// Draft revision this patch was written against.
    pub revision: u32,
    pub documents: Vec<DocPatch>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocPatch {
    pub index: usize,
    #[serde(default)]
    pub slots: BTreeMap<String, PlanSlot>,
    #[serde(default)]
    pub remove: Vec<String>,
    #[serde(default)]
    pub overrides: Vec<Override>,
}

// ---- reports ----
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub code: String,
    pub doc: usize,
    pub id: Option<String>,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct SlotRow {
    pub id: String,
    pub label: String,
    pub value: Option<String>,
    pub source: Option<String>,
    /// provided | default | unresolved | invalid (a value was given but the core rejected it)
    pub status: &'static str,
    /// invalid only: why the core rejected the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// invalid only: the accepted format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    pub required: bool,
    pub style_color: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CalcRow {
    pub id: String,
    pub label: String,
    pub expr: String,
    pub computed: Option<String>,
    pub stated: Option<String>,
    /// ok | mismatch | computed | skipped
    pub status: &'static str,
}
#[derive(Debug, Clone, Serialize)]
pub struct RuleRow {
    pub id: String,
    pub severity: &'static str,
    /// pass | fail | not_applicable
    pub status: &'static str,
    pub missing: Vec<String>,
    pub message: Option<String>,
    pub cite: Option<String>,
    pub overridden: bool,
    pub decision: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct DefaultRow {
    pub slot: String,
    pub label: String,
    pub value: String,
    pub style_color: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct DocReport {
    pub card: String,
    pub slots: Vec<SlotRow>,
    pub calcs: Vec<CalcRow>,
    pub rules: Vec<RuleRow>,
    pub defaults: Vec<DefaultRow>,
    pub unresolved: Vec<String>,
    pub missing_required: Vec<String>,
    pub warnings: Vec<Issue>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// No errors: the plan can be drafted.
    pub ok: bool,
    /// Same as `!ok`, spelled out so a caller cannot read an invalid slot as merely unresolved.
    pub has_errors: bool,
    /// ok and no un-overridden block rule failure: finalize will accept it.
    pub finalizable: bool,
    pub errors: Vec<Issue>,
    pub blockers: Vec<Issue>,
    pub documents: Vec<DocReport>,
    pub next: Vec<String>,
}

struct Resolved {
    value: String,
    source: String,
    is_default: bool,
}
struct Eval {
    report: DocReport,
    errors: Vec<Issue>,
    blockers: Vec<Issue>,
    resolved: BTreeMap<String, Resolved>,
    calc_values: BTreeMap<String, String>,
}

// ---- engine ----
pub struct Engine {
    pub defaults: Defaults,
    cards: BTreeMap<String, BoundCard>,
    regulations: BTreeMap<String, crate::regulation::Regulation>,
}

/// Validate a list slot value; returns canonical JSON.
fn normalize_list(slot: &crate::card::Slot, v: &Value) -> std::result::Result<String, String> {
    let Value::Array(items) = v else { return Err("배열이 아님".into()) };
    if items.len() > slot.max_items.unwrap_or(200) {
        return Err(format!("항목 {}개 초과", slot.max_items.unwrap_or(200)));
    }
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let Value::Object(o) = it else { return Err(format!("{}번 항목이 객체가 아님", i + 1)) };
        let mut m = serde_json::Map::new();
        for k in o.keys() {
            if !slot.fields.iter().any(|f| &f.id == k) {
                return Err(format!("{}번 항목: 모르는 필드 {k}", i + 1));
            }
        }
        for f in &slot.fields {
            let raw = match o.get(&f.id) {
                None | Some(Value::Null) => None,
                Some(Value::String(x)) => Some(x.clone()),
                Some(Value::Number(n)) => Some(n.to_string()),
                Some(_) => return Err(format!("{}번 항목 {}: 문자열 또는 숫자", i + 1, f.id)),
            };
            let Some(raw) = raw.filter(|x| !x.trim().is_empty()) else {
                if f.required {
                    return Err(format!("{}번 항목: {} 필수", i + 1, f.id));
                }
                continue;
            };
            // multi-line prose fields keep their line breaks
            let val = if f.kind == Kind::Prose {
                let t: Vec<String> = raw.lines().map(|l| l.trim_end().to_string()).filter(|l| !l.trim().is_empty()).collect();
                if t.iter().any(|l| l.chars().any(|c| c.is_control())) {
                    None
                } else {
                    Some(t.join("\n"))
                }
            } else {
                normalize(f.kind, &raw)
            };
            let Some(val) = val else { return Err(format!("{}번 항목 {}: {:?} 형식이 아님", i + 1, f.id, f.kind)) };
            if val.chars().count() > f.max_chars.unwrap_or(usize::MAX) {
                return Err(format!("{}번 항목 {}: 최대 {}자", i + 1, f.id, f.max_chars.unwrap_or(0)));
            }
            if !f.one_of.is_empty() && !f.one_of.contains(&val) {
                return Err(format!("{}번 항목 {}: {} 중 하나", i + 1, f.id, f.one_of.join("|")));
            }
            m.insert(f.id.clone(), Value::String(val));
        }
        out.push(Value::Object(m));
    }
    Ok(Value::Array(out).to_string())
}

fn issue(code: &str, doc: usize, id: Option<&str>, detail: impl Into<String>) -> Issue {
    Issue { code: code.into(), doc, id: id.map(String::from), detail: detail.into() }
}

fn normalize(kind: Kind, raw: &str) -> Option<String> {
    let v = raw.trim();
    match kind {
        Kind::Money | Kind::Decimal => parse_decimal(v).map(|d| d.to_string()),
        Kind::Date => parse_date(v).map(|_| v.to_string()),
        Kind::Email => (v.matches('@').count() == 1 && !v.starts_with('@') && !v.ends_with('@') && !v.chars().any(|c| c.is_whitespace() || c.is_control())).then(|| v.to_string()),
        Kind::Phone => (v.chars().filter(|c| c.is_ascii_digit()).count() >= 5 && v.chars().all(|c| c.is_ascii_digit() || "+-() ".contains(c))).then(|| v.to_string()),
        Kind::Text | Kind::Person | Kind::Prose | Kind::File => (!v.is_empty() && !v.chars().any(|c| c.is_control())).then(|| v.to_string()),
        Kind::List => None,
    }
}

fn expected_format(s: &crate::card::Slot) -> String {
    match s.kind {
        Kind::Money => "숫자만(예: 3200000 또는 3200000.50). 통화 단위·문자 없이, 천단위 쉼표는 허용".into(),
        Kind::Decimal => "숫자만(예: 1360.3)".into(),
        Kind::Date => "날짜 YYYY-MM-DD 형식(예: 2026-09-07)".into(),
        Kind::Email => "이메일 주소 하나(예: name@example.com)".into(),
        Kind::Phone => "숫자 5자리 이상, 숫자와 + - ( ) 공백만".into(),
        Kind::Text | Kind::Person => "한 줄 문자열(줄바꿈·제어문자 없음)".into(),
        Kind::Prose => format!("서술 문장, 최대 {}자, 금지 문구 없음", s.max_chars.map(|n| n.to_string()).unwrap_or_else(|| "제한 없음".into())),
        Kind::File => "작업 폴더 inbox의 파일 이름(예: ledger.xlsx)".into(),
        Kind::List => format!(
            "객체 배열(최대 {}개). 필드: {}",
            s.max_items.map(|n| n.to_string()).unwrap_or_else(|| "제한 없음".into()),
            s.fields.iter().map(|f| format!("{}({:?}{}{})", f.id, f.kind, if f.required { ",필수" } else { "" }, if f.one_of.is_empty() { String::new() } else { format!(",{}", f.one_of.join("|")) })).collect::<Vec<_>>().join(", ")
        ),
    }
}

impl Engine {
    /// `cards`: parsed card plus the template bytes it names (already read by the host).
    pub fn new(defaults: Defaults, cards: Vec<(Card, Vec<u8>)>) -> Result<Engine> {
        let mut map = BTreeMap::new();
        for (c, t) in cards {
            let b = bind(c, t, &defaults)?;
            if map.insert(b.card.id.clone(), b).is_some() {
                return Err(Error::new("duplicate_id", "card id"));
            }
        }
        Ok(Engine { defaults, cards: map, regulations: BTreeMap::new() })
    }
    /// Register a regulation table (replaces one with the same id).
    pub fn set_regulation(&mut self, r: crate::regulation::Regulation) {
        self.regulations.insert(r.id.clone(), r);
    }
    pub fn regulation(&self, id: &str) -> Option<&crate::regulation::Regulation> {
        self.regulations.get(id)
    }
    pub fn card(&self, id: &str) -> Option<&Card> {
        self.cards.get(id).map(|b| &b.card)
    }
    pub fn card_ids(&self) -> Vec<String> {
        self.cards.keys().cloned().collect()
    }
    pub fn bound(&self, id: &str) -> Option<&BoundCard> {
        self.cards.get(id)
    }

    fn default_of(&self, bc: &BoundCard, slot: &str) -> Option<(String, Option<String>)> {
        let s = bc.card.slot(slot)?;
        let v = s.default.clone().or_else(|| s.default_ref.as_ref().and_then(|r| self.defaults.defaults.get(r)).map(|d| d.value.clone()))?;
        Some((v, slot_style(&bc.card, &self.defaults, slot).map(|s| s.color)))
    }

    fn eval(&self, bc: &BoundCard, dp: &DocPlan, di: usize, carry_in: Option<&BTreeMap<String, (String, bool)>>, project: Option<(&str, &BTreeMap<String, Fact>)>) -> Eval {
        let card = &bc.card;
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut resolved: BTreeMap<String, Resolved> = BTreeMap::new();
        for (id, ps) in &dp.slots {
            if card.calcs.iter().any(|c| &c.id == id) {
                errors.push(issue("computed_slot_not_writable", di, Some(id), "계산 슬롯은 코어가 계산한다"));
                continue;
            }
            let Some(slot) = card.slot(id) else {
                errors.push(issue("unknown_slot", di, Some(id), format!("카드 {}에 없는 슬롯", card.id)));
                continue;
            };
            let value = if slot.kind == Kind::List {
                if ps.value.as_array().is_some_and(|a| a.is_empty()) {
                    warnings.push(issue("empty_value", di, Some(id), "빈 목록은 없는 것으로 취급"));
                    continue;
                }
                match normalize_list(slot, &ps.value) {
                    Ok(v) => v,
                    Err(d) => {
                        errors.push(issue("invalid_value", di, Some(id), d));
                        continue;
                    }
                }
            } else {
                let raw = match &ps.value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => {
                        errors.push(issue("invalid_value", di, Some(id), "문자열 또는 숫자만 허용"));
                        continue;
                    }
                };
                if raw.trim().is_empty() {
                    warnings.push(issue("empty_value", di, Some(id), "빈 값은 없는 것으로 취급"));
                    continue;
                }
                let norm = if slot.kind == Kind::Prose && slot.multiline {
                    let t: Vec<&str> = raw.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
                    (!t.is_empty() && !t.iter().any(|l| l.chars().any(|c| c.is_control()))).then(|| t.join("\n"))
                } else {
                    normalize(slot.kind, &raw)
                };
                let Some(value) = norm else {
                    errors.push(issue("invalid_value", di, Some(id), format!("{:?} 형식이 아님", slot.kind)));
                    continue;
                };
                value
            };
            if slot.kind == Kind::File && !dp.files.contains_key(id) {
                errors.push(issue("file_not_loaded", di, Some(id), "첨부 파일을 읽지 못함(호스트가 inbox에서 읽는다)"));
                continue;
            }
            if slot.kind == Kind::Prose {
                if value.chars().count() > slot.max_chars.unwrap_or(usize::MAX) {
                    errors.push(issue("prose_too_long", di, Some(id), format!("최대 {}자", slot.max_chars.unwrap_or(0))));
                    continue;
                }
                if let Some(p) = slot.forbidden.iter().find(|p| value.contains(p.as_str())) {
                    errors.push(issue("prose_forbidden_phrase", di, Some(id), format!("금지 문구: {p}")));
                    continue;
                }
            }
            let source = ps.source.as_deref().map(str::trim).unwrap_or("").to_string();
            let mut is_default = false;
            match source.as_str() {
                "llm" if !(slot.kind == Kind::Prose || (slot.kind == Kind::List && slot.fields.iter().all(|f| f.kind == Kind::Prose))) => {
                    errors.push(issue("llm_source_not_allowed", di, Some(id), "llm 출처는 서술(prose) 슬롯 전용"));
                    continue;
                }
                "llm" if slot.require_source => {
                    errors.push(issue("llm_source_not_allowed", di, Some(id), "출처 필수 슬롯에 llm 출처 불가"));
                    continue;
                }
                "default" => match self.default_of(bc, id) {
                    Some((d, _)) if d == value => is_default = true,
                    _ => {
                        errors.push(issue("default_source_mismatch", di, Some(id), "기본값이 없거나 값이 기본값과 다름"));
                        continue;
                    }
                },
                "" if slot.require_source => {
                    errors.push(issue("source_required", di, Some(id), "출처가 필요한 슬롯"));
                    continue;
                }
                "" => warnings.push(issue("source_missing", di, Some(id), "출처 없음")),
                _ => {}
            }
            if source == "default" && slot.require_source {
                errors.push(issue("source_required", di, Some(id), "기본값은 출처로 인정되지 않음"));
                continue;
            }
            resolved.insert(id.clone(), Resolved { value, source, is_default });
        }
        // defaults
        for s in &card.slots {
            if !resolved.contains_key(&s.id) && !errors.iter().any(|e| e.id.as_deref() == Some(&s.id)) {
                if let Some((v, _)) = self.default_of(bc, &s.id) {
                    resolved.insert(s.id.clone(), Resolved { value: v, source: "default".into(), is_default: true });
                }
            }
        }
        // carry from an earlier document of the plan (C6)
        if let (Some(c), Some(prev)) = (&card.carry, carry_in) {
            for (here, there) in &c.map {
                let Some((v, was_default)) = prev.get(there).filter(|(v, _)| !v.is_empty()) else { continue };
                // an explicit value that differs from the earlier document is kept but flagged
                if let Some(r) = resolved.get(here).filter(|_| dp.slots.contains_key(here)) {
                    let same = match (parse_decimal(&r.value), parse_decimal(v)) {
                        (Some(a), Some(b)) => a == b,
                        _ => r.value.trim() == v.trim(),
                    };
                    if !same {
                        warnings.push(issue("carry_mismatch", di, Some(here), format!("앞 문서({}.{there}) {v} / 입력 {} — 입력 값 사용", c.from_card, r.value)));
                    }
                    continue;
                }
                let ok = card.slot(here).is_some_and(|s| s.kind == Kind::List || normalize(s.kind, v).is_some());
                if ok {
                    // a default in the earlier document stays a marked default here (D6-2)
                    let source = if *was_default { "default".to_string() } else { format!("carry:{}.{there}", c.from_card) };
                    resolved.insert(here.clone(), Resolved { value: v.clone(), source, is_default: *was_default });
                }
            }
        }
        // project record (D11): explicit > carry > memory > default
        if let (Some(pm), Some((pkey, facts))) = (&card.project, project) {
            for (here, key) in &pm.map {
                let Some(f) = facts.get(key).filter(|f| !f.value.trim().is_empty()) else { continue };
                let Some(slot) = card.slot(here) else { continue };
                // a rejected explicit value stays reported as invalid; memory does not paper over it
                if errors.iter().any(|e| e.id.as_deref() == Some(here.as_str())) {
                    continue;
                }
                let Some(v) = (if slot.kind == Kind::List { Some(f.value.clone()) } else { normalize(slot.kind, &f.value) }) else {
                    warnings.push(issue("project_fact_invalid", di, Some(here), format!("사업 기록 {pkey}.{key} = {} 형식이 맞지 않아 쓰지 않는다", f.value)));
                    continue;
                };
                match resolved.get(here) {
                    Some(r) if !r.is_default => {
                        let same = match (parse_decimal(&r.value), parse_decimal(&v)) {
                            (Some(a), Some(b)) => a == b,
                            _ => r.value.trim() == v.trim(),
                        };
                        if !same {
                            warnings.push(issue("project_fact_mismatch", di, Some(here), format!("사업 기록 {pkey}.{key} = {v} ({}) / 이 문서 {} — 이 문서 값 사용. 같은 사업인데 값이 다르면 사용자에게 확인한다", f.source, r.value)));
                        }
                    }
                    _ => {
                        resolved.insert(here.clone(), Resolved { value: v, source: format!("memory:{pkey}.{key} ← {}", f.source), is_default: false });
                    }
                }
            }
        }
        // computed defaults (core), in slot order
        let reg = card.regulation.as_ref().and_then(|r| self.regulations.get(r));
        for s in &card.slots {
            if let (Some(src), false) = (&s.default_expr, resolved.contains_key(&s.id)) {
                if errors.iter().any(|e| e.id.as_deref() == Some(&s.id)) {
                    continue;
                }
                let Ok(e) = parse_expr(src) else { continue };
                let get = |id: &str| resolved.get(id).map(|r| r.value.clone());
                if let Some(v) = eval_val(&e, &Ctx { get: &get, reg }).and_then(|v| normalize(s.kind, &v)) {
                    resolved.insert(s.id.clone(), Resolved { value: v, source: "default".into(), is_default: true });
                }
            }
        }
        let mut slot_rows = Vec::new();
        let mut defaults = Vec::new();
        let mut unresolved = Vec::new();
        let mut missing_required = Vec::new();
        for s in &card.slots {
            let style = slot_style(&&bc.card, &self.defaults, &s.id).map(|x| x.color);
            match resolved.get(&s.id) {
                Some(r) => {
                    if r.is_default {
                        defaults.push(DefaultRow { slot: s.id.clone(), label: s.label.clone(), value: r.value.clone(), style_color: style.clone() });
                    }
                    slot_rows.push(SlotRow { id: s.id.clone(), label: s.label.clone(), value: Some(r.value.clone()), source: Some(r.source.clone()), status: if r.is_default { "default" } else { "provided" }, reason: None, expected: None, required: s.required, style_color: if r.is_default { style } else { None } });
                }
                None => {
                    unresolved.push(s.id.clone());
                    if s.required {
                        missing_required.push(s.id.clone());
                    }
                    let bad = errors.iter().find(|e| e.id.as_deref() == Some(&s.id));
                    let (status, reason, expected) = match bad {
                        Some(e) => ("invalid", Some(format!("{}: {}", e.code, e.detail)), Some(expected_format(s))),
                        None => ("unresolved", None, None),
                    };
                    slot_rows.push(SlotRow { id: s.id.clone(), label: s.label.clone(), value: None, source: None, status, reason, expected, required: s.required, style_color: None });
                }
            }
        }
        // calcs (document order; later calcs may use earlier ones)
        let mut calc_values: BTreeMap<String, String> = BTreeMap::new();
        let mut calc_rows = Vec::new();
        // xlsx extracts (C7): items are summed by the core and compared with the sheet subtotal
        for x in &card.extracts {
            let Some(bytes) = resolved.get(&x.from).and_then(|_| dp.files.get(&x.from)) else {
                calc_rows.push(CalcRow { id: x.id.clone(), label: x.label.clone(), expr: format!("xlsx:{}!{}", x.sheet, x.section), computed: None, stated: None, status: "skipped" });
                continue;
            };
            match crate::xlsx::section_sum(bytes, x) {
                Ok((sum, stated, n)) => {
                    let sum = crate::expr::round_krw(sum);
                    let st = stated.map(|d| crate::expr::round_krw(d).to_string());
                    let status = match &st {
                        Some(v) if parse_decimal(v) == Some(sum) => "ok",
                        Some(v) => {
                            warnings.push(issue("xlsx_subtotal_mismatch", di, Some(&x.id), format!("{} 항목 {n}개 합 {} / 시트 소계 {} — 항목 합을 쓴다", x.section, sum, v)));
                            "mismatch"
                        }
                        None => "computed",
                    };
                    calc_values.insert(x.id.clone(), sum.to_string());
                    calc_rows.push(CalcRow { id: x.id.clone(), label: x.label.clone(), expr: format!("xlsx:{}!{}", x.sheet, x.section), computed: Some(sum.to_string()), stated: st, status });
                }
                Err(e) => {
                    errors.push(issue("xlsx_read_failed", di, Some(&x.id), format!("{}: {}", e.code, e.detail)));
                    calc_rows.push(CalcRow { id: x.id.clone(), label: x.label.clone(), expr: format!("xlsx:{}!{}", x.sheet, x.section), computed: None, stated: None, status: "skipped" });
                }
            }
        }
        for (c, e) in &bc.calcs {
            let get = |id: &str| -> Option<String> { calc_values.get(id).cloned().or_else(|| resolved.get(id).map(|r| r.value.clone())) };
            let computed = eval_val(e, &Ctx { get: &get, reg });
            let stated = c.compare_to.as_ref().and_then(|t| resolved.get(t)).map(|r| r.value.clone());
            let status = match (&computed, &stated) {
                (None, _) => "skipped",
                (Some(_), None) => "computed",
                (Some(d), Some(s)) => {
                    if parse_decimal(s).is_some_and(|x| Some(x) == parse_decimal(d)) {
                        "ok"
                    } else {
                        warnings.push(issue("calc_mismatch", di, Some(&c.id), format!("계산 {} / 원문 {} — 원문 값 유지", d, s)));
                        "mismatch"
                    }
                }
            };
            if let Some(d) = &computed {
                calc_values.insert(c.id.clone(), d.clone());
            }
            calc_rows.push(CalcRow { id: c.id.clone(), label: c.label.clone(), expr: c.expr.clone(), computed, stated, status });
        }
        // rules
        let lookup = |id: &str| -> Option<String> { resolved.get(id).map(|r| r.value.clone()).or_else(|| calc_values.get(id).cloned()) };
        let mut rule_rows = Vec::new();
        let mut blockers = Vec::new();
        for ov in &dp.overrides {
            if !card.rules.iter().any(|r| r.id == ov.rule) {
                errors.push(issue("unknown_rule_override", di, Some(&ov.rule), "카드에 없는 룰"));
            } else if ov.decision.trim().is_empty() {
                errors.push(issue("override_decision_missing", di, Some(&ov.rule), "무시 결정 내용이 비어 있음"));
            }
        }
        for br in &bc.rules {
            let r = &br.rule;
            let ov = dp.overrides.iter().find(|o| o.rule == r.id && !o.decision.trim().is_empty());
            let (status, missing) = if !eval_cond(&br.when, &lookup) {
                ("not_applicable", Vec::new())
            } else {
                let missing: Vec<String> = r.require.iter().filter(|x| lookup(x).is_none_or(|v| v.is_empty())).cloned().collect();
                let asrt_ok = br.assert.as_ref().is_none_or(|a| eval_cond(a, &lookup));
                if missing.is_empty() && asrt_ok { ("pass", missing) } else { ("fail", missing) }
            };
            let sev = match r.severity {
                Severity::Block => "block",
                Severity::Warn => "warn",
            };
            if status == "fail" {
                let detail = format!("{}{}", r.message.clone().unwrap_or_default(), if missing.is_empty() { String::new() } else { format!(" (빠짐: {})", missing.join(", ")) });
                match (r.severity, ov) {
                    (Severity::Block, None) => blockers.push(issue("rule_block_failed", di, Some(&r.id), detail)),
                    (Severity::Warn, _) => warnings.push(issue("rule_warn", di, Some(&r.id), detail)),
                    _ => {}
                }
            }
            rule_rows.push(RuleRow { id: r.id.clone(), severity: sev, status, missing, message: r.message.clone(), cite: r.cite.clone(), overridden: status == "fail" && ov.is_some(), decision: ov.map(|o| o.decision.clone()) });
        }
        Eval {
            report: DocReport { card: card.id.clone(), slots: slot_rows, calcs: calc_rows, rules: rule_rows, defaults, unresolved, missing_required, warnings },
            errors,
            blockers,
            resolved,
            calc_values,
        }
    }

    fn eval_plan(&self, plan: &Plan) -> (Report, Vec<Option<Eval>>) {
        let mut errors = Vec::new();
        let mut blockers = Vec::new();
        let mut docs = Vec::new();
        let mut evals = Vec::new();
        let mut next: Vec<String> = Vec::new();
        for (i, dp) in plan.documents.iter().enumerate() {
            let Some(bc) = self.cards.get(&dp.card) else {
                errors.push(issue("unknown_card", i, Some(&dp.card), "라이브러리에 없는 카드"));
                evals.push(None);
                continue;
            };
            for f in &bc.card.follows {
                if !plan.documents[..i].iter().any(|d| &d.card == f) {
                    errors.push(issue("dependency_missing", i, Some(f), format!("{}는 {} 뒤에 와야 함", bc.card.id, f)));
                }
            }
            for n in &bc.card.next {
                if !plan.documents.iter().any(|d| &d.card == n) && !next.contains(n) {
                    next.push(n.clone());
                }
            }
            let carry_in: Option<BTreeMap<String, (String, bool)>> = bc.card.carry.as_ref().and_then(|c| {
                (0..i).rev().find(|j| plan.documents[*j].card == c.from_card).and_then(|j| evals[j].as_ref()).map(|e: &Eval| {
                    let mut m: BTreeMap<String, (String, bool)> = e.resolved.iter().map(|(k, r)| (k.clone(), (r.value.clone(), r.is_default))).collect();
                    m.extend(e.calc_values.iter().map(|(k, v)| (k.clone(), (v.clone(), false))));
                    m
                })
            });
            let project = plan.project.as_deref().map(|k| (k, &plan.facts));
            let mut ev = self.eval(bc, dp, i, carry_in.as_ref(), project);
            errors.append(&mut ev.errors);
            blockers.append(&mut ev.blockers);
            docs.push(ev.report.clone());
            evals.push(Some(ev));
        }
        if plan.documents.is_empty() {
            errors.push(issue("empty_plan", 0, None, "문서가 없음"));
        }
        let ok = errors.is_empty();
        let finalizable = ok && blockers.is_empty();
        (Report { ok, has_errors: !ok, finalizable, errors, blockers, documents: docs, next }, evals)
    }

    pub fn plan_check(&self, plan: &Plan) -> Report {
        self.eval_plan(plan).0
    }

    pub fn draft(&self, plan: &Plan) -> Result<Draft> {
        self.build(plan.clone(), 1)
    }

    fn build(&self, plan: Plan, revision: u32) -> Result<Draft> {
        let (report, mut evals) = self.eval_plan(&plan);
        if !report.ok {
            let codes: Vec<String> = report.errors.iter().map(|e| format!("{}@{}{}", e.code, e.doc, e.id.as_ref().map(|i| format!(":{i}")).unwrap_or_default())).collect();
            return Err(Error::new("plan_invalid", codes.join(", ")));
        }
        let mut documents = Vec::new();
        for (i, dp) in plan.documents.iter().enumerate() {
            let ev = evals[i].take().expect("ok plan has eval");
            let bc = &self.cards[&dp.card];
            // a card that types "1. " right before a slot whose value starts with its own number
            // ("1. {related}" + "1. 우리 …") doubles it; checked before rendering, from the card itself
            if let Some(d) = value_numbered(bc, &ev) {
                return Err(Error::new("numbering_doubled", format!("{}: {d}", bc.card.id)));
            }
            let bytes = self.render(bc, &ev)?;
            // a number Hancom adds by itself plus a typed one shows as "1. 1." (generic check)
            let found = crate::numbering::doubled_numbering(&bytes);
            let (block, ambiguous): (Vec<_>, Vec<_>) = found.into_iter().partition(|i| i.blocking);
            if !block.is_empty() {
                return Err(Error::new("numbering_doubled", format!("{}: {}", bc.card.id, crate::numbering::describe(&block))));
            }
            if bc.card.xlsx.is_some() && bc.xlsx_template.is_none() {
                return Err(Error::new("xlsx_template_missing", bc.card.id.clone()));
            }
            let xlsx = self.render_xlsx(bc, &ev)?;
            let pkg = Package::from_bytes(bytes.clone()).map_err(|c| Error::new("output_unreadable", c))?;
            let now = diagnostic_counts(&pkg);
            let regress: Vec<String> = now.iter().filter(|(k, n)| **n > bc.baseline_diagnostics.get(*k).copied().unwrap_or(0)).map(|(k, _)| k.clone()).collect();
            let text: Vec<String> = pkg.inspection.text_nodes.iter().filter(|n| !n.text.trim().is_empty()).map(|n| n.text.clone()).collect();
            let overrides = dp.overrides.clone();
            let mut report = ev.report;
            // same number twice in date range ("1. 1."): a doubled number or a date; the LLM asks
            for it in &ambiguous {
                report.warnings.push(Issue { code: "numbering_ambiguous".into(), doc: i, id: None, detail: crate::numbering::describe(std::slice::from_ref(it)) });
            }
            let mut doc = DraftDoc { card: bc.card.id.clone(), title: bc.card.title.clone(), card_version: bc.card.version.clone(), bytes, xlsx, report, overrides, text, receipt: None };
            doc.receipt = Some(DocReceipt {
                template_sha256: bc.card.template_sha256.clone(),
                bytes_sha256: hex_sha256(&doc.bytes),
                xlsx_sha256: doc.xlsx.as_ref().map(|b| hex_sha256(b)),
                evidence_sha256: evidence_hash(&doc),
                structure_regressions: regress,
                structure_baseline: bc.baseline_diagnostics.clone(),
            });
            documents.push(doc);
        }
        Ok(Draft { revision, plan, documents })
    }

    fn render(&self, bc: &BoundCard, ev: &Eval) -> Result<Vec<u8>> {
        let get = |id: &str| -> Option<String> { ev.resolved.get(id).map(|r| r.value.clone()).or_else(|| ev.calc_values.get(id).cloned()) };
        let mut ops: Vec<Value> = Vec::new();
        // (node index, node id, new text, span start, end, color)
        let mut styles: Vec<(usize, String, String, usize, usize, String)> = Vec::new();
        for t in &bc.targets {
            let rendered = render_fmt(&t.fmt, &get);
            let (value, spans) = rendered.unwrap_or_default();
            let chunks = split_value(t, &value);
            for (n, chunk) in t.nodes.iter().zip(chunks) {
                let new = format!("{}{}", n.prefix, chunk);
                if new != n.template_text {
                    ops.push(json!({"op":"replace_text","target":n.node_id,"expected":n.template_text,"text":new}));
                }
                if t.nodes.len() == 1 {
                    for sp in &spans {
                        if let (Some(r), Some(color)) = (ev.resolved.get(&sp.slot), slot_style(&bc.card, &self.defaults, &sp.slot).map(|s| s.color)) {
                            if r.is_default && sp.end > sp.start {
                                let off = n.prefix.chars().count();
                                styles.push((n.node_index, n.node_id.clone(), new.clone(), sp.start + off, sp.end + off, color));
                            }
                        }
                    }
                }
            }
        }
        let mut cur = bc.template.clone();
        if !ops.is_empty() {
            let pkg = Package::from_bytes(cur.clone()).map_err(|c| Error::new("template_unreadable", c))?;
            let raw = serde_json::to_vec(&json!({"revision": pkg.inspection.revision, "operations": ops})).unwrap();
            let plan = parse_contract(&raw, "invalid_plan").map_err(|c| Error::new("render_failed", c))?;
            cur = pkg.apply_bytes(plan).map_err(|c| Error::new("render_failed", c))?.bytes;
        }
        // A formatted range splits its run, shifting later node ids: go from the end of the document.
        styles.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, node_id, expected, start, end, color) in styles {
            let pkg = Package::from_bytes(cur.clone()).map_err(|c| Error::new("render_failed", c))?;
            if !pkg.inspection.text_nodes.iter().any(|n| n.id == node_id && n.text == expected) {
                return Err(Error::new("style_target_drift", node_id));
            }
            let raw = serde_json::to_vec(&json!({"revision": pkg.inspection.revision, "target": node_id, "properties": {"textColor": color}, "range": {"start": start, "end": end}})).unwrap();
            let spec = parse_contract(&raw, "invalid_format_spec").map_err(|c| Error::new("style_failed", c))?;
            cur = pkg.format_bytes(spec).map_err(|c| Error::new("style_failed", c))?.bytes;
        }
        // repeated rows (C3): delete surplus template rows, clone for extra items
        let mut cell_ops: Vec<Value> = Vec::new();
        for (rt, cols) in &bc.rows {
            let items: Vec<serde_json::Map<String, Value>> = get(&rt.list)
                .and_then(|v| serde_json::from_str::<Vec<serde_json::Map<String, Value>>>(&v).ok())
                .unwrap_or_default();
            let n = items.len().max(1);
            let table = format!("Contents/section0.xml#tbl{}", rt.table);
            let mut tops: Vec<Value> = Vec::new();
            for k in (rt.first + n..rt.first + rt.template_rows).rev() {
                tops.push(json!({"op": "delete_row", "index": k}));
            }
            for k in rt.template_rows..n {
                tops.push(json!({"op": "clone_row", "source": rt.first, "index": rt.first + k}));
            }
            if !tops.is_empty() {
                let pkg = Package::from_bytes(cur.clone()).map_err(|c| Error::new("render_failed", c))?;
                let raw = serde_json::to_vec(&json!({"revision": pkg.inspection.revision, "target": table, "operations": tops})).unwrap();
                let spec = parse_contract(&raw, "invalid_table_spec").map_err(|c| Error::new("rows_failed", c))?;
                cur = pkg.table_bytes_scoped(spec).map_err(|c| Error::new("rows_failed", c))?.bytes;
            }
            for k in 0..n {
                let item = items.get(k);
                for (ci, f) in cols.iter().enumerate() {
                    let iget = |id: &str| -> Option<String> {
                        match id.strip_prefix("item.") {
                            Some(fld) => item.and_then(|o| o.get(fld)).and_then(|v| v.as_str().map(String::from)),
                            None => get(id),
                        }
                    };
                    let text = render_fmt(f, &iget).map(|x| x.0).unwrap_or_default();
                    let lines: Vec<String> = if text.is_empty() { vec![] } else { text.split('\n').map(String::from).collect() };
                    cell_ops.push(json!({"table": table, "row": rt.first + k, "col": ci, "lines": lines}));
                }
            }
        }
        for (c, f) in &bc.cells {
            let text = match render_fmt(f, &get) {
                Some((t, _)) => t,
                None if c.keep_if_unresolved => continue,
                None => String::new(),
            };
            let lines: Vec<String> = if text.is_empty() { vec![] } else { text.split('\n').map(String::from).collect() };
            cell_ops.push(json!({"table": format!("Contents/section0.xml#tbl{}", c.table.unwrap_or(0)), "row": c.row, "col": c.col, "lines": lines}));
        }
        if !cell_ops.is_empty() {
            let pkg = Package::from_bytes(cur.clone()).map_err(|c| Error::new("render_failed", c))?;
            let raw = serde_json::to_vec(&json!({"revision": pkg.inspection.revision, "operations": cell_ops})).unwrap();
            let spec = parse_contract(&raw, "invalid_cell_text_spec").map_err(|c| Error::new("cells_failed", c))?;
            cur = pkg.cell_text_bytes(spec).map_err(|c| Error::new("cells_failed", c))?.bytes;
        }
        // shortened paragraphs keep line caches that point past their text: Hancom refuses those
        let pkg = Package::from_bytes(cur.clone()).map_err(|c| Error::new("render_failed", c))?;
        if let Some(p) = pkg.drop_stale_linesegs().map_err(|c| Error::new("render_failed", c))? {
            cur = p.bytes;
        }
        Ok(cur)
    }

    /// Second output (C8): the card's workbook with slot/calc values written in.
    fn render_xlsx(&self, bc: &BoundCard, ev: &Eval) -> Result<Option<Vec<u8>>> {
        let (Some(x), Some(t)) = (&bc.card.xlsx, &bc.xlsx_template) else { return Ok(None) };
        let get = |id: &str| -> Option<String> { ev.resolved.get(id).map(|r| r.value.clone()).or_else(|| ev.calc_values.get(id).cloned()) };
        let mut cells = Vec::new();
        for (c, f) in &bc.xlsx_cells {
            let Some((text, _)) = render_fmt(f, &get) else { continue };
            if c.number {
                let d = parse_decimal(&text).ok_or_else(|| Error::new("xlsx_not_number", c.at.clone()))?;
                cells.push((c.at.clone(), crate::xlsx::CellValue::Number(d)));
            } else {
                cells.push((c.at.clone(), crate::xlsx::CellValue::Text(text)));
            }
        }
        crate::xlsx::write_cells(t, &x.sheet, &cells).map(Some)
    }

    /// Attach the workbook template named by a card's `xlsx` (hash-checked).
    pub fn attach_xlsx_template(&mut self, card: &str, bytes: Vec<u8>) -> Result<()> {
        let bc = self.cards.get_mut(card).ok_or_else(|| Error::new("unknown_card", card))?;
        let x = bc.card.xlsx.as_ref().ok_or_else(|| Error::new("invalid_card", "no xlsx output"))?;
        if hex_sha256(&bytes) != x.template_sha256 {
            return Err(Error::new("template_hash_mismatch", x.template.clone()));
        }
        crate::xlsx::write_cells(&bytes, &x.sheet, &[]).map_err(|e| Error::new("template_unreadable", e.detail))?;
        bc.xlsx_template = Some(bytes);
        Ok(())
    }

    /// Rebuild a patched plan (from `patched_plan`) as the next revision of `draft`.
    pub fn redraft(&self, draft: &Draft, plan: Plan) -> Result<Draft> {
        self.build(plan, draft.revision + 1)
    }
    /// The draft's plan with the patch applied. `files` are dropped: the host must reload
    /// file slots (a changed name or a re-uploaded file must not reuse old bytes).
    pub fn patched_plan(&self, draft: &Draft, patch: &Patch) -> Result<Plan> {
        if patch.revision != draft.revision {
            return Err(Error::new("stale_draft", format!("draft is r{}, patch is for r{}", draft.revision, patch.revision)));
        }
        let mut plan = draft.plan.clone();
        for d in plan.documents.iter_mut() {
            d.files.clear();
        }
        for p in &patch.documents {
            let d = plan.documents.get_mut(p.index).ok_or_else(|| Error::new("unknown_document", p.index.to_string()))?;
            for r in &p.remove {
                d.slots.remove(r);
            }
            for (k, v) in &p.slots {
                d.slots.insert(k.clone(), v.clone());
            }
            for o in &p.overrides {
                d.overrides.retain(|x| x.rule != o.rule);
                d.overrides.push(o.clone());
            }
        }
        Ok(plan)
    }
    /// Pure-core revise: keeps the draft's file bytes (callers that read files use
    /// `patched_plan` + reload + `redraft`).
    pub fn revise(&self, draft: &Draft, patch: &Patch) -> Result<Draft> {
        if patch.revision != draft.revision {
            return Err(Error::new("stale_draft", format!("draft is r{}, patch is for r{}", draft.revision, patch.revision)));
        }
        let mut plan = draft.plan.clone();
        for p in &patch.documents {
            let d = plan.documents.get_mut(p.index).ok_or_else(|| Error::new("unknown_document", p.index.to_string()))?;
            for r in &p.remove {
                d.slots.remove(r);
            }
            for (k, v) in &p.slots {
                d.slots.insert(k.clone(), v.clone());
            }
            for o in &p.overrides {
                d.overrides.retain(|x| x.rule != o.rule);
                d.overrides.push(o.clone());
            }
        }
        self.build(plan, draft.revision + 1)
    }
}


fn split_value(t: &BoundTarget, value: &str) -> Vec<String> {
    let n = t.nodes.len();
    let mut out = Vec::new();
    let mut rest: Vec<char> = value.chars().collect();
    for i in 0..n {
        if i + 1 == n || rest.len() <= t.wrap_width.unwrap_or(usize::MAX) {
            out.push(rest.iter().collect());
            rest.clear();
            continue;
        }
        let w = t.wrap_width.unwrap();
        let cut = (0..=w.min(rest.len() - 1)).rev().find(|k| rest[*k] == ' ').map(|k| k + 1).unwrap_or(w);
        out.push(rest[..cut].iter().collect());
        let tail: String = rest[cut..].iter().collect();
        rest = format!("{}{}", t.continuation_indent, tail).chars().collect();
    }
    out
}

// ---- draft / receipt / finalize ----
#[derive(Debug, Clone, Serialize)]
pub struct DocReceipt {
    pub template_sha256: String,
    pub bytes_sha256: String,
    pub xlsx_sha256: Option<String>,
    pub evidence_sha256: String,
    pub structure_regressions: Vec<String>,
    pub structure_baseline: BTreeMap<String, usize>,
}
#[derive(Debug, Clone)]
pub struct DraftDoc {
    pub card: String,
    pub title: String,
    pub card_version: String,
    pub bytes: Vec<u8>,
    /// Second output (XLSX) when the card has one.
    pub xlsx: Option<Vec<u8>>,
    pub report: DocReport,
    pub overrides: Vec<Override>,
    /// Non-empty document text nodes in order (for the review packet).
    pub text: Vec<String>,
    /// Issued only by the engine; finalize refuses a document without a matching one.
    pub receipt: Option<DocReceipt>,
}
#[derive(Debug, Clone)]
pub struct Draft {
    pub revision: u32,
    pub plan: Plan,
    pub documents: Vec<DraftDoc>,
}

fn evidence_hash(d: &DraftDoc) -> String {
    let v = json!({"report": d.report, "overrides": d.overrides, "card": d.card, "card_version": d.card_version});
    hex_sha256(&serde_json::to_vec(&v).unwrap())
}

/// Drop null fields from a JSON object (token slimming; absent = null).
fn drop_null(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(_, x)| !x.is_null()).collect()),
        o => o,
    }
}

/// Rules that did not pass (`fail`, overridden ones included) with message/cite, plus the
/// number of passed and not-applicable rules. Full rows stay in the receipt file.
pub fn slim_rules(rules: &[RuleRow]) -> (Vec<Value>, usize, usize) {
    let failed = rules
        .iter()
        .filter(|r| r.status == "fail")
        .map(|r| {
            let mut v = json!({"id": r.id, "severity": r.severity, "message": r.message, "cite": r.cite});
            if !r.missing.is_empty() {
                v["missing"] = json!(r.missing);
            }
            if r.overridden {
                v["overridden"] = json!(true);
                v["decision"] = json!(r.decision);
            }
            drop_null(v)
        })
        .collect();
    let n = |st: &str| rules.iter().filter(|r| r.status == st).count();
    (failed, n("pass"), n("not_applicable"))
}

pub fn slim_defaults(rows: &[DefaultRow]) -> Vec<Value> {
    rows.iter().map(|d| drop_null(json!({"slot": d.slot, "label": d.label, "value": d.value, "style_color": d.style_color}))).collect()
}

impl DocReport {
    /// Calc results as `{id: computed}` plus the rows that are not a clean `ok`/`computed`
    /// (stated-value mismatch, skipped) with their stated value.
    fn slim_calcs(&self) -> (Value, Vec<Value>) {
        let map: serde_json::Map<String, Value> = self.calcs.iter().map(|c| (c.id.clone(), json!(c.computed))).collect();
        let issues = self
            .calcs
            .iter()
            .filter(|c| c.status == "mismatch" || c.status == "skipped")
            .map(|c| drop_null(json!({"id": c.id, "status": c.status, "computed": c.computed, "stated": c.stated})))
            .collect();
        (Value::Object(map), issues)
    }

    /// plan_check view of one document: provided slots as id/value only, everything else
    /// (unresolved, invalid, default) with the fields needed to act on it.
    pub fn slim_check(&self) -> Value {
        let slots: Vec<Value> = self
            .slots
            .iter()
            .map(|s| {
                if s.status == "provided" {
                    // values the core filled itself (project record, earlier document) say so
                    match s.source.as_deref().filter(|x| x.starts_with("memory:") || x.starts_with("carry:")) {
                        Some(src) => json!({"id": s.id, "value": s.value, "source": src}),
                        None => json!({"id": s.id, "value": s.value}),
                    }
                } else {
                    let mut v = json!({"id": s.id, "label": s.label, "status": s.status, "required": s.required, "value": s.value, "reason": s.reason, "expected": s.expected});
                    if s.status == "default" {
                        v["style_color"] = json!(s.style_color);
                    } else {
                        v["source"] = json!(s.source);
                    }
                    drop_null(v)
                }
            })
            .collect();
        let (calcs, calc_issues) = self.slim_calcs();
        let (rules_failed, passed, na) = slim_rules(&self.rules);
        let mut v = json!({
            "card": self.card, "slots": slots, "calcs": calcs, "rules_failed": rules_failed, "rules_passed": passed, "rules_not_applicable": na,
            "defaults": slim_defaults(&self.defaults), "unresolved": self.unresolved, "missing_required": self.missing_required, "warnings": self.warnings,
        });
        if !calc_issues.is_empty() {
            v["calc_issues"] = json!(calc_issues);
        }
        v
    }
}

impl Report {
    /// plan_check view: verdict keys unchanged, per-document detail slimmed.
    pub fn slim(&self) -> Value {
        json!({
            "ok": self.ok, "has_errors": self.has_errors, "finalizable": self.finalizable,
            "errors": self.errors, "blockers": self.blockers, "next": self.next,
            "documents": self.documents.iter().map(DocReport::slim_check).collect::<Vec<_>>(),
        })
    }
}

impl Draft {
    /// What the LLM reviewer sees: text, calc results, rules that did not pass, defaults,
    /// unresolved, warnings. No XML, no slot table (the LLM sent those values and they are in
    /// the text), no passed rules. Full rows stay in the receipt file.
    pub fn review_packet(&self) -> Value {
        json!({
            "revision": self.revision,
            "documents": self.documents.iter().enumerate().map(|(i, d)| {
                let (calcs, calc_issues) = d.report.slim_calcs();
                let (rules_failed, passed, na) = slim_rules(&d.report.rules);
                let mut v = json!({
                    "index": i, "card": d.card, "title": d.title,
                    "text": d.text,
                    "calcs": calcs,
                    "rules_failed": rules_failed, "rules_passed": passed, "rules_not_applicable": na,
                    "defaults_applied": slim_defaults(&d.report.defaults),
                    "unresolved": d.report.unresolved,
                    "missing_required": d.report.missing_required,
                    "warnings": d.report.warnings,
                });
                if !calc_issues.is_empty() {
                    v["calc_issues"] = json!(calc_issues);
                }
                if !d.overrides.is_empty() {
                    v["overrides"] = json!(d.overrides);
                }
                let mem: Vec<Value> = d.report.slots.iter().filter(|s| s.source.as_deref().is_some_and(|x| x.starts_with("memory:"))).map(|s| json!({"id": s.id, "value": s.value, "source": s.source})).collect();
                if !mem.is_empty() {
                    v["from_project_record"] = json!(mem);
                }
                v
            }).collect::<Vec<_>>()
        })
    }
}

#[derive(Debug)]
pub struct Finalized {
    pub bytes: Vec<u8>,
    pub xlsx: Option<Vec<u8>>,
    pub receipt: Value,
}

/// A source that may go into the project record: a document position or `user` — not empty,
/// not `llm`/`default`, not a value read from the record itself (`memory...`).
pub fn fact_source_ok(src: &str) -> bool {
    let s = src.trim().to_lowercase();
    !s.is_empty() && s != "llm" && s != "default" && !s.starts_with("memory")
}

impl Engine {
    /// Facts to write back to the project record after `finalize` (D11): mapped slots whose
    /// value was provided or carried — never defaults (D6-2) and never values read from the record.
    pub fn project_facts(&self, draft: &Draft, index: usize) -> Vec<(String, Fact)> {
        let Some(doc) = draft.documents.get(index) else { return vec![] };
        let Some(pm) = self.card(&doc.card).and_then(|c| c.project.clone()) else { return vec![] };
        let mut out = Vec::new();
        for row in &doc.report.slots {
            let (Some(key), Some(v), Some(src)) = (pm.map.get(&row.id), &row.value, &row.source) else { continue };
            if row.status != "provided" || !fact_source_ok(src) {
                continue;
            }
            // a value that disagrees with the record is not written back silently: the user confirms
            // it and saves with oh_project_save (shares may legitimately differ per document)
            if doc.report.warnings.iter().any(|w| w.code == "project_fact_mismatch" && w.id.as_deref() == Some(row.id.as_str())) {
                continue;
            }
            out.push((key.clone(), Fact { value: v.clone(), source: format!("{src} [{}]", doc.card) }));
        }
        out
    }
}

/// Accept a draft document only with an engine receipt that matches its bytes and evidence.
pub fn finalize(draft: &Draft, index: usize) -> Result<Finalized> {
    let d = draft.documents.get(index).ok_or_else(|| Error::new("unknown_document", index.to_string()))?;
    let r = d.receipt.as_ref().ok_or_else(|| Error::new("missing_receipt", "코어 영수증 없는 draft는 finalize 불가"))?;
    if hex_sha256(&d.bytes) != r.bytes_sha256 {
        return Err(Error::new("receipt_mismatch", "bytes가 영수증과 다름"));
    }
    if d.xlsx.as_ref().map(|b| hex_sha256(b)) != r.xlsx_sha256 {
        return Err(Error::new("receipt_mismatch", "xlsx가 영수증과 다름"));
    }
    if evidence_hash(d) != r.evidence_sha256 {
        return Err(Error::new("receipt_mismatch", "슬롯·계산·룰 기록이 영수증과 다름"));
    }
    if !r.structure_regressions.is_empty() {
        return Err(Error::new("structure_regression", r.structure_regressions.join(", ")));
    }
    let failed: Vec<&str> = d.report.rules.iter().filter(|x| x.severity == "block" && x.status == "fail" && !x.overridden).map(|x| x.id.as_str()).collect();
    if !failed.is_empty() {
        return Err(Error::new("block_rule_failed", failed.join(", ")));
    }
    let receipt = json!({
        "card": d.card, "card_version": d.card_version, "revision": draft.revision,
        "template_sha256": r.template_sha256, "output_sha256": r.bytes_sha256, "output_bytes": d.bytes.len(),
        "defaults_applied": d.report.defaults,
        "unresolved": d.report.unresolved,
        "missing_required": d.report.missing_required,
        "rules": d.report.rules,
        "overrides": d.overrides,
        "calc_warnings": d.report.warnings.iter().filter(|w| w.code == "calc_mismatch").collect::<Vec<_>>(),
        "warnings": d.report.warnings,
        "structure": {"regressions_vs_template": r.structure_regressions, "baseline_diagnostics": r.structure_baseline, "native_validated": false},
        "note": "코어 구조 검사 영수증이며 한컴 육안 검수를 대신하지 않는다",
    });
    let mut receipt = receipt;
    if let Some(x) = &r.xlsx_sha256 {
        receipt["xlsx_sha256"] = json!(x);
    }
    Ok(Finalized { bytes: d.bytes.clone(), xlsx: d.xlsx.clone(), receipt })
}

/// Slot values that start with a list marker where the card's fmt already typed one right before
/// the slot ("1. {related}" with related = "1. 우리 …"). Filters that renumber lines (num, ganada,
/// attach, nobullet) strip markers themselves and are skipped.
fn value_numbered(bc: &BoundCard, ev: &Eval) -> Option<String> {
    let fmts = bc.card.targets.iter().map(|t| t.fmt.as_str()).chain(bc.card.cells.iter().map(|c| c.fmt.as_str()));
    for f in fmts {
        let mut rest = f;
        while let Some(open) = rest.find('{') {
            let before = &rest[..open];
            let line = before.rsplit('\n').next().unwrap_or(before).trim_start_matches('[');
            let Some(close) = rest[open..].find('}') else { break };
            let inner = &rest[open + 1..open + close];
            let (slot, filter) = inner.split_once('|').map(|(a, b)| (a, Some(b))).unwrap_or((inner, None));
            let renumbers = filter.is_some_and(|x| ["num", "ganada", "attach", "nobullet"].contains(&x));
            // an optional "[…]" group is written only when all its slots resolve
            let group_open = before.rfind('[').is_some_and(|o| !before[o..].contains(']'));
            let group_ok = !group_open || {
                let end = rest[open..].find(']').map(|e| open + e).unwrap_or(rest.len());
                rest[before.rfind('[').unwrap_or(0)..end]
                    .split('{')
                    .skip(1)
                    .filter_map(|x| x.split('}').next())
                    .all(|x| ev.resolved.contains_key(x.split('|').next().unwrap_or("").trim()))
            };
            if group_ok && !line.is_empty() && !renumbers && crate::numbering::typed_marker_len(line) == Some(line.len()) {
                if let Some(v) = ev.resolved.get(slot.trim()) {
                    // a value that is itself a date ("3. 2. 회의") is not a number
                    if crate::numbering::typed_marker_len(&v.value).is_some() && !crate::numbering::starts_with_date(&v.value) {
                        return Some(format!(
                            "카드가 \"{}\" 뒤에 쓰는 {} 값이 번호로 시작한다(\"{}\"). 값에서 앞 번호를 뺀다",
                            line.trim(),
                            slot.trim(),
                            v.value.chars().take(20).collect::<String>()
                        ));
                    }
                }
            }
            rest = &rest[open + close + 1..];
        }
    }
    None
}
