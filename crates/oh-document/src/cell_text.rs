//! Whole-cell text (C1): write lines into a plain table cell, including a native empty cell
//! (`<hp:run charPrIDRef=".."/>` with no `<hp:t>`). Cells are addressed by table object id and
//! the cell's own `cellAddr`, so earlier edits that shift text-node ids do not move targets.
//! Only cells whose content is paragraphs of plain runs/text are written; anything else
//! (nested tables, controls, pictures) is refused, never discarded.
use crate::{escape_xml, parse_section, tag_end, valid_xml_char, Package, Result, HP};
use crate::styles::patch;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTextSpec {
    pub revision: String,
    pub operations: Vec<CellText>,
}
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CellText {
    /// Table object id from inspection (`Contents/section0.xml#tblN`).
    pub table: String,
    pub row: usize,
    pub col: usize,
    /// One paragraph per line. Empty list = one empty paragraph.
    pub lines: Vec<String>,
}

fn doc(xml: &str) -> Result<roxmltree::Document<'_>> {
    roxmltree::Document::parse(xml).map_err(|_| "invalid_section_xml")
}

/// Replacement for the cell's paragraphs, or None when nothing changes.
/// `next_id`: fresh paragraph ids for lines after the first (the first keeps the cell's id).
fn cell_edit(xml: &str, tc: roxmltree::Node, lines: &[String], next_id: &mut u32) -> Result<(std::ops::Range<usize>, String)> {
    let list = tc.children().find(|n| n.has_tag_name((HP, "subList"))).ok_or("unsupported_table_structure")?;
    let paras: Vec<_> = list.children().filter(|n| n.is_element()).collect();
    if paras.is_empty() || paras.iter().any(|p| !p.has_tag_name((HP, "p"))) {
        return Err("cell_has_objects");
    }
    for p in &paras {
        for c in p.children().filter(|n| n.is_element()) {
            match c.tag_name().name() {
                "run" if c.tag_name().namespace() == Some(HP) => {
                    for x in c.children().filter(|n| n.is_element()) {
                        // plain text only; a control/object inside the run is content we must keep
                        if !(x.has_tag_name((HP, "t")) && x.children().all(|y| y.is_text())) {
                            return Err("cell_has_objects");
                        }
                    }
                }
                "linesegarray" if c.tag_name().namespace() == Some(HP) => {}
                _ => return Err("cell_has_objects"),
            }
        }
    }
    let first = paras[0];
    let raw = &xml[first.range()];
    let open = &raw[..tag_end(raw).ok_or("invalid_section_xml")?];
    let open = open.strip_suffix("/>").map(|s| format!("{s}>")).unwrap_or_else(|| open.to_string());
    let char_pr = first
        .children()
        .find(|n| n.has_tag_name((HP, "run")))
        .and_then(|r| r.attribute("charPrIDRef"))
        .unwrap_or("0");
    if !char_pr.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid_section_xml");
    }
    let mut out = String::new();
    let empty = [String::new()];
    let lines = if lines.is_empty() { &empty[..] } else { lines };
    let first_id = first.attribute("id");
    for (k, l) in lines.iter().enumerate() {
        let open = if k == 0 || first_id.is_none() {
            open.clone()
        } else {
            let id = *next_id;
            *next_id = next_id.checked_add(1).ok_or("excess_object_ids")?;
            open.replacen(&format!("id=\"{}\"", first_id.unwrap()), &format!("id=\"{id}\""), 1)
        };
        if !l.chars().all(valid_xml_char) || l.contains('\n') || l.contains('\r') {
            return Err("invalid_xml_character");
        }
        if l.is_empty() {
            out.push_str(&format!("{open}<hp:run charPrIDRef=\"{char_pr}\"/></hp:p>"));
        } else {
            out.push_str(&format!("{open}<hp:run charPrIDRef=\"{char_pr}\"><hp:t>{}</hp:t></hp:run></hp:p>", escape_xml(l)));
        }
    }
    let start = paras.first().unwrap().range().start;
    let end = paras.last().unwrap().range().end;
    Ok((start..end, out))
}

impl Package {
    pub fn cell_text_bytes(&self, spec: CellTextSpec) -> Result<crate::Prepared> {
        if spec.revision != self.inspection.revision {
            return Err("stale_document");
        }
        if spec.operations.is_empty() || spec.operations.len() > 2000 {
            return Err("invalid_operations");
        }
        let mut by_part: BTreeMap<String, Vec<CellText>> = BTreeMap::new();
        for op in spec.operations {
            let obj = self.inspection.tables.iter().find(|n| n.id == op.table).ok_or("unknown_target")?;
            by_part.entry(obj.part.clone()).or_default().push(op);
        }
        let mut changed = BTreeMap::new();
        let mut next_id = self.next_object_id()?;
        for (part, ops) in by_part {
            let xml = std::str::from_utf8(&self.parts.iter().find(|(n, _)| n == &part).ok_or("unknown_target")?.1).map_err(|_| "unsupported_xml_encoding")?;
            let d = doc(xml)?;
            let tables: Vec<_> = d.descendants().filter(|n| n.has_tag_name((HP, "tbl"))).collect();
            let mut edits = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for op in ops {
                let index: usize = op.table.rsplit_once("#tbl").ok_or("unknown_target")?.1.parse().map_err(|_| "unknown_target")?;
                if !seen.insert((index, op.row, op.col)) {
                    return Err("duplicate_target");
                }
                let tbl = *tables.get(index).ok_or("unknown_target")?;
                let tc = tbl
                    .children()
                    .filter(|n| n.has_tag_name((HP, "tr")))
                    .flat_map(|tr| tr.children().filter(|n| n.has_tag_name((HP, "tc"))))
                    .find(|tc| {
                        tc.children().find(|n| n.has_tag_name((HP, "cellAddr"))).is_some_and(|a| {
                            a.attribute("rowAddr").and_then(|v| v.parse::<usize>().ok()) == Some(op.row)
                                && a.attribute("colAddr").and_then(|v| v.parse::<usize>().ok()) == Some(op.col)
                        })
                    })
                    .ok_or("unknown_cell")?;
                edits.push(cell_edit(xml, tc, &op.lines, &mut next_id)?);
            }
            let out = patch(xml, edits)?;
            parse_section(&part, out.as_bytes())?;
            changed.insert(part, out.into_bytes());
        }
        self.prepare_changed(changed, false)
    }
}

/// One table cell of `Contents/section0.xml` as laid out in the template (label anchors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridCell {
    /// Table index (`#tblN`, document order, nested tables included).
    pub table: usize,
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    /// The cell's own paragraphs (text of nested tables excluded), one line each.
    pub text: String,
    /// The cell holds a nested table.
    pub nested: bool,
}

impl Package {
    /// Cells of every table in the first section, for label-based cell anchors.
    pub fn table_cells(&self) -> Result<Vec<GridCell>> {
        let part = "Contents/section0.xml";
        let xml = std::str::from_utf8(&self.parts.iter().find(|(n, _)| n == part).ok_or("unknown_target")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let d = doc(xml)?;
        let tables: Vec<_> = d.descendants().filter(|n| n.has_tag_name((HP, "tbl"))).collect();
        let mut out = Vec::new();
        for (ti, tbl) in tables.iter().enumerate() {
            for tc in tbl.children().filter(|n| n.has_tag_name((HP, "tr"))).flat_map(|tr| tr.children().filter(|n| n.has_tag_name((HP, "tc")))) {
                let num = |tag: &str, a: &str| tc.children().find(|n| n.has_tag_name((HP, tag))).and_then(|n| n.attribute(a)).and_then(|v| v.parse::<usize>().ok());
                let (Some(row), Some(col)) = (num("cellAddr", "rowAddr"), num("cellAddr", "colAddr")) else { continue };
                let own = |n: &roxmltree::Node| !n.ancestors().take_while(|a| a != &tc).any(|a| a.has_tag_name((HP, "tbl")));
                let mut lines = Vec::new();
                let mut nested = false;
                if let Some(list) = tc.children().find(|n| n.has_tag_name((HP, "subList"))) {
                    for p in list.children().filter(|n| n.has_tag_name((HP, "p"))) {
                        nested |= p.descendants().any(|n| n.has_tag_name((HP, "tbl")));
                        let t: String = p.descendants().filter(|n| n.has_tag_name((HP, "t")) && own(n)).flat_map(|n| n.descendants().filter(|x| x.is_text()).map(|x| x.text().unwrap_or(""))).collect();
                        lines.push(t);
                    }
                }
                out.push(GridCell { table: ti, row, col, row_span: num("cellSpan", "rowSpan").unwrap_or(1), col_span: num("cellSpan", "colSpan").unwrap_or(1), text: lines.join("\n"), nested });
            }
        }
        Ok(out)
    }
}

impl Package {
    /// Drop the cached line layout (`hp:linesegarray`) of every paragraph whose cached line
    /// starts point at or past the end of its text. Shortened text leaves such stale entries,
    /// and Hancom 2022 refuses to open the file (`Open` returns false); without the cache it
    /// lays the paragraph out again. Paragraphs with valid caches are left byte-identical.
    /// Returns None when nothing is stale.
    pub fn drop_stale_linesegs(&self) -> Result<Option<crate::Prepared>> {
        let mut changed = BTreeMap::new();
        for (part, bytes) in &self.parts {
            if !(part.starts_with("Contents/section") && part.ends_with(".xml")) {
                continue;
            }
            let xml = std::str::from_utf8(bytes).map_err(|_| "unsupported_xml_encoding")?;
            let d = doc(xml)?;
            let mut edits = Vec::new();
            for p in d.descendants().filter(|n| n.has_tag_name((HP, "p"))) {
                let Some(segs) = p.children().find(|n| n.has_tag_name((HP, "linesegarray"))) else { continue };
                // only plain-text paragraphs: controls (tables, pictures) and inline tab/lineBreak take
                // text positions we do not count, so such a paragraph is never judged stale
                let plain = p.children().filter(|n| n.has_tag_name((HP, "run"))).all(|r| {
                    r.children().filter(|n| n.is_element()).all(|x| x.has_tag_name((HP, "t")) && x.children().all(|y| y.is_text()))
                });
                if !plain {
                    continue;
                }
                // own text only: runs of this paragraph, not of paragraphs nested in its tables
                let len: usize = p
                    .children()
                    .filter(|n| n.has_tag_name((HP, "run")))
                    .flat_map(|r| r.children().filter(|n| n.has_tag_name((HP, "t"))))
                    .flat_map(|t| t.descendants().filter(|x| x.is_text()).map(|x| x.text().unwrap_or("").chars().count()))
                    .sum();
                let stale = segs.children().filter(|n| n.has_tag_name((HP, "lineseg"))).any(|s| {
                    s.attribute("textpos").and_then(|v| v.parse::<usize>().ok()).is_some_and(|tp| tp > 0 && tp >= len)
                });
                if stale {
                    edits.push((segs.range(), String::new()));
                }
            }
            if !edits.is_empty() {
                let out = patch(xml, edits)?;
                parse_section(part, out.as_bytes())?;
                changed.insert(part.clone(), out.into_bytes());
            }
        }
        if changed.is_empty() {
            return Ok(None);
        }
        self.prepare_changed(changed, false).map(Some)
    }
}
