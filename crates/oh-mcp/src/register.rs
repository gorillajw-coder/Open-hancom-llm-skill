//! Template registration (D10, D14): a user's filled HWPX becomes a card in the personal library.
//!
//! 1. `extract`: the core reads the structure (text nodes with their indices, table cells, label/value
//!    pairs, number/date patterns, total rows) so the LLM can draft a card.
//! 2. `validate`: the LLM's card draft (TOML, without template fields) is bound to the user's file as
//!    template: anchors must hit exactly one place, calcs run on the original values (`sample`) and are
//!    compared with the numbers in the original (`expect`), and a round trip (draft the card with the
//!    original values, compare every paragraph with the original) must reproduce the file.
//! 3. `commit` (only after the user confirmed): card + template go to `<install>/personal-library`,
//!    its manifest is rewritten with hashes, the stack is reloaded and checked. Replacing a card id
//!    that already exists needs `replace` (D4-7: no overwrite outside this procedure).
//!
//! The LLM only proposes. Nothing here loads LLM code; an unknown calc function stops with
//! `unknown_function` from the card parser (a core extension request, LIBRARY-GUIDE 6).
use oh_document::Package;
use oh_library::{open_pack, Stack, PERSONAL_DIR};
use oh_recipe::{finalize, hex_sha256, Card, Engine, Frame, Plan};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Card ids written to the personal library: ascii lowercase, digits, `-`.
pub fn valid_card_id(id: &str) -> bool {
    (1..=60).contains(&id.len()) && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !id.starts_with('-') && !id.ends_with('-')
}

pub struct Registration {
    pub source: String,
    /// The user's file after the stale line-cache cleanup (the card's template).
    pub template: Vec<u8>,
    pub dir: PathBuf,
    pub attempts: u32,
    pub last: Option<Validated>,
}
pub struct Validated {
    pub card_id: String,
    /// Card file content as it will be saved (template fields filled by the core).
    pub card_toml: String,
    /// ready | needs_user_decision | needs_fix
    pub status: &'static str,
    /// Layer that already supplies this card id (commit needs `replace`).
    pub existing_layer: Option<String>,
    /// What the user has to decide about (round-trip differences, calc mismatches, failing block rules).
    pub decisions: Vec<Value>,
}

const TEXT_LIMIT: usize = 30_000;

/// Paragraph texts of every section in order (own text only: a paragraph that holds a table does
/// not include the table's text; the table's paragraphs follow as their own entries).
pub fn paragraphs(bytes: &[u8]) -> Result<Vec<String>, &'static str> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "invalid_package")?;
    let mut names: Vec<(u32, String)> = z
        .file_names()
        .filter_map(|n| n.strip_prefix("Contents/section").and_then(|s| s.strip_suffix(".xml")).and_then(|s| s.parse::<u32>().ok()).map(|k| (k, n.to_string())))
        .collect();
    names.sort();
    let mut out = Vec::new();
    for (_, n) in names {
        let mut s = String::new();
        z.by_name(&n).map_err(|_| "invalid_package")?.read_to_string(&mut s).map_err(|_| "invalid_section_xml")?;
        let d = roxmltree::Document::parse(&s).map_err(|_| "invalid_section_xml")?;
        for p in d.descendants().filter(|x| x.tag_name().name() == "p") {
            let own: String = p
                .descendants()
                .filter(|x| x.tag_name().name() == "t" && x.ancestors().skip(1).find(|a| a.tag_name().name() == "p") == Some(p))
                .flat_map(|t| t.descendants().filter(|x| x.is_text()).map(|x| x.text().unwrap_or("")))
                .collect();
            out.push(own);
        }
    }
    Ok(out)
}

/// Read and normalize the user's file. Returns (template bytes, cleanup note).
pub fn prepare(raw: Vec<u8>) -> Result<(Vec<u8>, Option<&'static str>), &'static str> {
    let p = Package::from_bytes(raw.clone())?;
    match p.drop_stale_linesegs()? {
        Some(prep) => Ok((prep.bytes, Some("낡은 줄 배치 캐시를 지웠다(한컴 2022가 여는 형태로). 글자는 그대로다"))),
        None => Ok((raw, None)),
    }
}

// ---------------------------------------------------------------- structure
fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Number-like tokens in a text: (kind, raw, normalized).
fn numbers_in(text: &str) -> Vec<(&'static str, String, String)> {
    let cs: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        if !cs[i].is_ascii_digit() || (i > 0 && (cs[i - 1].is_ascii_digit() || cs[i - 1] == '.' || cs[i - 1] == ',')) {
            i += 1;
            continue;
        }
        // date: YYYY[. -/년]M[. -/월]D[일 .]
        if let Some((len, iso)) = date_at(&cs[i..]) {
            out.push(("date", cs[i..i + len].iter().collect(), iso));
            i += len;
            continue;
        }
        let mut j = i;
        while j < cs.len() && (cs[j].is_ascii_digit() || ((cs[j] == ',' || cs[j] == '.') && j + 1 < cs.len() && cs[j + 1].is_ascii_digit())) {
            j += 1;
        }
        let raw: String = cs[i..j].iter().collect();
        let norm: String = raw.chars().filter(|c| *c != ',').collect();
        let after: String = cs[j..].iter().take(4).collect::<String>();
        let after = after.trim_start();
        let before: String = cs[..i].iter().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect();
        let before = before.trim_end();
        let kind = if after.starts_with('%') {
            "percent"
        } else if before.ends_with("USD") || before.ends_with('$') || before.ends_with("한화") || after.starts_with('원') || after.starts_with("달러") || after.starts_with("USD") || after.starts_with("백만원") || after.starts_with("천원") || raw.contains(',') {
            "money"
        } else {
            "number"
        };
        if raw.chars().filter(char::is_ascii_digit).count() >= 2 || kind != "number" {
            out.push((kind, raw, norm));
        }
        i = j;
    }
    out
}
fn date_at(cs: &[char]) -> Option<(usize, String)> {
    let num = |from: usize, max: usize| -> Option<(usize, u32)> {
        let mut k = from;
        while k < cs.len() && k - from < max && cs[k].is_ascii_digit() {
            k += 1;
        }
        (k > from).then(|| (k, cs[from..k].iter().collect::<String>().parse().unwrap_or(0)))
    };
    let sep = |from: usize| -> usize {
        let mut k = from;
        while k < cs.len() && (cs[k] == ' ' || cs[k] == '.' || cs[k] == '-' || cs[k] == '/' || cs[k] == '년' || cs[k] == '월') {
            k += 1;
        }
        k
    };
    let (e, y) = num(0, 4)?;
    if e != 4 || !(1990..=2100).contains(&y) {
        return None;
    }
    let s1 = sep(e);
    if s1 == e {
        return None;
    }
    let (e2, m) = num(s1, 2)?;
    let s2 = sep(e2);
    if s2 == e2 || !(1..=12).contains(&m) {
        return None;
    }
    let (e3, d) = num(s2, 2)?;
    if !(1..=31).contains(&d) {
        return None;
    }
    let mut end = e3;
    if end < cs.len() && (cs[end] == '일' || cs[end] == '.') {
        end += 1;
    }
    Some((end, format!("{y:04}-{m:02}-{d:02}")))
}

/// Structure the LLM drafts a card from. Indices are the core's text-node indices (the `after`
/// anchor counts them, empty nodes included).
pub fn extract(template: &[u8]) -> Result<Value, &'static str> {
    let p = Package::from_bytes(template.to_vec())?;
    let nodes = &p.inspection.text_nodes;
    let grid = p.table_cells()?;
    let mut used = 0usize;
    let mut text_nodes = Vec::new();
    let mut truncated = false;
    for (i, n) in nodes.iter().enumerate() {
        if n.text.trim().is_empty() {
            continue;
        }
        used += n.text.chars().count() + 12;
        if used > TEXT_LIMIT {
            truncated = true;
            break;
        }
        text_nodes.push(json!([i, n.text, n.in_table]));
    }
    // "라벨 : 값" lines (a `starts` anchor keeps the label and writes the value after it)
    let mut label_lines = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let t = &n.text;
        if let Some(k) = t.find(|c| c == ':' || c == '：') {
            let colon_len = t[k..].chars().next().map(char::len_utf8).unwrap_or(1);
            let mut end = k + colon_len;
            while t[end..].starts_with(' ') {
                end += 1;
            }
            let prefix = &t[..end];
            let label = t[..k].trim();
            if label.is_empty() || label.chars().count() > 24 {
                continue;
            }
            let unique = nodes.iter().filter(|x| x.text.starts_with(prefix)).count() == 1;
            label_lines.push(json!({"node": i, "starts": prefix, "value": t[end..].chars().take(200).collect::<String>(), "unique": unique}));
        }
    }
    if label_lines.len() > 150 {
        label_lines.truncate(150);
        truncated = true;
    }
    // label cells: short text whose right (or lower) neighbour is a value cell
    let mut label_cells = Vec::new();
    for g in grid.iter().filter(|g| !g.nested) {
        let sq = squash(&g.text);
        let n = sq.chars().count();
        if n == 0 || n > 12 || sq.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        let unique = grid.iter().filter(|x| !x.nested && squash(&x.text) == sq).count() == 1;
        let right = grid.iter().find(|x| x.table == g.table && x.row == g.row && x.col == g.col + g.col_span);
        let below = grid.iter().find(|x| x.table == g.table && x.row == g.row + g.row_span && x.col == g.col);
        let mut v = json!({"label": g.text, "table": g.table, "row": g.row, "col": g.col, "unique": unique});
        let cut = |t: &str| t.chars().take(200).collect::<String>();
        if let Some(r) = right {
            v["right"] = json!(cut(&r.text));
        }
        if let Some(b) = below {
            v["below"] = json!(cut(&b.text));
        }
        if right.is_some() || below.is_some() {
            label_cells.push(v);
        }
    }
    label_cells.truncate(120);
    let mut tables = Vec::new();
    let mut cell_budget = 1500usize;
    for (ti, t) in p.inspection.tables.iter().enumerate() {
        let cells: Vec<Value> = grid
            .iter()
            .filter(|g| g.table == ti)
            .take_while(|_| {
                let ok = cell_budget > 0;
                cell_budget = cell_budget.saturating_sub(1);
                if !ok {
                    truncated = true;
                }
                ok
            })
            .map(|g| {
                let text: String = g.text.chars().take(120).collect();
                let mut v = json!([g.row, g.col, text]);
                if g.row_span > 1 || g.col_span > 1 {
                    v.as_array_mut().unwrap().push(json!(format!("span {}x{}", g.row_span, g.col_span)));
                }
                if g.nested {
                    v.as_array_mut().unwrap().push(json!("nested_table"));
                }
                v
            })
            .collect();
        tables.push(json!({"table": ti, "rows": t.attributes.get("rowCnt"), "cols": t.attributes.get("colCnt"), "nested": t.in_table, "cells": cells}));
    }
    let mut numbers = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        for (kind, raw, norm) in numbers_in(&n.text) {
            numbers.push(json!({"node": i, "kind": kind, "raw": raw, "value": norm}));
        }
    }
    numbers.truncate(200);
    let total_words = ["합계", "합 계", "총계", "소계", "계", "총액"];
    let mut total_rows = Vec::new();
    for g in grid.iter().filter(|g| !g.nested && g.col == 0) {
        let sq = squash(&g.text);
        if total_words.iter().any(|w| sq == squash(w) || (sq.starts_with(&squash(w)) && w.chars().count() > 1)) {
            let row: Vec<String> = grid.iter().filter(|x| x.table == g.table && x.row == g.row).map(|x| x.text.clone()).collect();
            total_rows.push(json!({"table": g.table, "row": g.row, "cells": row}));
        }
    }
    let labels: BTreeSet<String> = grid.iter().map(|g| squash(&g.text)).collect();
    let has = |s: &str| labels.contains(&squash(s));
    let starts = |s: &str| nodes.iter().any(|n| n.text.starts_with(s));
    let frame_hint = if starts("작성일 : ") && has("사업명") && has("제목") && has("작성자") {
        json!({"frame": "approval-memo", "why": "작성일·사업명·제목·작성자 머리 표가 있다. extends = \"approval-memo\"로 머리 표를 상속하고 본문만 선언한다(머리 표 앵커가 하나씩 걸리는지는 검증이 확인한다)"})
    } else if has("수신") && has("제목") {
        json!({"frame": "official-letter", "why": "수신·제목 칸이 있는 공문 형태. 공문 틀 좌표가 다르면 같은 id의 [[cell]]을 카드에서 다시 선언한다"})
    } else {
        Value::Null
    };
    // paragraphs Hancom numbers by itself: their saved text has no number, so a card must not type one
    // capped like the other lists (a template with thousands of numbered paragraphs stays small)
    const NUM_CAP: usize = 200;
    let all_auto: Vec<_> = oh_recipe::numbering::auto_paragraphs(template).into_iter().filter(|a| !a.text.trim().is_empty()).collect();
    let auto_truncated = all_auto.len() > NUM_CAP;
    let auto: Vec<Value> = all_auto
        .into_iter()
        .take(NUM_CAP)
        .map(|a| json!({"text": a.text.chars().take(60).collect::<String>(), "kind": a.kind, "format": a.format.map(|f| f.chars().take(20).collect::<String>())}))
        .collect();
    let all_doubled = oh_recipe::numbering::doubled_numbering(template);
    let doubled_truncated = all_doubled.len() > NUM_CAP;
    let doubled: Vec<Value> = all_doubled.into_iter().take(NUM_CAP).map(|d| json!({"text": d.text, "reason": d.reason, "blocking": d.blocking})).collect();
    Ok(json!({
        "text_nodes": text_nodes, "text_nodes_note": "[번호, 글자, 표 안 여부]. 빈 노드는 뺐지만 번호는 코어 번호 그대로다(after 앵커의 n은 빈 노드도 센다)",
        "auto_numbered": auto,
        "auto_numbered_note": "한컴이 번호(NUMBER)·글머리(BULLET)를 스스로 붙이는 문단이다. 화면에는 번호가 보이지만 text에는 없다. 이 문단을 쓰는 fmt와 슬롯 값에는 번호를 넣지 않는다. 넣으면 \"1. 1.\"로 찍히고 검증이 numbering_doubled로 막는다. 번호가 없는 일반 문단에 여러 줄 번호가 필요하면 num/ganada 필터를 쓴다",
        "auto_numbered_truncated": auto_truncated,
        "numbering_doubled": doubled,
        "numbering_doubled_truncated": doubled_truncated,
        "numbering_doubled_note": "원본에서 번호가 두 번 보이는 문단. blocking: true(자동 번호+글자 번호, 서로 다른 번호 두 개)면 작성이 막히므로 카드가 그 문단을 keep_prefix = false로 다시 쓰면서 글자 번호를 뺀다. blocking: false(reason maybe_date: 같은 숫자 두 번 \"1. 1.\", auto_date: 자동 번호 문단이 날짜 모양 \"3. 2.\"로 시작)는 날짜일 수 있으니 고치지 말고 사용자에게 확인한다",
        "truncated": truncated,
        "label_lines": label_lines, "label_cells": label_cells, "tables": tables,
        "numbers": numbers, "total_rows": total_rows, "frame_hint": frame_hint,
    }))
}

// ---------------------------------------------------------------- validation
/// Line-level diff of two paragraph lists (LCS). Returns changed hunks (original lines, new lines).
fn diff(a: &[String], b: &[String]) -> Vec<(Vec<String>, Vec<String>)> {
    let (n, m) = (a.len(), b.len());
    if n * m > 4_000_000 {
        return if a == b { vec![] } else { vec![(a.to_vec(), b.to_vec())] };
    }
    let mut l = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            l[i][j] = if a[i] == b[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    let mut cur: (Vec<String>, Vec<String>) = (vec![], vec![]);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            if !cur.0.is_empty() || !cur.1.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            i += 1;
            j += 1;
        } else if j < m && (i == n || l[i][j + 1] >= l[i + 1][j]) {
            cur.1.push(b[j].clone());
            j += 1;
        } else {
            cur.0.push(a[i].clone());
            i += 1;
        }
    }
    if !cur.0.is_empty() || !cur.1.is_empty() {
        out.push(cur);
    }
    out
}

fn norm_num(s: &str) -> Option<decnum::Num> {
    decnum::parse(s)
}
/// Decimal equality without pulling rust_decimal into this crate: compare the canonical digit
/// strings (sign, integer part without leading zeros, fraction without trailing zeros).
mod decnum {
    #[derive(PartialEq, Eq, Debug)]
    pub struct Num(pub bool, pub String, pub String);
    pub fn parse(s: &str) -> Option<Num> {
        let t: String = s.trim().chars().filter(|c| *c != ',' && !c.is_whitespace()).collect();
        let t = t.trim_start_matches("USD").trim_start_matches('$').trim_end_matches('원').trim_end_matches('%');
        let (neg, t) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t),
        };
        let (i, f) = t.split_once('.').unwrap_or((t, ""));
        if i.is_empty() && f.is_empty() || !i.chars().all(|c| c.is_ascii_digit()) || !f.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let i = i.trim_start_matches('0').to_string();
        let f = f.trim_end_matches('0').to_string();
        let zero = i.is_empty() && f.is_empty();
        Some(Num(neg && !zero, i, f))
    }
}

pub struct Input<'a> {
    pub card_toml: &'a str,
    pub sample: &'a Value,
    pub expect: Option<&'a serde_json::Map<String, Value>>,
}

/// General-purpose calc functions named in hints (travel-table functions are left out on purpose).
pub const GENERAL_FUNCS: &str = "sum(a, b, …), sub(a, b), mul(a, b), div(a, b), neg, abs, pct(x, p), round_to(x, unit, 'half_up'|'floor'|'ceil'), round_krw, fx(usd, rate), vat, share(total, p, unit, mode), share_rest(…), prorate(amount, 연요율%, 개월), coalesce(a, b), if_pos, if_eq, date_add(date, n), days(a, b), add_months(date, n), months_between(a, b), weekday, list_sum(list, 'field'), list_len";

fn problem(code: &str, detail: impl Into<String>) -> Value {
    let mut d: String = detail.into();
    match code {
        "unknown_filter" => d.push_str(&format!(" (쓸 수 있는 필터: {})", oh_recipe::FILTERS.join(", "))),
        "unknown_function" => d.push_str(&format!(" (일반 계산 함수: {GENERAL_FUNCS}. 이 밖의 계산은 코어 확장 요청이다. 카드로 우회하지 않는다)")),
        "sample_missing_required" => d.push_str(" (상속한 틀의 필수 슬롯도 원본 값으로 넣는다)"),
        _ => {}
    }
    json!({"code": code, "detail": d})
}

/// Check a card draft against the registered file. Returns (report, validated).
pub fn validate(reg: &Registration, stack: &Stack, base_engine: &Engine, inp: &Input) -> (Value, Validated) {
    let mut fixes: Vec<Value> = Vec::new();
    let mut decisions: Vec<Value> = Vec::new();
    let fail = |fixes: Vec<Value>, id: String, toml: String, existing: Option<String>| {
        let v = Validated { card_id: id.clone(), card_toml: toml, status: "needs_fix", existing_layer: existing, decisions: vec![] };
        (json!({"status": "needs_fix", "card": id, "problems": fixes, "next_step": "problems를 고친 카드 초안으로 oh_library_register를 다시 부른다(registration_id 그대로)"}), v)
    };
    // 1. card text: the core owns template fields; no second output or file inputs in 1.0 registration
    let table: toml::Table = match toml::from_str(inp.card_toml) {
        Ok(t) => t,
        Err(e) => return fail(vec![problem("invalid_card", e.to_string())], String::new(), String::new(), None),
    };
    for k in ["template", "template_sha256"] {
        if table.contains_key(k) {
            fixes.push(problem("invalid_card", format!("{k}: 코어가 넣는다. 초안에서 뺀다")));
        }
    }
    if table.contains_key("xlsx") || table.contains_key("extract") {
        fixes.push(problem("unsupported_in_registration", "등록 흐름은 HWPX 카드만 만든다(xlsx·extract는 손으로 만드는 카드에서만)"));
    }
    let id = table.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if !valid_card_id(&id) {
        fixes.push(problem("invalid_card_id", format!("{id:?}: 영문 소문자·숫자·- 1~60자")));
    }
    if !fixes.is_empty() {
        return fail(fixes, id, String::new(), None);
    }
    let sha = hex_sha256(&reg.template);
    let full = format!("# 등록 흐름으로 만든 카드 (원본: {}). 템플릿은 원본 파일 그대로다.\ntemplate = \"templates/{id}.hwpx\"\ntemplate_sha256 = \"{sha}\"\n{}", reg.source.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>(), inp.card_toml);
    let existing = stack.card_layer(&id).map(String::from);
    // 2. frames from the stack (highest layer wins), bind against the file
    let mut frames: Vec<Frame> = Vec::new();
    for (_, lib) in stack.libraries() {
        if let Ok(fs) = lib.frames() {
            for f in fs {
                if !frames.iter().any(|x| x.id == f.id) {
                    frames.push(f);
                }
            }
        }
    }
    let card = match Card::from_toml(&full).and_then(|c| c.with_frames(&frames)) {
        Ok(c) => c,
        Err(e) => return fail(vec![problem(e.code, e.detail)], id, full, existing),
    };
    let reg_id = card.regulation.clone();
    let mut eng = match Engine::new(base_engine.defaults.clone(), vec![(card.clone(), reg.template.clone())]) {
        Ok(e) => e,
        Err(e) => return fail(vec![problem(e.code, e.detail)], id, full, existing),
    };
    if let Some(r) = reg_id {
        match base_engine.regulation(&r) {
            Some(x) => eng.set_regulation(x.clone()),
            None => return fail(vec![problem("unknown_regulation", r)], id, full, existing),
        }
    }
    // 3. sample = the original's values as plan slots
    let plan_json = json!({"documents": [{"card": id, "slots": inp.sample}]});
    let plan: Plan = match oh_document::parse_contract(plan_json.to_string().as_bytes(), "invalid_plan") {
        Ok(p) => p,
        Err(c) => return fail(vec![problem(c, "sample: {슬롯id: {value, source}} (원본에 있는 값, 출처는 \"원본 <위치>\")")], id, full, existing),
    };
    let rep = eng.plan_check(&plan);
    for e in &rep.errors {
        fixes.push(json!({"code": e.code, "slot": e.id, "detail": e.detail}));
    }
    let doc = &rep.documents[0];
    if !doc.missing_required.is_empty() {
        fixes.push(problem("sample_missing_required", format!("원본 값이 필요한 슬롯: {:?}", doc.missing_required)));
    }
    if !fixes.is_empty() {
        return fail(fixes, id, full, existing);
    }
    // 4. calcs on the original numbers
    let mut calc_checks = Vec::new();
    if let Some(exp) = inp.expect {
        for (k, v) in exp {
            let want = v.as_str().map(str::to_string).or_else(|| v.as_number().map(|n| n.to_string())).unwrap_or_default();
            match doc.calcs.iter().find(|c| &c.id == k) {
                None => fixes.push(problem("unknown_calc", format!("expect {k}: 카드에 없는 계산"))),
                Some(c) => {
                    let ok = match (c.computed.as_deref().and_then(norm_num), norm_num(&want)) {
                        (Some(a), Some(b)) => a == b,
                        _ => c.computed.as_deref() == Some(want.as_str()),
                    };
                    calc_checks.push(json!({"calc": k, "expr": c.expr, "computed": c.computed, "original": want, "ok": ok}));
                    if !ok {
                        decisions.push(json!({"kind": "calc_mismatch", "calc": k, "computed": c.computed, "original": want, "note": "계산식이 틀렸거나 원본 숫자가 틀렸다. 사용자에게 어느 쪽인지 묻는다"}));
                    }
                }
            }
        }
    }
    let unchecked: Vec<&str> = doc.calcs.iter().filter(|c| inp.expect.is_none_or(|e| !e.contains_key(&c.id)) && c.status != "ok").map(|c| c.id.as_str()).collect();
    for w in doc.warnings.iter().filter(|w| w.code == "calc_mismatch") {
        decisions.push(json!({"kind": "calc_mismatch", "calc": w.id, "detail": w.detail}));
    }
    if !fixes.is_empty() {
        return fail(fixes, id, full, existing);
    }
    for r in doc.rules.iter().filter(|r| r.status == "fail") {
        decisions.push(json!({"kind": "rule_fails_on_original", "rule": r.id, "severity": r.severity, "message": r.message}));
    }
    // 5. round trip: draft with the original values, compare every paragraph with the original
    let draft = match eng.draft(&plan) {
        Ok(d) => d,
        Err(e) => return fail(vec![problem(e.code, e.detail)], id, full, existing),
    };
    let out = &draft.documents[0].bytes;
    let structure = match finalize(&draft, 0) {
        Ok(f) => f.receipt["structure"].clone(),
        Err(e) if e.code == "block_rule_failed" => Value::Null,
        Err(e) => return fail(vec![problem(e.code, e.detail)], id, full, existing),
    };
    let (orig, new) = match (paragraphs(&reg.template), paragraphs(out)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => return fail(vec![problem("roundtrip_unreadable", "")], id, full, existing),
    };
    let hunks = diff(&orig, &new);
    let shown: Vec<Value> = hunks.iter().take(30).map(|(a, b)| json!({"original": a, "rebuilt": b})).collect();
    if !hunks.is_empty() {
        decisions.push(json!({"kind": "roundtrip_differences", "count": hunks.len(), "note": "원본 값으로 다시 만든 문서가 원본과 다르다. 대개 fmt·필터·앵커 문제라 카드를 고친다. 원본의 오타처럼 의도한 차이면 사용자에게 확인받는다"}));
    }
    // 6. which places the card writes (so the user can see nothing important is left fixed)
    let written: Vec<Value> = card
        .targets
        .iter()
        .map(|t| json!({"target": t.id, "fmt": t.fmt}))
        .chain(card.cells.iter().map(|c| json!({"cell": c.id, "label": c.label, "table": c.table, "row": c.row, "col": c.col, "fmt": c.fmt})))
        .chain(card.rows.iter().map(|r| json!({"rows": r.id, "table": r.table, "list": r.list})))
        .collect();
    // 7. numbers the card leaves as they are: draft again with every given value changed and list the
    //    original paragraphs with money/percent figures that still come out unchanged. They are not an
    //    error (the user may want them fixed), but a total or share derived from changed values would go stale.
    let fixed_numbers = fixed_numbers(&eng, &card, &orig, inp.sample);
    // 8. D4-2 guards the round trip cannot see:
    //    a calc no fmt writes (its place in the document stays the original figure);
    for c in card.calcs.iter().filter(|c| !calc_used(&card, &c.id)) {
        decisions.push(json!({"kind": "calc_not_written", "calc": c.id, "note": "이 계산을 쓰는 자리(fmt)가 없다. 원본의 그 숫자는 고정으로 남는다. 자리를 [[cell]]·[[target]]으로 넣는다"}));
    }
    //    an expected (computed) figure that stays fixed everywhere it appears in the original;
    if let (Some(exp), Some(fixed)) = (inp.expect, fixed_numbers.as_ref()) {
        for (k, v) in exp {
            let want = v.as_str().map(str::to_string).or_else(|| v.as_number().map(|n| n.to_string())).unwrap_or_default();
            let Some(w) = norm_num(&want) else { continue };
            let count = |ps: &mut dyn Iterator<Item = &String>| ps.map(|p| numbers_in(p).iter().filter(|(_, _, n)| norm_num(n).as_ref() == Some(&w)).count()).sum::<usize>();
            let (all, kept) = (count(&mut orig.iter()), count(&mut fixed.iter()));
            if all > 0 && kept == all {
                decisions.push(json!({"kind": "expected_value_stays_fixed", "calc": k, "value": want, "note": "원본에서 이 계산값이 있는 자리를 카드가 하나도 쓰지 않는다. 값이 바뀌어도 문서에는 원본 숫자가 남는다"}));
            }
        }
    }
    //    a money slot whose original value is a sum of other money slots (a total the LLM would fill).
    for id in computed_looking_slots(&card, inp.sample) {
        decisions.push(json!({"kind": "slot_looks_computed", "slot": id, "note": "원본 값이 다른 금액 슬롯들의 합과 같거나 합계 이름이다. 합계·차액이면 슬롯이 아니라 [[calc]]로 둔다(D4-2). 원본에 적힌 대조값이면 calc의 compare_to로 쓴다"}));
    }
    let status = if decisions.is_empty() { "ready" } else { "needs_user_decision" };
    let summary = json!({
        "id": card.id, "title": card.title, "extends": card.extends, "triggers": card.triggers,
        "slots": card.slots.iter().map(|s| json!({"id": s.id, "label": s.label, "kind": format!("{:?}", s.kind).to_lowercase(), "required": s.required})).collect::<Vec<_>>(),
        "calcs": card.calcs.iter().map(|c| json!({"id": c.id, "expr": c.expr})).collect::<Vec<_>>(),
        "rules": card.rules.iter().map(|r| json!({"id": r.id, "severity": format!("{:?}", r.severity).to_lowercase(), "message": r.message})).collect::<Vec<_>>(),
        "writes": written,
    });
    let report = json!({
        "status": status, "card": id,
        "existing_card": existing.as_ref().map(|l| json!({"layer": l, "note": "같은 id 카드가 이미 있다. 저장하면 개인 카드가 이 카드를 가린다(commit에 replace 필요)"})),
        "summary_for_user": summary,
        "calc_checks": calc_checks, "calcs_not_checked": unchecked,
        "fixed_numbers": fixed_numbers,
        "roundtrip": {"paragraphs": orig.len(), "differences": shown, "regressions_vs_template": structure.get("regressions_vs_template")},
        "decisions": decisions,
        "next_step": match status {
            "ready" => "summary_for_user(슬롯·계산·룰·쓰는 자리)와 fixed_numbers(카드가 바꾸지 않아 원본 그대로 남는 금액·비율 문단)와 저장 위치를 사용자에게 보여 주고 확인받은 뒤 oh_library_commit(user_confirmed=true)을 부른다. 바뀌어야 할 숫자(다른 값에서 계산되는 합계·분담 등)가 fixed_numbers에 있으면 슬롯·계산으로 넣자고 먼저 제안한다",
            _ => "decisions를 사용자에게 보여 준다. 카드 문제면 고쳐 다시 register, 원본 쪽 차이로 사용자가 받아들이면 commit에 accept_differences=true",
        },
    });
    let v = Validated { card_id: id, card_toml: full, status, existing_layer: existing, decisions };
    (report, v)
}

/// A calc is used when a fmt shows it, or another calc or a rule reads it.
fn calc_used(card: &Card, id: &str) -> bool {
    let mentions = |text: &str| {
        let b = text.as_bytes();
        let mut from = 0;
        while let Some(i) = text[from..].find(id) {
            let st = from + i;
            let en = st + id.len();
            let ok_l = st == 0 || !(b[st - 1].is_ascii_alphanumeric() || b[st - 1] == b'_');
            let ok_r = en >= b.len() || !(b[en].is_ascii_alphanumeric() || b[en] == b'_');
            if ok_l && ok_r {
                return true;
            }
            from = en;
        }
        false
    };
    card.targets.iter().any(|t| mentions(&t.fmt))
        || card.cells.iter().any(|c| mentions(&c.fmt))
        || card.rows.iter().any(|r| r.cols.iter().any(|f| mentions(f)))
        || card.calcs.iter().any(|c| c.id != id && mentions(&c.expr))
        || card.rules.iter().any(|r| r.when.as_deref().is_some_and(mentions) || r.assert.as_deref().is_some_and(mentions) || r.require.iter().any(|x| x == id))
}

/// Money/decimal slots whose sample value equals the sum of two or more other money/decimal
/// sample values, or whose label says total.
fn computed_looking_slots(card: &Card, sample: &Value) -> Vec<String> {
    use oh_recipe::Kind;
    // value in units of 1/10000, None when it does not parse
    let val = |s: &str| -> Option<i128> {
        let decnum::Num(neg, i, f) = norm_num(s)?;
        if f.len() > 4 || i.len() > 20 {
            return None;
        }
        let iv: i128 = if i.is_empty() { 0 } else { i.parse().ok()? };
        let fv: i128 = format!("{f:0<4}").parse().ok()?;
        Some(if neg { -(iv * 10000 + fv) } else { iv * 10000 + fv })
    };
    let mut nums: Vec<(String, String, i128)> = Vec::new();
    for sl in card.slots.iter().filter(|s| matches!(s.kind, Kind::Money | Kind::Decimal)) {
        if let Some(v) = sample.get(&sl.id).and_then(|x| x.get("value")).and_then(Value::as_str).and_then(val) {
            nums.push((sl.id.clone(), sl.label.clone(), v));
        }
    }
    let mut out = Vec::new();
    for (i, (id, label, v)) in nums.iter().enumerate() {
        let named = ["합계", "총액", "소계", "총계", "합 계"].iter().any(|w| label.contains(w));
        let others: Vec<i128> = nums.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| x.2).filter(|x| *x != 0).take(12).collect();
        let mut sum_hit = false;
        if *v != 0 {
            for mask in 1u32..(1u32 << others.len()) {
                if mask.count_ones() >= 2 && others.iter().enumerate().filter(|(b, _)| mask >> b & 1 == 1).map(|(_, x)| *x).sum::<i128>() == *v {
                    sum_hit = true;
                    break;
                }
            }
        }
        if named || sum_hit {
            out.push(id.clone());
        }
    }
    out
}

/// Original paragraphs holding money/percent figures that the card never rewrites (see step 7).
/// None when the perturbed sample does not draft (then nothing can be said).
fn fixed_numbers(eng: &Engine, card: &Card, orig: &[String], sample: &Value) -> Option<Vec<String>> {
    use oh_recipe::Kind;
    let mut m = sample.as_object()?.clone();
    for (k, v) in m.iter_mut() {
        let Some(val) = v.get("value").and_then(Value::as_str) else { continue };
        let new = match card.slot(k).map(|s| s.kind) {
            Some(Kind::Money) | Some(Kind::Decimal) => {
                let digits: String = val.chars().filter(|c| *c != ',').collect();
                if digits.is_empty() { continue }
                format!("9{digits}")
            }
            Some(Kind::Text) | Some(Kind::Person) | Some(Kind::Prose) => format!("{val}※"),
            _ => continue,
        };
        v["value"] = json!(new);
    }
    let plan: Plan = oh_document::parse_contract(json!({"documents": [{"card": card.id, "slots": m}]}).to_string().as_bytes(), "invalid_plan").ok()?;
    let d = eng.draft(&plan).ok()?;
    let moved = paragraphs(&d.documents[0].bytes).ok()?;
    if moved.len() != orig.len() {
        // rows added/removed: positions do not line up, compare as sets
        let set: BTreeSet<&String> = moved.iter().collect();
        return Some(orig.iter().filter(|p| set.contains(p) && numbers_in(p).iter().any(|(k, _, _)| *k == "money" || *k == "percent")).take(40).cloned().collect());
    }
    Some(orig.iter().zip(&moved).filter(|(a, b)| a == b && numbers_in(a).iter().any(|(k, _, _)| *k == "money" || *k == "percent")).map(|(a, _)| a.clone()).take(40).collect())
}

// ---------------------------------------------------------------- commit
fn toml_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Write card + template into `<root>/personal-library` and rewrite its manifest.
/// * The card id is checked again against the libraries on disk now (another registration or
///   another server process may have saved the same id since validation): replacing needs `replace`.
/// * Any failure after the first write restores the previous state: replaced files from the backup,
///   new files removed, the old manifest back (or the personal folder removed if this commit created it).
pub fn commit(root: &Path, library: &Path, v: &Validated, template: &[u8], replace: bool, stamp: &str) -> Result<Value, (String, String)> {
    let e = |c: &str, d: String| (c.to_string(), d);
    let dir = root.join(PERSONAL_DIR);
    let man_path = dir.join("manifest.toml");
    let created_dir = !dir.exists();
    // existing personal library must be intact: never rewrite a manifest we could not verify
    let mut rows: Vec<(String, String, String)> = Vec::new();
    let old_manifest = if man_path.is_file() {
        let lib = open_pack(&dir, false).map_err(|x| e("personal_library_invalid", x.to_string()))?;
        rows = lib.manifest.files.iter().map(|f| (f.path.clone(), f.sha256.clone(), f.kind.clone())).collect();
        Some(oh_host::bounded_read(&man_path, 1024 * 1024).map_err(|c| e(c, man_path.display().to_string()))?)
    } else {
        None
    };
    // the id as the libraries on disk see it now
    let now_layer = Stack::load_installed(library).ok().and_then(|s| s.card_layer(&v.card_id).map(String::from));
    let listed = rows.iter().any(|(p, _, _)| p == &format!("cards/{}.toml", v.card_id));
    if (now_layer.is_some() || listed) && !replace {
        return Err(e("card_exists", format!("{} ({})", v.card_id, now_layer.unwrap_or_else(|| "personal".into()))));
    }
    let card_rel = format!("cards/{}.toml", v.card_id);
    let tpl_rel = format!("templates/{}.hwpx", v.card_id);
    // back up files this commit replaces (kept out of the manifest); a free folder name per commit
    let mut bdir = dir.join("backup").join(stamp);
    let mut k = 1;
    while bdir.exists() {
        k += 1;
        bdir = dir.join("backup").join(format!("{stamp}-{k}"));
    }
    let mut backup: Vec<(String, PathBuf)> = Vec::new();
    for rel in [&card_rel, &tpl_rel] {
        let p = dir.join(rel);
        if p.is_file() {
            let b = oh_host::bounded_read(&p, oh_library::MAX_FILE).map_err(|c| e(c, p.display().to_string()))?;
            let to = bdir.join(rel);
            oh_host::ensure_dir(to.parent().unwrap()).map_err(|c| e("personal_library_write_failed", format!("{}: {c}", to.display())))?;
            oh_host::replace_file(&to, &b).map_err(|c| e("personal_library_write_failed", format!("{}: {c}", to.display())))?;
            backup.push((rel.clone(), to));
        }
    }
    let rollback = |why: (String, String)| -> (String, String) {
        if created_dir {
            let _ = std::fs::remove_dir_all(&dir);
            return why;
        }
        for rel in [&card_rel, &tpl_rel] {
            match backup.iter().find(|(r, _)| r == rel) {
                Some((_, from)) => {
                    if let Ok(b) = oh_host::bounded_read(from, oh_library::MAX_FILE) {
                        let _ = oh_host::replace_file(&dir.join(rel), &b);
                    }
                }
                None => {
                    let _ = std::fs::remove_file(dir.join(rel));
                }
            }
        }
        match &old_manifest {
            Some(b) => {
                let _ = oh_host::replace_file(&man_path, b);
            }
            None => {
                let _ = std::fs::remove_file(&man_path);
            }
        }
        why
    };
    for (rel, bytes) in [(&tpl_rel, template), (&card_rel, v.card_toml.as_bytes())] {
        let p = dir.join(rel);
        oh_host::ensure_dir(p.parent().unwrap()).map_err(|c| rollback(e("personal_library_write_failed", format!("{}: {c}", p.display()))))?;
        oh_host::replace_file(&p, bytes).map_err(|c| rollback(e("personal_library_write_failed", format!("{}: {c}", p.display()))))?;
    }
    rows.retain(|(p, _, _)| p != &card_rel && p != &tpl_rel);
    rows.push((tpl_rel.clone(), hex_sha256(template), "template".into()));
    rows.push((card_rel.clone(), hex_sha256(v.card_toml.as_bytes()), "card".into()));
    let mut out = format!("# 개인 라이브러리 (D12). oh_library_commit이 관리한다. 손으로 고치면 해시를 맞춰야 로드된다.\nversion = {}\nanonymized = false\nname = \"personal\"\n", toml_str(&format!("personal-{stamp}")));
    for (p, h, k) in &rows {
        out.push_str(&format!("\n[[file]]\npath = {}\nsha256 = {}\nkind = {}\n", toml_str(p), toml_str(h), toml_str(k)));
    }
    oh_host::replace_file(&man_path, out.as_bytes()).map_err(|c| rollback(e("personal_library_write_failed", format!("{}: {c}", man_path.display()))))?;
    // reload the whole stack: the personal layer must load and supply the card
    let check = Stack::load_installed(library).map_err(|x| x.to_string()).and_then(|s| {
        let layer = s.layers.iter().find(|l| l.kind == "personal").map(|l| l.status.clone()).unwrap_or_default();
        if layer != "loaded" {
            return Err(format!("personal: {layer}"));
        }
        if s.card_layer(&v.card_id) != Some("personal") {
            return Err(format!("{} not supplied by personal", v.card_id));
        }
        s.engine_with_local(None).map(|_| ()).map_err(|x| x.to_string())
    });
    if let Err(why) = check {
        return Err(rollback(e("commit_verification_failed", why)));
    }
    Ok(json!({
        "card": v.card_id, "layer": "personal", "library": dir,
        "files": [dir.join(&card_rel), dir.join(&tpl_rel)],
        "backup": backup.iter().map(|(_, p)| p).collect::<Vec<_>>(),
        "manifest_files": rows.len(),
    }))
}
