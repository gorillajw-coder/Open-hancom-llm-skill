//! Card registration: pin the template hash and bind label anchors to real text nodes
//! (via the existing inspection), then check the card is self-consistent.
use crate::card::{Anchor, Calc, Card, Defaults, Kind, Rule, Style};
use crate::error::{Error, Result};
use crate::expr::*;
use oh_document::Package;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct BoundNode {
    pub node_id: String,
    pub node_index: usize,
    /// Kept template prefix (label text before the value).
    pub prefix: String,
    pub template_text: String,
}
#[derive(Debug, Clone)]
pub struct BoundTarget {
    pub id: String,
    pub nodes: Vec<BoundNode>,
    pub fmt: Vec<Tok>,
    pub wrap_width: Option<usize>,
    pub continuation_indent: String,
}
#[derive(Debug, Clone)]
pub struct BoundRule {
    pub rule: Rule,
    pub when: Cond,
    pub assert: Option<Cond>,
}
#[derive(Debug, Clone)]
pub struct BoundCard {
    pub card: Card,
    pub template: Vec<u8>,
    /// Second-output workbook template (hash-checked against `card.xlsx`).
    pub xlsx_template: Option<Vec<u8>>,
    pub cells: Vec<(crate::card::CellTarget, Vec<Tok>)>,
    pub rows: Vec<(crate::card::RowsTarget, Vec<Vec<Tok>>)>,
    pub xlsx_cells: Vec<(crate::card::XlsxCell, Vec<Tok>)>,
    pub targets: Vec<BoundTarget>,
    pub calcs: Vec<(Calc, Expr)>,
    pub rules: Vec<BoundRule>,
    /// Diagnostic code counts of the pristine template; a draft may not add to them.
    pub baseline_diagnostics: BTreeMap<String, usize>,
}

/// Core version cards compare `requires_core` against.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn hex_sha256(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

pub fn diagnostic_counts(pkg: &Package) -> BTreeMap<String, usize> {
    let v = pkg.validate_supported();
    let mut m = BTreeMap::new();
    for d in v["diagnostics"].as_array().into_iter().flatten() {
        *m.entry(d["code"].as_str().unwrap_or("?").to_string()).or_insert(0) += 1;
    }
    m
}

pub fn slot_style(card: &Card, defaults: &Defaults, slot: &str) -> Option<Style> {
    let s = card.slot(slot)?;
    s.default_style.clone().or_else(|| s.default_ref.as_ref().and_then(|r| defaults.defaults.get(r)?.style.clone()))
}

fn locate(pkg: &Package, a: &Anchor, prev: Option<usize>, tid: &str) -> Result<(usize, String)> {
    let nodes = &pkg.inspection.text_nodes;
    let err = |code: &'static str, d: String| Error::new(code, format!("target {tid}: {d}"));
    let set = a.starts.is_some() as u8 + a.after.is_some() as u8 + a.follow.is_some() as u8;
    if set != 1 {
        return Err(err("invalid_anchor", "exactly one of starts/after/follow".into()));
    }
    let (idx, prefix) = if let Some(x) = &a.starts {
        let m: Vec<usize> = (0..nodes.len()).filter(|i| nodes[*i].text.starts_with(x.as_str())).collect();
        match m.len() {
            0 => return Err(err("anchor_not_found", x.clone())),
            1 => {}
            _ => return Err(err("anchor_ambiguous", x.clone())),
        }
        (m[0], if a.keep_prefix.unwrap_or(true) { x.clone() } else { String::new() })
    } else if let Some(l) = &a.after {
        let m: Vec<usize> = (0..nodes.len()).filter(|i| nodes[*i].text.trim().starts_with(l.as_str())).collect();
        match m.len() {
            0 => return Err(err("anchor_not_found", l.clone())),
            1 => {}
            _ => return Err(err("anchor_ambiguous", l.clone())),
        }
        let n = a.n.unwrap_or(1);
        if n == 0 {
            return Err(err("invalid_anchor", "n must be >= 1".into()));
        }
        (m[0] + n, String::new())
    } else {
        let p = prev.ok_or_else(|| err("invalid_anchor", "follow needs a previous anchor".into()))?;
        (p + a.follow.unwrap(), String::new())
    };
    let node = nodes.get(idx).ok_or_else(|| err("anchor_not_found", "offset past end".into()))?;
    if !node.editable {
        return Err(err("anchor_not_editable", node.id.clone()));
    }
    Ok((idx, prefix))
}

pub fn bind(card: Card, template: Vec<u8>, defaults: &Defaults) -> Result<BoundCard> {
    let bad = |code: &'static str, d: String| Error::new(code, format!("card {}: {d}", card.id));
    if hex_sha256(&template) != card.template_sha256 {
        return Err(bad("template_hash_mismatch", format!("expected {}", card.template_sha256)));
    }
    if let Some(req) = &card.requires_core {
        let v = |s: &str| -> Option<Vec<u64>> { let p: Vec<u64> = s.split('.').map(|x| x.parse().ok()).collect::<Option<_>>()?; (p.len() == 3).then_some(p) };
        let need = v(req).ok_or_else(|| bad("invalid_card", format!("requires_core {req}: x.y.z")))?;
        if v(CORE_VERSION).unwrap() < need {
            return Err(bad("core_too_old", format!("needs core {req}, this core is {CORE_VERSION}")));
        }
    }
    if card.extends.is_some() && !card.frame_merged {
        return Err(bad("frame_not_merged", "extends set: merge the frame first (Card::with_frames)".into()));
    }
    if let Some(p) = &card.project {
        if let Some(k) = p.map.keys().find(|k| card.slot(k).is_none()) {
            return Err(bad("unknown_reference", format!("project {k}")));
        }
    }
    let pkg = Package::from_bytes(template.clone()).map_err(|c| bad("template_unreadable", c.to_string()))?;
    // ids
    let mut ids = BTreeSet::new();
    for id in card.slots.iter().map(|s| &s.id).chain(card.calcs.iter().map(|c| &c.id)) {
        if !ids.insert(id.clone()) {
            return Err(bad("duplicate_id", id.clone()));
        }
    }
    for s in &card.slots {
        if s.default.is_some() && s.default_ref.is_some() {
            return Err(bad("invalid_slot", format!("{}: default and default_ref", s.id)));
        }
        if let Some(r) = &s.default_ref {
            if !defaults.defaults.contains_key(r) {
                return Err(bad("unknown_default_ref", r.clone()));
            }
        }
        if s.default_style.is_some() && s.default.is_none() && s.default_ref.is_none() {
            return Err(bad("invalid_slot", format!("{}: default_style without default", s.id)));
        }
        if s.kind == Kind::Prose && s.max_chars.is_none() {
            return Err(bad("invalid_slot", format!("{}: prose needs max_chars", s.id)));
        }
        if (s.kind == Kind::List) != !s.fields.is_empty() {
            return Err(bad("invalid_slot", format!("{}: list slots (only) need fields", s.id)));
        }
        if matches!(s.kind, Kind::List | Kind::File) && (s.default.is_some() || s.default_ref.is_some() || s.default_expr.is_some()) {
            return Err(bad("invalid_slot", format!("{}: no default for list/file", s.id)));
        }
        if s.default_expr.is_some() && (s.default.is_some() || s.default_ref.is_some()) {
            return Err(bad("invalid_slot", format!("{}: default_expr with default", s.id)));
        }
    }
    let mut known: BTreeSet<String> = card.slots.iter().map(|s| s.id.clone()).collect();
    for s in &card.slots {
        if let Some(src) = &s.default_expr {
            let e = parse_expr(src)?;
            let mut refs = Vec::new();
            expr_refs(&e, &mut refs);
            if let Some(r) = refs.iter().find(|r| !known.contains(*r)) {
                return Err(bad("unknown_reference", format!("default_expr {}: {r}", s.id)));
            }
        }
    }
    for x in &card.extracts {
        if card.slot(&x.from).map(|s| s.kind) != Some(Kind::File) {
            return Err(bad("invalid_extract", format!("{}: from must be a file slot", x.id)));
        }
        if !known.insert(x.id.clone()) {
            return Err(bad("duplicate_id", x.id.clone()));
        }
    }
    if let Some(c) = &card.carry {
        if let Some(k) = c.map.keys().find(|k| card.slot(k).is_none()) {
            return Err(bad("unknown_reference", format!("carry {k}")));
        }
    }
    // calcs
    let mut calcs = Vec::new();
    for c in &card.calcs {
        let e = parse_expr(&c.expr)?;
        let mut refs = Vec::new();
        expr_refs(&e, &mut refs);
        for r in refs {
            if !known.contains(&r) {
                return Err(bad("unknown_reference", format!("calc {}: {r}", c.id)));
            }
        }
        if let Some(t) = &c.compare_to {
            if card.slot(t).is_none() {
                return Err(bad("unknown_reference", format!("calc {} compare_to {t}", c.id)));
            }
        }
        known.insert(c.id.clone());
        calcs.push((c.clone(), e));
    }
    // rules
    let mut rules = Vec::new();
    let mut rule_ids = BTreeSet::new();
    for r in &card.rules {
        if !rule_ids.insert(r.id.clone()) {
            return Err(bad("duplicate_id", format!("rule {}", r.id)));
        }
        let when = parse_cond(r.when.as_deref().unwrap_or("always"))?;
        let asrt = r.assert.as_deref().map(parse_cond).transpose()?;
        let mut refs: Vec<String> = r.require.clone();
        cond_refs(&when, &mut refs);
        if let Some(a) = &asrt {
            cond_refs(a, &mut refs);
        }
        if let Some(x) = refs.iter().find(|x| !known.contains(*x)) {
            return Err(bad("unknown_reference", format!("rule {}: {x}", r.id)));
        }
        if r.require.is_empty() && asrt.is_none() {
            return Err(bad("invalid_rule", format!("{}: nothing to check", r.id)));
        }
        rules.push(BoundRule { rule: r.clone(), when, assert: asrt });
    }
    // targets
    let mut targets = Vec::new();
    let mut used = BTreeSet::new();
    let mut tids = BTreeSet::new();
    for t in &card.targets {
        if !tids.insert(t.id.clone()) {
            return Err(bad("duplicate_id", format!("target {}", t.id)));
        }
        if t.anchors.is_empty() {
            return Err(bad("invalid_anchor", format!("target {} has no anchor", t.id)));
        }
        let fmt = parse_fmt(&t.fmt)?;
        let mut refs = Vec::new();
        fmt_refs(&fmt, &mut refs);
        let mut styled = 0;
        for (r, f) in &refs {
            if !known.contains(r) {
                return Err(bad("unknown_reference", format!("target {}: {r}", t.id)));
            }
            if f.as_deref().is_some_and(|f| !FILTERS.contains(&f)) {
                return Err(bad("unknown_filter", format!("target {}: {f:?}", t.id)));
            }
            if slot_style(&card, defaults, r).is_some() {
                styled += 1;
            }
            if card.slot(r).is_some_and(|s| s.multiline || s.kind == Kind::List) {
                return Err(bad("invalid_target", format!("target {}: multiline/list slot {r} needs a cell target", t.id)));
            }
        }
        if styled > 1 || (styled == 1 && t.anchors.len() > 1) {
            return Err(bad("unsupported_style_layout", format!("target {}", t.id)));
        }
        if t.anchors.len() > 1 && t.wrap_width.is_none() {
            return Err(bad("invalid_anchor", format!("target {}: several anchors need wrap_width", t.id)));
        }
        let mut nodes = Vec::new();
        let mut prev = None;
        for a in &t.anchors {
            let (idx, prefix) = locate(&pkg, a, prev, &t.id).map_err(|e| Error::new(e.code, format!("card {}: {}", card.id, e.detail)))?;
            if !used.insert(idx) {
                return Err(bad("target_overlap", format!("target {} node {idx}", t.id)));
            }
            let n = &pkg.inspection.text_nodes[idx];
            nodes.push(BoundNode { node_id: n.id.clone(), node_index: idx, prefix, template_text: n.text.clone() });
            prev = Some(idx);
        }
        targets.push(BoundTarget {
            id: t.id.clone(),
            nodes,
            fmt,
            wrap_width: t.wrap_width,
            continuation_indent: t.continuation_indent.clone(),
        });
    }
    // whole-cell targets and repeated rows
    let check_fmt = |id: &str, fmt: &[Tok], item: Option<&crate::card::Slot>| -> Result<()> {
        let mut refs = Vec::new();
        fmt_refs(fmt, &mut refs);
        for (r, f) in &refs {
            let ok = match r.strip_prefix("item.") {
                Some(fld) => item.is_some_and(|s| s.fields.iter().any(|x| x.id == fld)) || (item.is_none() && false),
                None => known.contains(r),
            };
            if !ok {
                return Err(bad("unknown_reference", format!("{id}: {r}")));
            }
            if f.as_deref().is_some_and(|f| !FILTERS.contains(&f)) {
                return Err(bad("unknown_filter", format!("{id}: {f:?}")));
            }
            if slot_style(&card, defaults, r).is_some() {
                return Err(bad("unsupported_style_layout", format!("{id}: styled slot in a cell")));
            }
        }
        Ok(())
    };
    let ntables = pkg.inspection.tables.len();
    let dims = |t: usize| -> Option<(usize, usize)> {
        let a = &pkg.inspection.tables.get(t)?.attributes;
        Some((a.get("rowCnt")?.parse().ok()?, a.get("colCnt")?.parse().ok()?))
    };
    let grid = if card.cells.iter().any(|c| c.label.is_some()) { pkg.table_cells().map_err(|c| bad("template_unreadable", c.to_string()))? } else { Vec::new() };
    let squash = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    // label cells of every label anchor: a value cell may not land on one (e.g. `below` into the next label)
    let label_cells: BTreeSet<(usize, usize, usize)> = card
        .cells
        .iter()
        .filter_map(|c| c.label.as_ref().map(|l| (c, squash(l))))
        .filter_map(|(c, want)| {
            // unique matches only; an ambiguous label is reported as such below
            let m: Vec<_> = grid.iter().filter(|g| c.table.is_none_or(|t| t == g.table) && !g.nested && squash(&g.text) == want).collect();
            (m.len() == 1).then(|| (m[0].table, m[0].row, m[0].col))
        })
        .collect();
    let mut cells = Vec::new();
    let mut cell_ids = BTreeSet::new();
    for c in &card.cells {
        let mut c = c.clone();
        match (&c.label, c.row, c.col) {
            (Some(l), None, None) => {
                let want = squash(l);
                let hits: Vec<&oh_document::GridCell> = grid.iter().filter(|g| c.table.is_none_or(|t| t == g.table) && !g.nested && squash(&g.text) == want).collect();
                let lc = match hits.as_slice() {
                    [one] => *one,
                    [] => return Err(bad("anchor_not_found", format!("cell {}: label {l}", c.id))),
                    _ => return Err(bad("anchor_ambiguous", format!("cell {}: label {l} ({}곳, table로 좁힌다)", c.id, hits.len()))),
                };
                let (r, k) = match c.dir.as_deref().unwrap_or("right") {
                    "right" => (lc.row, lc.col + lc.col_span),
                    "below" => (lc.row + lc.row_span, lc.col),
                    d => return Err(bad("invalid_anchor", format!("cell {}: dir {d}", c.id))),
                };
                let v = grid.iter().find(|g| g.table == lc.table && g.row == r && g.col == k).ok_or_else(|| bad("anchor_not_found", format!("cell {}: no cell {} of label {l}", c.id, c.dir.as_deref().unwrap_or("right"))))?;
                if v.nested {
                    return Err(bad("anchor_not_editable", format!("cell {}: value cell holds a table", c.id)));
                }
                if label_cells.contains(&(v.table, v.row, v.col)) {
                    return Err(bad("target_overlap", format!("cell {}: value cell ({}, {}) is another label", c.id, v.row, v.col)));
                }
                c.table = Some(lc.table);
                c.row = Some(r);
                c.col = Some(k);
            }
            (None, Some(_), Some(_)) if c.table.is_some() && c.dir.is_none() => {}
            _ => return Err(bad("invalid_anchor", format!("cell {}: table+row+col, or label (+table, dir)", c.id))),
        }
        let (t, row, col) = (c.table.unwrap(), c.row.unwrap(), c.col.unwrap());
        if !tids.insert(c.id.clone()) || !cell_ids.insert((t, row, col)) {
            return Err(bad("duplicate_id", format!("cell {}", c.id)));
        }
        let (r, k) = dims(t).ok_or_else(|| bad("unknown_table", format!("cell {}: table {} of {ntables}", c.id, t)))?;
        if row >= r || col >= k {
            return Err(bad("unknown_cell", format!("cell {}", c.id)));
        }
        let f = parse_fmt(&c.fmt)?;
        check_fmt(&c.id, &f, None)?;
        cells.push((c, f));
    }
    let mut rows = Vec::new();
    for rt in &card.rows {
        if !tids.insert(rt.id.clone()) {
            return Err(bad("duplicate_id", format!("rows {}", rt.id)));
        }
        let (r, k) = dims(rt.table).ok_or_else(|| bad("unknown_table", format!("rows {}", rt.id)))?;
        if rt.template_rows == 0 || rt.first + rt.template_rows > r || rt.cols.len() != k {
            return Err(bad("invalid_rows", format!("{}: rows {}+{} of {r}, cols {} of {k}", rt.id, rt.first, rt.template_rows, rt.cols.len())));
        }
        let item = card.slot(&rt.list).filter(|s| s.kind == Kind::List).ok_or_else(|| bad("invalid_rows", format!("{}: list must be a list slot", rt.id)))?;
        if cells.iter().any(|(c, _)| c.table == Some(rt.table) && c.row.is_some_and(|r| r >= rt.first)) {
            return Err(bad("invalid_rows", format!("{}: cell targets below repeated rows", rt.id)));
        }
        let mut fs = Vec::new();
        for col in &rt.cols {
            let f = parse_fmt(col)?;
            check_fmt(&rt.id, &f, Some(item))?;
            fs.push(f);
        }
        rows.push((rt.clone(), fs));
    }
    let mut xlsx_cells = Vec::new();
    if let Some(x) = &card.xlsx {
        for c in &x.cells {
            let f = parse_fmt(&c.fmt)?;
            check_fmt(&format!("xlsx {}", c.at), &f, None)?;
            if c.number && !matches!(f.as_slice(), [Tok::Ph(_, None)]) {
                return Err(bad("invalid_xlsx_cell", format!("{}: number cell takes one {{id}}", c.at)));
            }
            xlsx_cells.push((c.clone(), f));
        }
    }
    let baseline_diagnostics = diagnostic_counts(&pkg);
    Ok(BoundCard { card, template, xlsx_template: None, cells, rows, xlsx_cells, targets, calcs, rules, baseline_diagnostics })
}
