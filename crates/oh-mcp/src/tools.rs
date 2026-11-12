//! The twelve MCP tools. Nothing here edits XML/ZIP, tables or styles, and nothing sends,
//! signs or approves anything (D4). Output goes to `outbox` with no-clobber only.
use crate::register::{self, Registration};
use crate::{xlsx, Config, Server, PROMPT};
use oh_library::Stack;
use oh_recipe::{finalize, hex_sha256, slim_defaults, slim_rules, Draft, Engine, Fact, Patch, Plan};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

// `oh_calibrate` (DESIGN P6 calibration set) is not part of the 1.0 surface (D14).
const NAMES: [&str; 12] = [
    "oh_guide", "oh_library_search", "oh_card_get", "oh_case_open", "oh_plan_check", "oh_draft",
    "oh_revise", "oh_finalize", "oh_project_get", "oh_project_save", "oh_library_register", "oh_library_commit",
];
pub fn tool_names() -> &'static [&'static str] {
    &NAMES
}

const MAX_OPEN: usize = 200;
const MAX_ATTACH: usize = 20;
const TEXT_CAP: usize = 40_000;
const CLIENTS: [&str; 4] = ["claude-desktop", "chatgpt-desktop", "codex", "claude-code"];

pub struct ToolErr {
    code: String,
    detail: Option<String>,
    extra: Option<Value>,
}
impl ToolErr {
    pub fn to_json(&self) -> Value {
        let mut v = json!({"ok": false, "error": self.code, "message": ko(&self.code)});
        if let Some(d) = &self.detail {
            v["detail"] = json!(d);
        }
        if let Some(e) = &self.extra {
            v["info"] = e.clone();
        }
        v
    }
}
fn err(code: &str, detail: impl Into<String>) -> ToolErr {
    let d: String = detail.into();
    ToolErr { code: code.into(), detail: (!d.is_empty()).then_some(d), extra: None }
}
fn ko(code: &str) -> &'static str {
    match code {
        "source_locked" => "첨부 파일을 다른 프로그램이 잠그고 있다. 그 프로그램에서 파일을 닫고 다시 시도한다",
        "input_io_error" => "첨부 파일을 읽지 못했다. 경로와 권한을 확인한다",
        "input_too_large" => "파일이 너무 크다",
        "target_exists" => "같은 이름의 출력이 이미 있다. 다른 출력 이름을 쓴다(덮어쓰지 않는다)",
        "target_locked" => "같은 이름의 출력을 다른 프로그램이 열어 두고 있다. 닫거나 다른 이름을 쓴다",
        "output_io_error" => "출력 폴더에 쓰지 못했다. 작업 폴더 권한을 확인한다(oh doctor)",
        "workdir_unavailable" => "작업 폴더를 만들거나 쓸 수 없다. oh doctor로 점검한다",
        "library_unavailable" => "라이브러리를 불러오지 못했다. oh doctor로 점검하고 필요하면 다시 설치한다",
        "invalid_arguments" => "도구 인수가 올바르지 않다",
        "invalid_plan" => "plan 형식이 올바르지 않다(null·중복 키·알 수 없는 필드 불가)",
        "invalid_patch" => "patch 형식이 올바르지 않다",
        "unknown_case" => "알 수 없는 case id. oh_case_open부터 한다",
        "unknown_draft" => "알 수 없는 draft id. oh_draft부터 한다",
        "unknown_card" => "알 수 없는 카드 id. oh_library_search로 찾는다",
        "too_many_open" => "열린 작업이 너무 많다. 서버를 다시 시작한다",
        "invalid_output_name" => "출력 이름이 올바르지 않다(경로 구분자·예약 문자·예약 이름 불가)",
        "document_index_required" => "문서가 둘 이상이다. document 번호를 지정한다",
        "unsupported_attachment_path" => "첨부는 inbox 안의 상대 경로이거나 절대 경로여야 한다",
        "invalid_xlsx" | "invalid_attachment" => "첨부 문서를 해석하지 못했다",
        "unknown_registration" => "알 수 없는 registration id. oh_library_register에 template부터 준다",
        "unsupported_template" => "등록할 양식은 HWPX만 받는다. HWP는 한글에서 다른 이름으로 저장 → HWPX로 바꾼다",
        "invalid_template" => "양식 HWPX를 해석하지 못했다",
        "not_validated" => "아직 검증한 카드 초안이 없다. oh_library_register에 registration_id·card·sample을 준다",
        "registration_not_ready" => "마지막 검증이 needs_fix다. 문제를 고친 초안으로 다시 검증한다",
        "user_confirmation_required" => "사용자가 카드 요약과 저장 위치를 확인한 뒤에만 저장한다(user_confirmed=true)",
        "decision_required" => "검증에 사용자 결정이 필요한 차이가 있다. 사용자가 받아들였으면 accept_differences=true",
        "card_exists" => "같은 id 카드가 이미 있다. 사용자가 바꾸기로 했으면 replace=true(개인 카드는 백업 후 교체)",
        "personal_library_invalid" => "개인 라이브러리 manifest가 맞지 않아 고치지 않았다. oh doctor로 점검한다",
        "personal_library_write_failed" => "개인 라이브러리(설치 폴더의 personal-library)에 쓰지 못해 이전 상태로 되돌렸다. 폴더 권한·백신·동기화 프로그램이 파일을 잡고 있는지 확인한다",
        "commit_verification_failed" => "저장 뒤 다시 읽기 검사에 실패해 이전 상태로 되돌렸다",
        "plan_invalid" => "plan에 오류가 있어 작성하지 못했다. oh_plan_check 결과를 보고 고친다",
        "stale_draft" => "draft 개정 번호가 낡았다. 최신 revision으로 patch한다",
        "missing_receipt" | "receipt_mismatch" | "structure_regression" => "코어 영수증 검사에 실패했다",
        "block_rule_failed" => "block 룰이 실패했다. 사용자 결정(overrides)이 필요하다",
        "forbidden_operation" => "이 동작은 허용되지 않는다(D4)",
        "invalid_project_key" => "사업 키가 올바르지 않다(영문·숫자·한글·-_ 1~64자, 예: 사업코드)",
        "unknown_project" => "그 사업 기록이 없다. oh_project_get으로 목록을 본다",
        "invalid_fact" => "사실 값·출처가 올바르지 않다(출처는 문서 위치나 user. llm·default·memory 불가)",
        "project_record_corrupt" => "사업 기록 파일을 읽지 못했다. 작업 폴더 projects/의 파일을 확인한다",
        _ => "요청을 처리하지 못했다",
    }
}

/// True if any JSON object in `raw` repeats a key. serde maps silently keep the last value, so
/// the slot map needs its own check.
fn has_duplicate_keys(raw: &str) -> bool {
    use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
    struct Any(bool);
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = bool;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("json")
        }
        fn visit_unit<E>(self) -> Result<bool, E> { Ok(false) }
        fn visit_bool<E>(self, _: bool) -> Result<bool, E> { Ok(false) }
        fn visit_i64<E>(self, _: i64) -> Result<bool, E> { Ok(false) }
        fn visit_u64<E>(self, _: u64) -> Result<bool, E> { Ok(false) }
        fn visit_f64<E>(self, _: f64) -> Result<bool, E> { Ok(false) }
        fn visit_str<E>(self, _: &str) -> Result<bool, E> { Ok(false) }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<bool, A::Error> {
            let mut dup = false;
            while let Some(Any(d)) = a.next_element()? {
                dup |= d;
            }
            Ok(dup)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<bool, A::Error> {
            let mut seen = std::collections::HashSet::new();
            let mut dup = false;
            while let Some(k) = a.next_key::<String>()? {
                dup |= !seen.insert(k);
                let Any(d) = a.next_value()?;
                dup |= d;
            }
            Ok(dup)
        }
    }
    impl<'de> serde::Deserialize<'de> for Any {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Any, D::Error> {
            d.deserialize_any(V).map(Any)
        }
    }
    serde_json::from_str::<Any>(raw).map(|a| a.0).unwrap_or(true)
}

fn contract<T: serde::de::DeserializeOwned>(raw: Option<&String>, code: &'static str) -> Result<T, ToolErr> {
    let raw = raw.ok_or_else(|| err("invalid_arguments", format!("{} 필요", &code[8..])))?;
    if has_duplicate_keys(raw) {
        return Err(err(code, "중복 키"));
    }
    oh_document::parse_contract(raw.as_bytes(), code).map_err(|c| err(c, ""))
}

/// Raw JSON text of `plan`/`patch`, re-read from the request line so the strict contract
/// parser (no nulls, no duplicate keys) sees the client's exact tokens.
#[derive(Default)]
pub struct RawArgs {
    plan: Option<String>,
    patch: Option<String>,
}
impl RawArgs {
    pub fn from_line(line: &[u8]) -> RawArgs {
        #[derive(Deserialize)]
        struct A {
            plan: Option<Box<serde_json::value::RawValue>>,
            patch: Option<Box<serde_json::value::RawValue>>,
        }
        #[derive(Deserialize)]
        struct P {
            arguments: Option<A>,
        }
        #[derive(Deserialize)]
        struct R {
            params: Option<P>,
        }
        match serde_json::from_slice::<R>(line).ok().and_then(|r| r.params).and_then(|p| p.arguments) {
            Some(a) => RawArgs { plan: a.plan.map(|r| r.get().to_string()), patch: a.patch.map(|r| r.get().to_string()) },
            None => RawArgs::default(),
        }
    }
}

struct Loaded {
    /// personal > connected packs > base (D12 socket)
    stack: Stack,
    engine: Engine,
    /// Which regulation tables are in use (library sample or the company table in the workdir).
    regulations: Vec<String>,
}
struct CaseEntry {
    dir: PathBuf,
}
struct DraftEntry {
    case: String,
    draft: Draft,
}
pub struct State {
    loaded: Result<Loaded, String>,
    cases: BTreeMap<String, CaseEntry>,
    drafts: BTreeMap<String, DraftEntry>,
    registrations: BTreeMap<String, Registration>,
    counter: u64,
}

impl State {
    pub fn load(cfg: &Config) -> State {
        let loaded = Stack::load_installed(&cfg.library)
            .map_err(|e| e.to_string())
            .and_then(|stack| {
                stack
                    .engine_with_local(Some(&cfg.workdir.join("regulations")))
                    .map(|(engine, regulations)| Loaded { stack, engine, regulations })
                    .map_err(|e| e.to_string())
            });
        State { loaded, cases: BTreeMap::new(), drafts: BTreeMap::new(), registrations: BTreeMap::new(), counter: 0 }
    }
}

fn civil(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}
pub fn now_stamp() -> String {
    civil(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

/// Output file name: one path component, no reserved characters or device names.
pub fn safe_output_name(raw: &str) -> Result<String, ToolErr> {
    let bad = || err("invalid_output_name", "");
    let n = raw.trim();
    if n.is_empty() || n.chars().count() > 120 || n.starts_with('.') || n.ends_with('.') {
        return Err(bad());
    }
    if n.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)) {
        return Err(bad());
    }
    let stem = n.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0');
    if reserved {
        return Err(bad());
    }
    // Exactly one lowercase `.hwpx` suffix, whatever case the client used.
    let base = if n.to_ascii_lowercase().ends_with(".hwpx") { &n[..n.len() - 5] } else { n };
    if base.is_empty() || base.ends_with('.') || base.ends_with(' ') {
        return Err(bad());
    }
    Ok(format!("{base}.hwpx"))
}

/// Attachment path: absolute (explicit) or a clean relative path inside `inbox`.
fn resolve_attachment(inbox: &Path, raw: &str) -> Result<PathBuf, ToolErr> {
    let p = Path::new(raw);
    if raw.is_empty() || raw.contains('\0') {
        return Err(err("unsupported_attachment_path", raw));
    }
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    if p.components().all(|c| matches!(c, Component::Normal(_))) && !raw.contains(':') {
        return Ok(inbox.join(p));
    }
    Err(err("unsupported_attachment_path", raw))
}

fn only(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), ToolErr> {
    match args.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(err("invalid_arguments", format!("알 수 없는 인수: {k}"))),
        None => Ok(()),
    }
}
fn req_str<'a>(args: &'a Map<String, Value>, key: &str, max: usize) -> Result<&'a str, ToolErr> {
    match args.get(key).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() && s.chars().count() <= max && !s.contains('\0') => Ok(s),
        _ => Err(err("invalid_arguments", format!("{key}: 1~{max}자 문자열 필요"))),
    }
}
fn opt_str<'a>(args: &'a Map<String, Value>, key: &str, max: usize) -> Result<Option<&'a str>, ToolErr> {
    if args.contains_key(key) { req_str(args, key, max).map(Some) } else { Ok(None) }
}

/// Drop null fields and empty arrays, recursively (token slimming for card_get; absent = null/empty).
fn prune(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, x)| (k, prune(x))).filter(|(_, x)| !x.is_null() && x.as_array().is_none_or(|a| !a.is_empty())).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(prune).collect()),
        o => o,
    }
}

fn rerr(e: oh_recipe::Error) -> ToolErr {
    err(e.code, e.detail)
}
fn file_stem(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

/// Project record key: one safe path component (D11).
fn project_key(raw: &str) -> Result<String, ToolErr> {
    let k = raw.trim();
    let ok = !k.is_empty() && k.chars().count() <= 64 && !k.starts_with('.') && k.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if ok { Ok(k.to_string()) } else { Err(err("invalid_project_key", raw)) }
}
/// `projects/<key>.json`: {"project", "updated", "facts": {key: {value, source, updated}}}
#[derive(serde::Serialize, Deserialize, Default)]
struct ProjectRecord {
    project: String,
    updated: String,
    facts: BTreeMap<String, StoredFact>,
}
#[derive(serde::Serialize, Deserialize, Clone)]
struct StoredFact {
    value: String,
    source: String,
    updated: String,
}
const FACT_KEYS: [&str; 11] = ["project_name", "project_code", "client", "dept", "our_company", "partner_name", "our_share_pct", "contract_amount_usd", "contract_date", "contract_start", "contract_end"];

impl Server {
    fn project_path(&self, key: &str) -> Result<PathBuf, ToolErr> {
        Ok(self.dir("projects")?.join(format!("{key}.json")))
    }
    fn read_project(&self, key: &str) -> Result<Option<ProjectRecord>, ToolErr> {
        let p = self.project_path(key)?;
        if !p.is_file() {
            return Ok(None);
        }
        let b = oh_host::bounded_read(&p, 1024 * 1024).map_err(|c| err(c, p.display().to_string()))?;
        serde_json::from_slice(&b).map(Some).map_err(|_| err("project_record_corrupt", p.display().to_string()))
    }
    /// Merge facts into the record (newer value replaces, source kept per fact).
    fn save_facts(&self, key: &str, facts: &[(String, Fact)]) -> Result<Vec<String>, ToolErr> {
        if facts.is_empty() {
            return Ok(vec![]);
        }
        let mut rec = self.read_project(key)?.unwrap_or_else(|| ProjectRecord { project: key.to_string(), ..Default::default() });
        let now = now_stamp();
        let mut changed = Vec::new();
        for (k, f) in facts {
            let same = rec.facts.get(k).is_some_and(|x| x.value == f.value);
            if !same {
                changed.push(k.clone());
            }
            if !same || rec.facts.get(k).is_none() {
                rec.facts.insert(k.clone(), StoredFact { value: f.value.clone(), source: f.source.clone(), updated: now.clone() });
            }
        }
        if !changed.is_empty() {
            rec.updated = now;
            let p = self.project_path(key)?;
            let body = serde_json::to_vec_pretty(&rec).unwrap_or_default();
            oh_host::replace_file(&p, &body).map_err(|c| err(c, p.display().to_string()))?;
        }
        Ok(changed)
    }
    /// Fill `plan.facts` from the record named by `plan.project` (absent record = no facts).
    fn load_facts(&self, plan: &mut Plan) -> Result<Option<Value>, ToolErr> {
        let Some(raw) = plan.project.clone() else { return Ok(None) };
        let key = project_key(&raw)?;
        plan.project = Some(key.clone());
        plan.facts.clear();
        match self.read_project(&key)? {
            Some(rec) => {
                for (k, f) in rec.facts {
                    plan.facts.insert(k, Fact { value: f.value, source: f.source });
                }
                Ok(Some(json!({"project": key, "facts_loaded": plan.facts.len()})))
            }
            None => Ok(Some(json!({"project": key, "facts_loaded": 0, "note": "사업 기록 없음. 확정(finalize)하면 이 문서의 사업 정보로 새로 만든다"}))),
        }
    }
    fn loaded(&self) -> Result<&Loaded, ToolErr> {
        self.state.loaded.as_ref().map_err(|d| err("library_unavailable", d.clone()))
    }
    fn dir(&self, sub: &str) -> Result<PathBuf, ToolErr> {
        let d = self.cfg.workdir.join(sub);
        oh_host::ensure_dir(&d).map_err(|c| err(c, d.display().to_string()))?;
        Ok(d)
    }
    fn next_id(&mut self, prefix: &str) -> String {
        self.state.counter += 1;
        format!("{prefix}-{}-{}", now_stamp(), self.state.counter)
    }
    /// Read `file` slots (inbox name or absolute path) so the core can use their bytes.
    fn load_files(&self, plan: &mut Plan) -> Result<(), ToolErr> {
        let inbox = self.dir("inbox")?;
        let eng = &self.loaded()?.engine;
        for dp in plan.documents.iter_mut() {
            let Some(card) = eng.card(&dp.card) else { continue };
            for s in card.slots.iter().filter(|s| s.kind == oh_recipe::Kind::File) {
                let Some(name) = dp.slots.get(&s.id).and_then(|p| p.value.as_str()).map(str::trim).filter(|v| !v.is_empty()) else { continue };
                let path = resolve_attachment(&inbox, name)?;
                let bytes = oh_host::bounded_read(&path, 64 * 1024 * 1024).map_err(|c| err(c, name))?;
                dp.files.insert(s.id.clone(), bytes);
            }
        }
        Ok(())
    }
    fn case_dir(&self, id: &str) -> Result<PathBuf, ToolErr> {
        self.state.cases.get(id).map(|c| c.dir.clone()).ok_or_else(|| err("unknown_case", id))
    }

    pub(crate) fn call_tool(&mut self, name: &str, a: &Map<String, Value>, raw: &RawArgs) -> Result<Value, ToolErr> {
        match name {
            "oh_guide" => {
                only(a, &["client"])?;
                let client = opt_str(a, "client", 40)?;
                if let Some(c) = client.filter(|c| !CLIENTS.contains(c)) {
                    return Err(err("invalid_arguments", format!("client는 {CLIENTS:?} 중 하나: {c}")));
                }
                let cards: Vec<Value> = match &self.state.loaded {
                    Ok(l) => l.stack.cards().map(|cs| cs.into_iter().map(|(c, layer)| json!({"id": c.id, "title": c.title, "version": c.version, "triggers": c.triggers, "layer": layer})).collect()).unwrap_or_default(),
                    Err(_) => vec![],
                };
                Ok(json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "client": client,
                    "prompt": PROMPT,
                    "profile": Value::Null,
                    "cards": cards,
                    "library_loaded": self.state.loaded.is_ok(),
                    "regulations": self.state.loaded.as_ref().map(|l| l.regulations.clone()).unwrap_or_default(),
                    "library_layers": self.state.loaded.as_ref().map(|l| layers_json(&l.stack)).unwrap_or(Value::Null),
                    "work_folders": {
                        "inbox": self.cfg.workdir.join("inbox"), "work": self.cfg.workdir.join("work"), "outbox": self.cfg.workdir.join("outbox")
                    },
                }))
            }
            "oh_library_search" => {
                only(a, &["query", "kind", "limit"])?;
                let q = req_str(a, "query", 200)?.trim().to_string();
                let kind = opt_str(a, "kind", 20)?;
                if let Some(k) = kind.filter(|k| !["card", "regulation", "template", "example"].contains(k)) {
                    return Err(err("invalid_arguments", format!("kind: {k}")));
                }
                let limit = match a.get("limit") {
                    None => 5,
                    Some(v) => v.as_u64().filter(|n| (1..=20).contains(n)).ok_or_else(|| err("invalid_arguments", "limit: 1~20"))? as usize,
                };
                let l = self.loaded()?;
                let mut hits: Vec<Value> = Vec::new();
                if kind.is_none() || kind == Some("card") {
                    let mut qs = vec![q.clone()];
                    qs.extend(q.split_whitespace().map(str::to_string));
                    // order: strong specific, strong fallback, weak specific, weak fallback (each in
                    // discovery order: whole query first, then each word). Weak = the card shares only a
                    // generic document word (품의·서류·요청 …) with the request. So a generic card never
                    // goes before a card the request names (해외출장, 하도급, 보험료 …), and a generic
                    // request is not buried under cards that only share the word 품의.
                    const GENERIC_WORDS: [&str; 16] = ["품의", "품의서", "서류", "문서", "요청", "신청", "신청서", "결재", "작성", "기안", "건", "관련", "만들어줘", "써줘", "해줘", "올려줘"];
                    // every phase first; a card is strong if any phase matched it strongly (word order
                    // must not matter), then ordered by bucket and first discovery
                    let mut found: Vec<(Value, bool, bool)> = Vec::new(); // (hit, fallback, strong)
                    for (phase, s) in qs.into_iter().enumerate() {
                        let generic_word = phase > 0 && (GENERIC_WORDS.contains(&s.as_str()) || s.chars().count() < 2);
                        for h in l.stack.search(&s).map_err(|e| err("library_unavailable", e.to_string()))? {
                            let strong = h.strong || !generic_word;
                            match found.iter_mut().find(|(v, _, _)| v["id"] == json!(h.id)) {
                                Some(f) => f.2 |= strong,
                                None => {
                                    let mut v = json!({"kind": "card", "id": h.id, "title": h.title, "layer": h.layer});
                                    if h.fallback {
                                        v["fallback"] = json!(true);
                                    }
                                    found.push((v, h.fallback, strong));
                                }
                            }
                        }
                    }
                    let mut buckets: [Vec<Value>; 4] = Default::default();
                    for (v, fallback, strong) in found {
                        buckets[(!strong as usize) * 2 + fallback as usize].push(v);
                    }
                    for b in buckets {
                        hits.extend(b);
                    }
                }
                if kind != Some("card") {
                    for (layer, lib) in l.stack.libraries() {
                    for f in &lib.manifest.files {
                        let want = matches!(f.kind.as_str(), "regulation" | "template" | "example") && kind.map_or(f.kind == "regulation", |k| k == f.kind);
                        if !want {
                            continue;
                        }
                        let toks: Vec<&str> = q.split_whitespace().collect();
                        if f.kind == "regulation" {
                            let text = String::from_utf8_lossy(lib.file(&f.path).unwrap_or(&[])).to_string();
                            for (i, line) in text.lines().enumerate() {
                                if toks.iter().any(|t| line.contains(t)) {
                                    let snip: String = line.trim().chars().take(120).collect();
                                    hits.push(json!({"kind": "regulation", "id": f.path, "layer": layer, "line": i + 1, "snippet": snip}));
                                }
                            }
                        } else if toks.iter().any(|t| f.path.contains(t)) {
                            hits.push(json!({"kind": f.kind, "id": f.path, "layer": layer}));
                        }
                    }
                    }
                }
                hits.truncate(limit);
                let conflicts: Vec<Value> = l.stack.conflicts.iter().filter(|c| c.kind == "card" && hits.iter().any(|h| h["id"] == json!(c.id))).map(conflict_json).collect();
                Ok(prune(json!({"query": q, "search": l.stack.base().manifest.search, "hits": hits, "conflicts": conflicts})))
            }
            "oh_card_get" => {
                only(a, &["card"])?;
                let id = req_str(a, "card", 100)?;
                let c = self.loaded()?.engine.card(id).ok_or_else(|| err("unknown_card", id))?;
                let ds = &self.loaded()?.engine.defaults;
                let layer = self.loaded()?.stack.card_layer(id).map(String::from);
                Ok(prune(json!({
                    "id": c.id, "title": c.title, "version": c.version, "layer": layer, "triggers": c.triggers, "next": c.next, "follows": c.follows, "sources": c.sources,
                    "slots": c.slots.iter().map(|s| json!({
                        "id": s.id, "label": s.label, "kind": format!("{:?}", s.kind).to_lowercase(), "required": s.required, "require_source": s.require_source,
                        "default": s.default.clone().or_else(|| s.default_ref.as_ref().and_then(|r| ds.defaults.get(r)).map(|d| d.value.clone())),
                        "guide": s.guide, "max_chars": s.max_chars, "forbidden": s.forbidden,
                        "default_expr": s.default_expr, "multiline": s.multiline.then_some(true), "max_items": s.max_items,
                        "fields": (!s.fields.is_empty()).then(|| s.fields.iter().map(|f| json!({"id": f.id, "label": f.label, "kind": format!("{:?}", f.kind).to_lowercase(), "required": f.required, "max_chars": f.max_chars, "one_of": (!f.one_of.is_empty()).then(|| f.one_of.clone())})).collect::<Vec<_>>()),
                    })).collect::<Vec<_>>(),
                    "extracts": c.extracts.iter().map(|x| json!({"id": x.id, "label": x.label, "from": x.from, "note": "첨부 xlsx에서 코어가 읽는다"})).collect::<Vec<_>>(),
                    "carry": c.carry.as_ref().map(|x| json!({"from_card": x.from_card, "slots": x.map.keys().collect::<Vec<_>>(), "note": "같은 계획에 앞 문서가 있으면 코어가 가져온다"})),
                    "extends": c.extends,
                    "project": c.project.as_ref().map(|p| json!({"slots": p.map, "note": "plan에 project 키를 주면 빈 슬롯을 사업 기록에서 memory 출처로 채운다"})),
                    "outputs": if c.xlsx.is_some() { json!(["hwpx", "xlsx"]) } else { json!(["hwpx"]) },
                    "calcs": c.calcs.iter().map(|x| json!({"id": x.id, "label": x.label, "expr": x.expr, "note": "코어가 계산한다. 값을 넣지 않는다"})).collect::<Vec<_>>(),
                    "rules": c.rules.iter().map(|r| json!({"id": r.id, "severity": format!("{:?}", r.severity).to_lowercase(), "when": r.when, "require": r.require, "assert": r.assert, "message": r.message, "cite": r.cite})).collect::<Vec<_>>(),
                })))
            }
            "oh_case_open" => {
                only(a, &["summary", "attachments"])?;
                let summary = req_str(a, "summary", 2000)?;
                let list: Vec<&str> = match a.get("attachments") {
                    None => vec![],
                    Some(Value::Array(v)) if v.len() <= MAX_ATTACH => v.iter().map(|x| x.as_str().ok_or_else(|| err("invalid_arguments", "attachments: 문자열 배열"))).collect::<Result<_, _>>()?,
                    Some(_) => return Err(err("invalid_arguments", format!("attachments: 문자열 {MAX_ATTACH}개 이하 배열"))),
                };
                self.loaded()?;
                if self.state.cases.len() >= MAX_OPEN {
                    return Err(err("too_many_open", ""));
                }
                let inbox = self.dir("inbox")?;
                let mut out = Vec::new();
                for item in list {
                    let path = resolve_attachment(&inbox, item)?;
                    out.push(read_attachment(item, &path)?);
                }
                let id = self.next_id("case");
                let dir = self.dir("work")?.join(&id);
                oh_host::ensure_dir(&dir).map_err(|c| err(c, dir.display().to_string()))?;
                self.state.cases.insert(id.clone(), CaseEntry { dir: dir.clone() });
                Ok(json!({"case_id": id, "summary": summary, "work_dir": dir, "inbox": inbox, "attachments": out}))
            }
            "oh_plan_check" => {
                only(a, &["case_id", "plan"])?;
                self.case_dir(req_str(a, "case_id", 100)?)?;
                let mut plan: Plan = contract(raw.plan.as_ref(), "invalid_plan")?;
                self.load_files(&mut plan)?;
                let project = self.load_facts(&mut plan)?;
                let rep = self.loaded()?.engine.plan_check(&plan);
                // Top-level `ok` here means "the plan has no errors", not "the call ran": an invalid slot must not read as success.
                let problems: Vec<Value> = rep.errors.iter().map(|e| json!({"document": e.doc, "slot": e.id, "code": e.code, "detail": e.detail})).collect();
                let mut out = json!({"ok": !rep.has_errors, "has_errors": rep.has_errors, "error_count": rep.errors.len(), "report": rep.slim()});
                if let Some(p) = project {
                    out["project"] = p;
                }
                if rep.has_errors {
                    out["problems"] = json!(problems);
                    out["next_step"] = json!("report.errors·problems의 슬롯을 고쳐 oh_plan_check를 다시 호출한다. status가 invalid인 슬롯은 reason·expected를 따른다. 고치기 전에는 oh_draft가 거절한다.");
                }
                Ok(out)
            }
            "oh_draft" => {
                only(a, &["case_id", "plan"])?;
                let case = req_str(a, "case_id", 100)?.to_string();
                let cdir = self.case_dir(&case)?;
                let mut plan: Plan = contract(raw.plan.as_ref(), "invalid_plan")?;
                self.load_files(&mut plan)?;
                self.load_facts(&mut plan)?;
                if self.state.drafts.len() >= MAX_OPEN {
                    return Err(err("too_many_open", ""));
                }
                let eng = &self.loaded()?.engine;
                let finalizable = eng.plan_check(&plan).finalizable;
                let draft = eng.draft(&plan).map_err(rerr)?;
                let id = self.next_id("draft");
                let files = write_draft_files(&cdir, &id, &draft)?;
                let packet = draft.review_packet();
                let rev = draft.revision;
                self.state.drafts.insert(id.clone(), DraftEntry { case, draft });
                Ok(json!({"draft_id": id, "revision": rev, "files": files, "finalizable": finalizable, "review_packet": packet}))
            }
            "oh_revise" => {
                only(a, &["draft_id", "patch"])?;
                let id = req_str(a, "draft_id", 100)?.to_string();
                let patch: Patch = contract(raw.patch.as_ref(), "invalid_patch")?;
                let entry = self.state.drafts.get(&id).ok_or_else(|| err("unknown_draft", id.clone()))?;
                let cdir = self.case_dir(&entry.case)?;
                let mut plan = self.loaded()?.engine.patched_plan(&entry.draft, &patch).map_err(rerr)?;
                self.load_files(&mut plan)?;
                self.load_facts(&mut plan)?;
                let entry = self.state.drafts.get(&id).ok_or_else(|| err("unknown_draft", id.clone()))?;
                let new = self.loaded()?.engine.redraft(&entry.draft, plan).map_err(rerr)?;
                let files = write_draft_files(&cdir, &id, &new)?;
                let packet = new.review_packet();
                let rev = new.revision;
                self.state.drafts.get_mut(&id).expect("checked").draft = new;
                Ok(json!({"draft_id": id, "revision": rev, "files": files, "review_packet": packet}))
            }
            "oh_finalize" => {
                only(a, &["draft_id", "output_name", "document"])?;
                let id = req_str(a, "draft_id", 100)?;
                let name = safe_output_name(req_str(a, "output_name", 200)?)?;
                let entry = self.state.drafts.get(id).ok_or_else(|| err("unknown_draft", id))?;
                let n = entry.draft.documents.len();
                let index = match a.get("document") {
                    None if n == 1 => 0,
                    None => return Err(err("document_index_required", format!("문서 {n}개"))),
                    Some(v) => v.as_u64().filter(|i| (*i as usize) < n).ok_or_else(|| err("invalid_arguments", format!("document: 0~{}", n - 1)))? as usize,
                };
                let fin = finalize(&entry.draft, index).map_err(rerr)?;
                let cdir = self.case_dir(&entry.case)?;
                let outbox = self.dir("outbox")?;
                let target = outbox.join(&name);
                let stem = name.strip_suffix(".hwpx").unwrap_or(&name).to_string();
                let xlsx_target = fin.xlsx.as_ref().map(|_| outbox.join(format!("{stem}.xlsx")));
                if let Some(x) = &xlsx_target {
                    if x.exists() {
                        return Err(err("output_exists", x.display().to_string()));
                    }
                }
                oh_host::publish_no_clobber(&target, &fin.bytes).map_err(|c| err(c, name.clone()))?;
                if let (Some(x), Some(b)) = (&xlsx_target, &fin.xlsx) {
                    oh_host::publish_no_clobber(x, b).map_err(|c| err(c, x.display().to_string()))?;
                }
                let sha = hex_sha256(&fin.bytes);
                // Full receipt goes to a file in the case work folder; the response carries only what the LLM must report.
                let mut full = fin.receipt.clone();
                full["path"] = json!(target);
                if let Some(x) = &xlsx_target {
                    full["xlsx_path"] = json!(x);
                }
                full["sha256"] = json!(sha);
                let rfile = cdir.join(format!("{stem}.receipt.json"));
                let body = serde_json::to_vec_pretty(&full).unwrap_or_default();
                let receipt_file = oh_host::publish_no_clobber(&rfile, &body).map(|_| json!(rfile)).map_err(|c| json!(c));
                let doc = &entry.draft.documents[index];
                let (rules_failed, passed, na) = slim_rules(&doc.report.rules);
                let mut out = json!({
                    "path": target, "sha256": sha, "bytes": fin.bytes.len(), "xlsx_path": xlsx_target,
                    "card": doc.card, "revision": entry.draft.revision,
                    "unresolved": doc.report.unresolved, "missing_required": doc.report.missing_required,
                    "defaults_applied": slim_defaults(&doc.report.defaults), "warnings": doc.report.warnings,
                    "rules_failed": rules_failed, "rules_passed": passed, "rules_not_applicable": na,
                    "native_validated": fin.receipt["structure"]["native_validated"],
                    "regressions_vs_template": fin.receipt["structure"]["regressions_vs_template"],
                });
                if !doc.overrides.is_empty() {
                    out["overrides"] = json!(doc.overrides);
                }
                match receipt_file {
                    Ok(f) => out["receipt_file"] = f,
                    Err(c) => out["receipt_file_error"] = c,
                }
                // D11: confirmed project facts go back to the project record (never defaults)
                if let Some(key) = entry.draft.plan.project.clone() {
                    let facts = self.loaded()?.engine.project_facts(&entry.draft, index);
                    // reported as record fact keys (what oh_project_save takes), not slot ids
                    let pmap = self.loaded()?.engine.card(&doc.card).and_then(|c| c.project.clone()).map(|p| p.map).unwrap_or_default();
                    let held: Vec<String> = doc.report.warnings.iter().filter(|w| w.code == "project_fact_mismatch").filter_map(|w| w.id.as_ref()).map(|id| pmap.get(id).cloned().unwrap_or_else(|| id.clone())).collect();
                    match self.save_facts(&key, &facts) {
                        Ok(changed) => {
                            out["project_saved"] = json!({"project": key, "changed": changed});
                            if !held.is_empty() {
                                out["project_saved"]["not_saved_mismatch"] = json!(held);
                                out["project_saved"]["note"] = json!("기록과 다른 값은 저장하지 않았다. 사용자가 이 값을 사업 기준으로 확정하면 oh_project_save로 저장한다");
                            }
                        }
                        Err(e) => out["project_save_error"] = e.to_json(),
                    }
                }
                Ok(out)
            }
            "oh_project_get" => {
                only(a, &["project"])?;
                match opt_str(a, "project", 64)? {
                    Some(raw) => {
                        let key = project_key(raw)?;
                        let rec = self.read_project(&key)?.ok_or_else(|| err("unknown_project", key.clone()))?;
                        Ok(json!({"project": rec.project, "updated": rec.updated, "facts": rec.facts, "note": "plan에 \"project\": 키를 넣으면 코어가 빈 슬롯을 memory 출처로 채우고, 다른 값은 경고한다"}))
                    }
                    None => {
                        let dir = self.dir("projects")?;
                        let mut list = Vec::new();
                        for e in std::fs::read_dir(&dir).map_err(|_| err("workdir_unavailable", dir.display().to_string()))?.flatten() {
                            let name = e.file_name().to_string_lossy().to_string();
                            let Some(key) = name.strip_suffix(".json").filter(|k| project_key(k).is_ok()) else { continue };
                            if let Ok(Some(r)) = self.read_project(key) {
                                list.push(json!({"project": key, "project_name": r.facts.get("project_name").map(|f| f.value.clone()), "updated": r.updated, "facts": r.facts.len()}));
                            }
                        }
                        list.sort_by(|x, y| x["project"].as_str().cmp(&y["project"].as_str()));
                        list.truncate(50);
                        Ok(json!({"projects": list}))
                    }
                }
            }
            "oh_project_save" => {
                only(a, &["project", "facts"])?;
                let key = project_key(req_str(a, "project", 64)?)?;
                let Some(Value::Object(m)) = a.get("facts") else { return Err(err("invalid_arguments", "facts: {키: {value, source}}")) };
                if m.is_empty() || m.len() > 30 {
                    return Err(err("invalid_arguments", "facts: 1~30개"));
                }
                let mut facts = Vec::new();
                for (k, v) in m {
                    if !FACT_KEYS.contains(&k.as_str()) && !(k.starts_with("x_") && project_key(k).is_ok()) {
                        return Err(err("invalid_fact", format!("{k}: 키는 {FACT_KEYS:?} 또는 x_로 시작")));
                    }
                    let value = v.get("value").and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_number().map(|n| n.to_string()))).map(|x| x.trim().to_string()).filter(|x| !x.is_empty() && x.chars().count() <= 500);
                    let source = v.get("source").and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty() && x.chars().count() <= 300);
                    let (Some(value), Some(source)) = (value, source) else { return Err(err("invalid_fact", k.clone())) };
                    if !oh_recipe::fact_source_ok(source) {
                        return Err(err("invalid_fact", format!("{k}: 출처 {source} 불가")));
                    }
                    facts.push((k.clone(), Fact { value, source: source.to_string() }));
                }
                let changed = self.save_facts(&key, &facts)?;
                Ok(json!({"project": key, "changed": changed, "path": self.project_path(&key)?}))
            }
            "oh_library_register" => self.register(a),
            "oh_library_commit" => self.commit(a),
            _ => Err(err("forbidden_operation", name)),
        }
    }
}

impl Server {
    fn install_root(&self) -> PathBuf {
        self.cfg.library.parent().map(Path::to_path_buf).unwrap_or_default()
    }
    /// D10 step 1 (template) or step 3 (registration_id + card draft + sample).
    fn register(&mut self, a: &Map<String, Value>) -> Result<Value, ToolErr> {
        only(a, &["template", "registration_id", "card", "sample", "expect"])?;
        self.loaded()?;
        let personal = self.install_root().join(oh_library::PERSONAL_DIR);
        match (a.contains_key("template"), a.contains_key("registration_id")) {
            (true, false) => {
                if ["card", "sample", "expect"].iter().any(|k| a.contains_key(*k)) {
                    return Err(err("invalid_arguments", "첫 호출은 template만 준다"));
                }
                let name = req_str(a, "template", 500)?;
                if self.state.registrations.len() >= MAX_OPEN {
                    return Err(err("too_many_open", ""));
                }
                let inbox = self.dir("inbox")?;
                let path = resolve_attachment(&inbox, name)?;
                if !path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("hwpx")) {
                    return Err(err("unsupported_template", name));
                }
                let raw = oh_host::bounded_read(&path, oh_document::MAX_INPUT).map_err(|c| err(c, name))?;
                let (tpl, cleanup) = register::prepare(raw).map_err(|c| err("invalid_template", format!("{name}: {c}")))?;
                let structure = register::extract(&tpl).map_err(|c| err("invalid_template", format!("{name}: {c}")))?;
                let id = self.next_id("reg");
                let dir = self.dir("work")?.join(&id);
                oh_host::ensure_dir(&dir).map_err(|c| err(c, dir.display().to_string()))?;
                let _ = oh_host::publish_no_clobber(&dir.join("structure.json"), &serde_json::to_vec_pretty(&structure).unwrap_or_default());
                let sha = hex_sha256(&tpl);
                let source = Path::new(name).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
                self.state.registrations.insert(id.clone(), Registration { source, template: tpl, dir: dir.clone(), attempts: 0, last: None });
                Ok(json!({
                    "registration_id": id, "template": name, "sha256": sha, "cleanup": cleanup, "work_dir": dir,
                    "structure": structure, "card_format": register_guide(), "save_to": personal,
                    "next_step": "structure로 카드 초안(TOML)과 sample(원본 값)을 만들어 oh_library_register(registration_id, card, sample, expect)로 검증받는다. 저장은 아직 하지 않는다",
                }))
            }
            (false, true) => {
                let rid = req_str(a, "registration_id", 100)?.to_string();
                let card = req_str(a, "card", 60_000)?;
                let sample = match a.get("sample") {
                    Some(v @ Value::Object(_)) => v,
                    _ => return Err(err("invalid_arguments", "sample: {슬롯id: {value, source}} 객체(원본에 있는 값)")),
                };
                let expect = match a.get("expect") {
                    None => None,
                    Some(Value::Object(m)) if m.len() <= 100 => Some(m),
                    Some(_) => return Err(err("invalid_arguments", "expect: {계산id: \"원본에 적힌 값\"} 100개 이하")),
                };
                let l = self.loaded()?;
                let reg = self.state.registrations.get(&rid).ok_or_else(|| err("unknown_registration", rid.clone()))?;
                let (mut report, v) = register::validate(reg, &l.stack, &l.engine, &register::Input { card_toml: card, sample, expect });
                let n = reg.attempts + 1;
                let dir = reg.dir.clone();
                let _ = oh_host::replace_file(&dir.join(format!("card-{n}.toml")), v.card_toml.as_bytes());
                let _ = oh_host::replace_file(&dir.join(format!("check-{n}.json")), &serde_json::to_vec_pretty(&report).unwrap_or_default());
                report["registration_id"] = json!(rid);
                report["attempt"] = json!(n);
                report["save_to"] = json!(personal);
                let reg = self.state.registrations.get_mut(&rid).expect("checked");
                reg.attempts = n;
                reg.last = Some(v);
                Ok(report)
            }
            _ => Err(err("invalid_arguments", "template(첫 호출) 또는 registration_id+card+sample(검증) 중 하나")),
        }
    }
    /// D10 step 5: save the last validated card to the personal library (user confirmed).
    fn commit(&mut self, a: &Map<String, Value>) -> Result<Value, ToolErr> {
        only(a, &["registration_id", "user_confirmed", "accept_differences", "replace"])?;
        let rid = req_str(a, "registration_id", 100)?.to_string();
        let flag = |k: &str| -> Result<bool, ToolErr> {
            match a.get(k) {
                None => Ok(false),
                Some(Value::Bool(b)) => Ok(*b),
                Some(_) => Err(err("invalid_arguments", format!("{k}: true/false"))),
            }
        };
        let (confirmed, accept, replace) = (flag("user_confirmed")?, flag("accept_differences")?, flag("replace")?);
        let reg = self.state.registrations.get(&rid).ok_or_else(|| err("unknown_registration", rid.clone()))?;
        let v = reg.last.as_ref().ok_or_else(|| err("not_validated", rid.clone()))?;
        if !confirmed {
            return Err(err("user_confirmation_required", ""));
        }
        match v.status {
            "ready" => {}
            "needs_user_decision" if accept => {}
            "needs_user_decision" => return Err(ToolErr { code: "decision_required".into(), detail: None, extra: Some(json!(v.decisions)) }),
            _ => return Err(err("registration_not_ready", v.card_id.clone())),
        }
        if let Some(layer) = &v.existing_layer {
            if !replace {
                return Err(err("card_exists", format!("{} ({layer})", v.card_id)));
            }
        }
        let stamp = now_stamp();
        // commit checks the id again against the disk (another registration may have saved it since)
        let mut out = register::commit(&self.install_root(), &self.cfg.library, v, &reg.template, replace, &stamp).map_err(|(c, d)| err(&c, d))?;
        let card_id = v.card_id.clone();
        let accepted = (v.status == "needs_user_decision").then(|| v.decisions.clone());
        let replaced = out.get("backup").and_then(Value::as_array).filter(|b| !b.is_empty()).map(|_| json!("personal")).or_else(|| v.existing_layer.clone().map(Value::from));
        let dir = reg.dir.clone();
        // the running server picks the new card up at once; other server processes on restart
        self.state.loaded = State::load(&self.cfg).loaded;
        let layer = self.loaded()?.stack.card_layer(&card_id).map(String::from);
        if layer.as_deref() != Some("personal") {
            return Err(err("commit_verification_failed", format!("reload: {card_id} {layer:?}")));
        }
        out["replaced_layer"] = replaced.unwrap_or(Value::Null);
        if let Some(d) = accepted {
            out["accepted_differences"] = json!(d);
        }
        out["next_step"] = json!("이 카드로 바로 문서를 만들 수 있다(oh_card_get → oh_case_open → oh_plan_check → oh_draft → oh_finalize). 다른 클라이언트에서 쓰려면 그 클라이언트를 다시 시작한다");
        let _ = oh_host::replace_file(&dir.join("commit.json"), &serde_json::to_vec_pretty(&out).unwrap_or_default());
        self.state.registrations.remove(&rid);
        Ok(out)
    }
}

fn register_guide() -> String {
    format!(
        "카드 초안 = TOML 문자열(LIBRARY-GUIDE 3절·필드 참고와 같은 형식). template·template_sha256은 쓰지 않는다(코어가 사용자 파일로 넣는다). \
필수: id(영문 소문자·숫자·-), title, version, triggers. 머리 표가 품의서면 extends = \"approval-memo\"(공문이면 \"official-letter\")로 상속하고 본문만 선언한다. 상속하면 틀의 슬롯(작성일·부서·작성자·사업명·사업코드·제목 등)도 카드 슬롯이다. \
[[slot]]: id, label, kind(text·person·prose·money·decimal·date·email·phone·list), required, require_source(첨부·사용자 값), prose는 guide + max_chars 필수. \
위치: 라벨 줄은 [[target]] id·fmt·anchor = [{{ starts = \"라벨 : \" }}](label_lines의 unique=true만), 라벨 칸 옆 칸은 [[cell]] id·label·fmt(label_cells의 unique=true만), 그 밖의 칸은 [[cell]] id·table·row·col·fmt, 반복 행은 [[rows]]. 모든 target·cell에 fmt가 있어야 한다. \
fmt는 \"글자 {{슬롯id|필터}} 글자\", [ … ]는 안의 값이 없으면 통째로 빠진다. 필터: {filters}. 천 단위 쉼표는 usd(정수)·usd2(소수 둘째)·krw, 날짜는 date_kr(2026년 1월 2일)·date_dot(2026.01.02). \
합계·분담·환산·비율처럼 원본에서 계산된 숫자는 슬롯이 아니라 [[calc]] id·label·expr로 둔다(LLM이 값을 넣지 않는다). 계산 함수: {funcs}. 이 밖의 함수는 쓰지 않는다(unknown_function). \
sample = 원본에 적힌 값 그대로의 plan 슬롯({{\"슬롯id\": {{\"value\": \"…\", \"source\": \"원본 <위치>\"}}}}), 틀의 필수 슬롯 포함, 계산 슬롯은 넣지 않는다. expect = {{\"계산id\": \"원본에 적힌 값\"}}. \
코어는 앵커 유일성, 계산식 검산(expect), 왕복 시험(원본 값으로 다시 만든 문서 = 원본)을 확인한다. 저장은 사용자 확인 뒤 oh_library_commit. \
최소 예: id = \"my-memo\"\\ntitle = \"내 품의\"\\nversion = \"1.0.0\"\\nextends = \"approval-memo\"\\ntriggers = [\"내 품의\"]\\n\\n[[slot]]\\nid = \"fee\"\\nlabel = \"수수료(USD)\"\\nkind = \"money\"\\nrequired = true\\nrequire_source = true\\n\\n[[calc]]\\nid = \"total\"\\nlabel = \"합계\"\\nexpr = \"sum(fee, tax)\"\\n\\n[[cell]]\\nid = \"fee_cell\"\\ntable = 1\\nrow = 3\\ncol = 2\\nfmt = \"USD {{fee|usd}}\"",
        filters = oh_recipe::FILTERS.join(", "),
        funcs = register::GENERAL_FUNCS
    )
}

fn write_draft_files(case_dir: &Path, draft_id: &str, d: &Draft) -> Result<Vec<String>, ToolErr> {
    let mut files = Vec::new();
    for (i, doc) in d.documents.iter().enumerate() {
        let path = case_dir.join(format!("{draft_id}-{}-{}-r{}.hwpx", i + 1, file_stem(&doc.card), d.revision));
        oh_host::publish_no_clobber(&path, &doc.bytes).map_err(|c| err(c, ""))?;
        files.push(path.display().to_string());
        if let Some(x) = &doc.xlsx {
            let p = path.with_extension("xlsx");
            oh_host::publish_no_clobber(&p, x).map_err(|c| err(c, ""))?;
            files.push(p.display().to_string());
        }
    }
    Ok(files)
}

fn read_attachment(shown: &str, path: &Path) -> Result<Value, ToolErr> {
    let bytes = oh_host::bounded_read(path, oh_document::MAX_INPUT).map_err(|c| ToolErr { code: c.into(), detail: Some(shown.to_string()), extra: Some(json!({"attachment": shown})) })?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let fail = |c: &str| ToolErr { code: c.into(), detail: Some(shown.to_string()), extra: Some(json!({"attachment": shown})) };
    let (kind, text, truncated): (&str, Option<Vec<String>>, bool) = match ext.as_str() {
        "hwpx" => {
            let p = oh_document::Package::from_bytes(bytes.clone()).map_err(|c| fail(c))?;
            let mut used = 0usize;
            let mut out = Vec::new();
            let mut cut = false;
            for n in p.inspection.text_nodes.iter().filter(|n| !n.text.trim().is_empty()) {
                used += n.text.chars().count() + 1;
                if used > TEXT_CAP {
                    cut = true;
                    break;
                }
                out.push(n.text.clone());
            }
            ("hwpx", Some(out), cut)
        }
        "xlsx" => {
            let (lines, cut) = xlsx::extract(&bytes, TEXT_CAP).map_err(|c| fail(c))?;
            ("xlsx", Some(lines), cut)
        }
        "txt" | "md" | "csv" | "json" => {
            let s = String::from_utf8(bytes.clone()).map_err(|_| fail("invalid_attachment"))?;
            let cut = s.chars().count() > TEXT_CAP;
            ("text", Some(s.chars().take(TEXT_CAP).collect::<String>().lines().map(String::from).collect()), cut)
        }
        _ => ("other", None, false),
    };
    let mut v = json!({"name": shown, "path": path, "bytes": bytes.len(), "sha256": hex_sha256(&bytes), "kind": kind});
    match text {
        Some(t) => {
            v["text"] = json!(t);
            v["truncated"] = json!(truncated);
        }
        None => v["note"] = json!("내용 추출 미지원 형식. 클라이언트가 직접 읽는다"),
    }
    Ok(v)
}

pub fn tool_defs() -> Value {
    let s = |props: Value, req: Value| json!({"type": "object", "properties": props, "required": req, "additionalProperties": false});
    let plan_desc = "{\"project\":\"사업 키(선택, 사업 기록이 있으면)\",\"documents\":[{\"card\":\"카드id\",\"slots\":{\"슬롯id\":{\"value\":\"값\",\"source\":\"출처\"}},\"overrides\":[{\"rule\":\"룰id\",\"decision\":\"사용자 결정\"}]}]}. source: 파일명+위치 / user / llm(서술 슬롯만). memory 출처는 코어가 사업 기록에서 붙인다. 계산 슬롯은 넣지 않는다.";
    json!([
        {"name": "oh_guide", "description": "진입 지침과 카드 목록. 가장 먼저 호출한다.", "inputSchema": s(json!({"client": {"type": "string", "enum": CLIENTS}}), json!([]))},
        {"name": "oh_library_search", "description": "라이브러리(카드·규정·템플릿·예시)에서 문서 유형을 찾는다.", "inputSchema": s(json!({"query": {"type": "string"}, "kind": {"type": "string", "enum": ["card", "regulation", "template", "example"]}, "limit": {"type": "integer", "minimum": 1, "maximum": 20}}), json!(["query"]))},
        {"name": "oh_card_get", "description": "카드의 슬롯·계산·룰·문장 지침 요약(템플릿 XML 없음).", "inputSchema": s(json!({"card": {"type": "string"}}), json!(["card"]))},
        {"name": "oh_case_open", "description": "작업을 연다. attachments는 작업 폴더 inbox 안의 파일 이름이나 절대 경로. HWPX·XLSX·텍스트는 내용을 추출해 돌려준다.", "inputSchema": s(json!({"summary": {"type": "string"}, "attachments": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_ATTACH}}), json!(["summary"]))},
        {"name": "oh_plan_check", "description": "계획을 코어가 검증한다(빠진 슬롯, 출처 규칙, 계산, 룰, 기본값).", "inputSchema": s(json!({"case_id": {"type": "string"}, "plan": {"type": "object", "description": plan_desc}}), json!(["case_id", "plan"]))},
        {"name": "oh_draft", "description": "코어가 문서를 작성한다. draft id와 리뷰 패킷을 돌려준다.", "inputSchema": s(json!({"case_id": {"type": "string"}, "plan": {"type": "object", "description": plan_desc}}), json!(["case_id", "plan"]))},
        {"name": "oh_revise", "description": "슬롯 패치로 draft를 새 개정으로 다시 작성한다.", "inputSchema": s(json!({"draft_id": {"type": "string"}, "patch": {"type": "object", "description": "{\"revision\":N,\"documents\":[{\"index\":0,\"slots\":{...},\"remove\":[슬롯id],\"overrides\":[...]}]}"}}), json!(["draft_id", "patch"]))},
        {"name": "oh_finalize", "description": "코어 영수증이 있는 draft를 outbox에 저장한다(덮어쓰지 않음). 문서가 여럿이면 document 번호가 필요하다.", "inputSchema": s(json!({"draft_id": {"type": "string"}, "output_name": {"type": "string"}, "document": {"type": "integer", "minimum": 0}}), json!(["draft_id", "output_name"]))},
        {"name": "oh_project_get", "description": "사업 기록(작업 폴더 projects/)을 본다. project 없이 부르면 목록. 같은 사업의 두 번째 서류부터 사업명·코드·부서·지분 등을 다시 묻지 않게 한다.", "inputSchema": s(json!({"project": {"type": "string"}}), json!([]))},
        {"name": "oh_project_save", "description": "확인된 사업 정보를 출처와 함께 사업 기록에 저장한다(사업정보 문서·계약서 등에서 읽은 값). 문서를 확정하면 코어가 자동 저장하므로 문서 없이 기록만 할 때 쓴다.", "inputSchema": s(json!({"project": {"type": "string"}, "facts": {"type": "object", "description": format!("{{\"키\": {{\"value\": \"값\", \"source\": \"파일명+위치 또는 user\"}}}}. 키: {FACT_KEYS:?} 또는 x_로 시작")}}), json!(["project", "facts"]))},
        {"name": "oh_library_register", "description": "사용자 양식(값이 들어간 HWPX)을 개인 카드로 등록하는 1·2단계. ① template만 주면 코어가 구조(라벨·표·숫자·합계 행)를 돌려준다. ② registration_id + card(카드 초안 TOML) + sample(원본 값) [+ expect(계산 검산값)]을 주면 코어가 앵커·계산·왕복 시험으로 검증한다. 저장하지 않는다.", "inputSchema": s(json!({
            "template": {"type": "string", "description": "inbox 안의 HWPX 이름이나 절대 경로(첫 호출)"},
            "registration_id": {"type": "string"},
            "card": {"type": "string", "description": "카드 초안 TOML(template 필드 없이)"},
            "sample": {"type": "object", "description": "{\"슬롯id\": {\"value\": \"원본 값\", \"source\": \"원본 <위치>\"}}"},
            "expect": {"type": "object", "description": "{\"계산id\": \"원본에 적힌 값\"}"}
        }), json!([]))},
        {"name": "oh_library_commit", "description": "검증된 등록 카드를 개인 라이브러리에 저장한다. 사용자가 요약·저장 위치를 확인한 뒤에만 user_confirmed=true로 부른다. 같은 id가 있으면 replace, 사용자 결정이 필요한 차이를 받아들였으면 accept_differences.", "inputSchema": s(json!({
            "registration_id": {"type": "string"}, "user_confirmed": {"type": "boolean"},
            "accept_differences": {"type": "boolean"}, "replace": {"type": "boolean"}
        }), json!(["registration_id", "user_confirmed"]))}
    ])
}

fn conflict_json(c: &oh_library::Conflict) -> Value {
    json!({"kind": c.kind, "id": c.id, "used": c.used, "shadowed": c.shadowed})
}
/// Layers in priority order with their status, plus id conflicts (D12 socket).
pub(crate) fn layers_json(s: &Stack) -> Value {
    json!({
        "order": "personal > packs > base",
        "layers": s.layers.iter().map(|l| json!({"name": l.name, "kind": l.kind, "path": l.path, "version": l.version, "status": l.status, "cards": l.cards})).collect::<Vec<_>>(),
        "conflicts": s.conflicts.iter().map(conflict_json).collect::<Vec<_>>(),
    })
}
