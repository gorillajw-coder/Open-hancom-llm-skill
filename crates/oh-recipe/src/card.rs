//! Card TOML model. Cards are declarative; the engine executes them deterministically.
use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub version: String,
    /// Common frame (`frames/<id>.toml`) whose slots/calcs/targets/cells/rules this card inherits.
    /// A card entry with the same id replaces the frame's; `omit` drops frame entries.
    pub extends: Option<String>,
    /// Lowest core version (`x.y.z`) the card needs; an older core refuses the card.
    pub requires_core: Option<String>,
    #[serde(default)]
    pub omit: Vec<String>,
    /// Project facts (D11): slot id here = fact key in the project record.
    pub project: Option<ProjectMap>,
    pub template: String,
    pub template_sha256: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub next: Vec<String>,
    #[serde(default)]
    pub follows: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default, rename = "slot")]
    pub slots: Vec<Slot>,
    #[serde(default, rename = "calc")]
    pub calcs: Vec<Calc>,
    #[serde(default, rename = "target")]
    pub targets: Vec<Target>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<Rule>,
    /// Whole-cell targets (C1): write into a table cell by address, empty cells included.
    #[serde(default, rename = "cell")]
    pub cells: Vec<CellTarget>,
    /// Repeated table rows from a list slot (C3).
    #[serde(default, rename = "rows")]
    pub rows: Vec<RowsTarget>,
    /// Values read from an attached XLSX (C7).
    #[serde(default, rename = "extract")]
    pub extracts: Vec<Extract>,
    /// Slots taken from an earlier document of the same plan when not given (C6).
    #[serde(default)]
    pub carry: Option<Carry>,
    /// Second output: an XLSX filled from slots/calcs (C8).
    pub xlsx: Option<XlsxOut>,
    /// Regulation table id used by `grade`/`rate`/`discount` calc functions.
    pub regulation: Option<String>,
    /// Set by `with_frames` (never read from TOML): the frame named by `extends` is merged in.
    #[serde(skip)]
    pub frame_merged: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTarget {
    pub id: String,
    /// Table object index in the section (`#tblN`, document order, nested tables included).
    /// With `label`: only search this table.
    pub table: Option<usize>,
    pub row: Option<usize>,
    pub col: Option<usize>,
    /// Label anchor: the cell whose own text equals this label (whitespace ignored, must be
    /// unique); the value cell is the next one to the right (or below, `dir = "below"`).
    /// Survives templates whose head tables have different column counts.
    pub label: Option<String>,
    pub dir: Option<String>,
    /// Same format language as targets. `\n` in the result starts a new paragraph.
    pub fmt: String,
    /// Leave the cell as it is in the template when the format does not resolve.
    #[serde(default)]
    pub keep_if_unresolved: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowsTarget {
    pub id: String,
    pub table: usize,
    /// List slot (or calc returning a JSON list) that drives the rows.
    pub list: String,
    /// Template row copied for each item; rows `first..first+template_rows` are replaced.
    pub first: usize,
    pub template_rows: usize,
    /// One format per column; `{item.field|filter}` reads the current item.
    pub cols: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extract {
    /// Calc-like id the value is published under.
    pub id: String,
    pub label: String,
    /// Slot of kind `file` that names the XLSX.
    pub from: String,
    pub sheet: String,
    /// Section label in `label_col` (spaces ignored); items run until a row whose
    /// `total_col` reads `total_marker`.
    pub section: String,
    pub label_col: String,
    pub total_col: String,
    pub total_marker: String,
    pub value_col: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Carry {
    pub from_card: String,
    /// slot id here = slot or calc id in the earlier document
    pub map: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectMap {
    pub map: BTreeMap<String, String>,
}

/// Common frame (e.g. `approval-memo`): the shared head table, body skeleton and attachments
/// of one family of documents. It has no template of its own; each card brings its template.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub id: String,
    pub title: String,
    pub version: String,
    #[serde(default, rename = "slot")]
    pub slots: Vec<Slot>,
    #[serde(default, rename = "calc")]
    pub calcs: Vec<Calc>,
    #[serde(default, rename = "target")]
    pub targets: Vec<Target>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<Rule>,
    #[serde(default, rename = "cell")]
    pub cells: Vec<CellTarget>,
    pub project: Option<ProjectMap>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XlsxOut {
    pub template: String,
    pub template_sha256: String,
    pub sheet: String,
    #[serde(rename = "cell")]
    pub cells: Vec<XlsxCell>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XlsxCell {
    /// A1 address.
    pub at: String,
    /// Text format, or a single `{id}` written as a number when `number = true`.
    pub fmt: String,
    #[serde(default)]
    pub number: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: Kind,
    #[serde(default)]
    pub required: bool,
    pub max_chars: Option<usize>,
    /// Allowed values (text fields only).
    #[serde(default)]
    pub one_of: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Person,
    Prose,
    Money,
    Decimal,
    Date,
    Email,
    Phone,
    /// JSON array of objects with `fields`.
    List,
    /// Attachment file name (inbox) read by the host; the core reads its bytes.
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    pub color: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub id: String,
    pub label: String,
    pub kind: Kind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub require_source: bool,
    pub default: Option<String>,
    pub default_ref: Option<String>,
    pub default_style: Option<Style>,
    pub guide: Option<String>,
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub forbidden: Vec<String>,
    /// Computed default (core), e.g. `date_add(doc_date, 14)`. Marked as a default.
    pub default_expr: Option<String>,
    /// Prose only: line breaks kept (cell targets render one paragraph per line).
    #[serde(default)]
    pub multiline: bool,
    #[serde(default, rename = "field")]
    pub fields: Vec<Field>,
    pub max_items: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calc {
    pub id: String,
    pub label: String,
    pub expr: String,
    /// Slot holding the source-document value to compare against. A mismatch only warns.
    pub compare_to: Option<String>,
}

/// Where a value lands in the template. Exactly one of `starts`, `after`, `follow`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    /// Text node whose text starts with this string (must be unique).
    pub starts: Option<String>,
    /// Text node `n` positions after the unique node whose trimmed text starts with this label.
    pub after: Option<String>,
    pub n: Option<usize>,
    /// Text node `follow` positions after the previous anchor of the same target.
    pub follow: Option<usize>,
    /// With `starts`: keep the matched prefix and write the value after it (default true).
    pub keep_prefix: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub id: String,
    #[serde(rename = "anchor")]
    pub anchors: Vec<Anchor>,
    /// `{slot}`, `{slot|filter}`, `[optional {slot}]`. Filters: usd, krw, date_kr, date_kr0, date_dot.
    pub fmt: String,
    pub wrap_width: Option<usize>,
    #[serde(default)]
    pub continuation_indent: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Block,
    Warn,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    /// Guard condition; default `always`.
    pub when: Option<String>,
    #[serde(default)]
    pub require: Vec<String>,
    /// Condition that must hold when the guard holds.
    pub assert: Option<String>,
    pub severity: Severity,
    pub cite: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultEntry {
    pub value: String,
    pub style: Option<Style>,
}

/// `library/defaults.toml` (DECISIONS D6-2).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub defaults: BTreeMap<String, DefaultEntry>,
}

impl Card {
    pub fn from_toml(src: &str) -> Result<Card> {
        toml::from_str(src).map_err(|e| Error::new("invalid_card", e.to_string()))
    }
    pub fn slot(&self, id: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.id == id)
    }
}
/// Frame entries not replaced by the card come first, in frame order; then the card's own.
fn inherit<T: Clone>(frame: &[T], card: &mut Vec<T>, id: impl Fn(&T) -> &str, omit: &[String]) {
    let mut out: Vec<T> = frame.iter().filter(|f| !omit.iter().any(|o| o == id(f)) && !card.iter().any(|c| id(c) == id(f))).cloned().collect();
    out.append(card);
    *card = out;
}

impl Frame {
    pub fn from_toml(src: &str) -> Result<Frame> {
        toml::from_str(src).map_err(|e| Error::new("invalid_frame", e.to_string()))
    }
    fn ids(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.slots.iter().map(|x| x.id.as_str()).collect();
        v.extend(self.calcs.iter().map(|x| x.id.as_str()));
        v.extend(self.targets.iter().map(|x| x.id.as_str()));
        v.extend(self.rules.iter().map(|x| x.id.as_str()));
        v.extend(self.cells.iter().map(|x| x.id.as_str()));
        v
    }
}

impl Card {
    /// Merge the frame named by `extends`. Cards without `extends` are returned unchanged.
    pub fn with_frames(mut self, frames: &[Frame]) -> Result<Card> {
        let Some(fid) = self.extends.clone() else {
            if !self.omit.is_empty() {
                return Err(Error::new("invalid_card", format!("{}: omit without extends", self.id)));
            }
            return Ok(self);
        };
        let f = frames.iter().find(|f| f.id == fid).ok_or_else(|| Error::new("unknown_frame", format!("card {}: {fid}", self.id)))?;
        let ids = f.ids();
        if let Some(o) = self.omit.iter().find(|o| !ids.contains(&o.as_str())) {
            return Err(Error::new("invalid_card", format!("{}: omit {o} is not in frame {fid}", self.id)));
        }
        let om = self.omit.clone();
        inherit(&f.slots, &mut self.slots, |x| &x.id, &om);
        inherit(&f.calcs, &mut self.calcs, |x| &x.id, &om);
        inherit(&f.targets, &mut self.targets, |x| &x.id, &om);
        inherit(&f.rules, &mut self.rules, |x| &x.id, &om);
        inherit(&f.cells, &mut self.cells, |x| &x.id, &om);
        if let Some(fp) = &f.project {
            let mut map: BTreeMap<String, String> = fp.map.iter().filter(|(k, _)| !om.contains(k) && self.slot(k).is_some()).map(|(k, v)| (k.clone(), v.clone())).collect();
            if let Some(cp) = &self.project {
                map.extend(cp.map.clone());
            }
            self.project = Some(ProjectMap { map });
        }
        self.frame_merged = true;
        Ok(self)
    }
}

impl Defaults {
    pub fn from_toml(src: &str) -> Result<Defaults> {
        toml::from_str(src).map_err(|e| Error::new("invalid_defaults", e.to_string()))
    }
}
