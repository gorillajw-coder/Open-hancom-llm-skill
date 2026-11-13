//! Local-only, preservation-first HWPX package engine. No layout guarantee.
mod zip_preserve;
mod anonymize;
pub use anonymize::{AUTHOR_PLACEHOLDER, BLANK_PREVIEW_PNG};
mod styles;
pub use styles::FormatSpec;
mod paragraph;
mod line_spacing;
pub use paragraph::ParagraphSpec;
mod create;
pub use create::{CreateSpec,create_bytes};
mod table;
pub use table::TableSpec;
mod cell_text;
pub use cell_text::{CellText,CellTextSpec,GridCell};
mod schema;
pub use schema::discovery;
mod named_style;
pub use named_style::StyleSpec;
mod border;
pub use border::BorderSpec;
mod diagnostics;
mod native_lists;
mod package_model;
mod inline;
mod jpeg;
mod inline_edit;
pub use inline_edit::{ImageEdit,LinkEdit};
mod media_edit;
pub use media_edit::{ImageReplace,ImageInsert};
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, BTreeMap};
use std::io::Write;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::{Cursor, Read}};

pub type Result<T> = std::result::Result<T, &'static str>;
/// Admission for strict JSON CLI contracts: no nullable fields or duplicate fields.
pub fn parse_contract<T:serde::de::DeserializeOwned>(raw:&[u8],code:&'static str)->Result<T> {
    fn has_null(v:&serde_json::Value)->bool {match v {
        serde_json::Value::Null=>true,
        serde_json::Value::Array(a)=>a.iter().any(has_null),
        serde_json::Value::Object(o)=>o.values().any(has_null),_=>false,
    }}
    let value:serde_json::Value=serde_json::from_slice(raw).map_err(|_|code)?;
    if has_null(&value) {return Err(code);}
    // Deserialize original tokens, not Value: serde's duplicate-field checks stay active.
    serde_json::from_slice(raw).map_err(|_|code)
}
const HP: &str = "http://www.hancom.co.kr/hwpml/2011/paragraph";
/// Maximum HWPX package size accepted as input or produced as output.
pub const MAX_INPUT: u64 = 64 * 1024 * 1024;
/// Maximum JSON contract (plan/spec) size accepted by the CLI.
pub const MAX_CONTRACT: u64 = 2 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;
const MAX_ENTRY: u64 = 32 * 1024 * 1024;

#[derive(Serialize, Clone)]
pub struct TextNode {
    pub id: String,
    pub part: String,
    pub text: String,
    pub in_table: bool,
    pub editable: bool,
    #[serde(skip)]
    range: std::ops::Range<usize>,
}
#[derive(Serialize)]
pub struct Inspection {
    pub revision: String,
    pub text_nodes: Vec<TextNode>,
    pub paragraphs: Vec<ObjectNode>,
    pub tables: Vec<ObjectNode>,
    pub cells: Vec<ObjectNode>,
    pub images: Vec<serde_json::Value>,
    pub hyperlinks: Vec<serde_json::Value>,
    pub section_count: usize,
    pub member_count: usize,
    pub limitations: Vec<&'static str>,
}
#[derive(Serialize)]
pub struct ObjectNode {
    pub id: String,
    pub part: String,
    pub in_table: bool,
    pub attributes: BTreeMap<String,String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub direct_run_count: Option<usize>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub direct_runs: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub native_list: Option<serde_json::Value>,
}
fn objects(name: &str, bytes: &[u8], tag: &str, prefix: &str) -> Result<Vec<ObjectNode>> {
    let xml=std::str::from_utf8(bytes).map_err(|_| "unsupported_xml_encoding")?;
    let doc=roxmltree::Document::parse(xml).map_err(|_| "invalid_section_xml")?;
    Ok(doc.descendants().filter(|n| n.has_tag_name((HP,tag))).enumerate().map(|(i,n)| ObjectNode {
        id:format!("{name}#{prefix}{i}"),part:name.to_string(),
        in_table:n.ancestors().skip(1).any(|a| a.has_tag_name((HP,"tbl"))),
        attributes:n.attributes().filter(|a| a.namespace().is_none()).map(|a| (a.name().to_string(),a.value().to_string())).collect(),
        native_list:None,
        direct_run_count:if tag=="p" {Some(n.children().filter(|c|c.has_tag_name((HP,"run"))).count())}else{None},
        direct_runs:if tag=="p" {Some(n.children().filter(|c|c.has_tag_name((HP,"run"))).enumerate().map(|(index,r)| {
            let fields:Vec<_>=r.descendants().filter(|f|f.ancestors().skip(1).find(|a|a.has_tag_name((HP,"p")))==Some(n)).collect();
            serde_json::json!({"index":index,"char_pr_id":r.attribute("charPrIDRef"),
                "children":r.children().filter(|c|c.is_element()).map(|c|c.tag_name().name()).collect::<Vec<_>>(),
                "field_begins":fields.iter().filter(|f|f.has_tag_name((HP,"fieldBegin"))).map(|f|serde_json::json!({"id":f.attribute("id"),"type":f.attribute("type")})).collect::<Vec<_>>(),
                "field_ends":fields.iter().filter(|f|f.has_tag_name((HP,"fieldEnd"))).map(|f|f.attribute("beginIDRef")).collect::<Vec<_>>()})
        }).collect())}else{None},
    }).collect())
}
/// A prepared output: complete package bytes plus a receipt. The receipt never
/// contains a path or publication state; the host adds those after publishing.
#[derive(Debug)]
pub struct Prepared {
    pub bytes: Vec<u8>,
    pub receipt: serde_json::Value,
}
pub struct Package {
    raw: Vec<u8>,
    parts: Vec<(String, Vec<u8>)>,
    pub inspection: Inspection,
}
fn is_section(name: &str) -> bool {
    name.strip_prefix("Contents/section")
        .and_then(|s| s.strip_suffix(".xml"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
pub(crate) fn parse_section(name: &str, bytes: &[u8]) -> Result<Vec<TextNode>> {
    let xml = std::str::from_utf8(bytes).map_err(|_| "unsupported_xml_encoding")?;
    // roxmltree's default parser rejects DTD; no entity resolver/network access.
    let doc = roxmltree::Document::parse(xml).map_err(|_| "invalid_section_xml")?;
    let mut rows = Vec::new();
    for (i, n) in doc.descendants().filter(|n| n.has_tag_name((HP, "t"))).enumerate() {
        rows.push(TextNode {
            id: format!("{name}#t{i}"), part: name.into(),
            text: n.descendants().filter(|x| x.is_text()).filter_map(|x| x.text()).collect(),
            in_table: n.ancestors().any(|x| x.has_tag_name((HP, "tbl"))),
            editable: n.children().all(|x| x.is_text()),
            range: n.range(),
        });
    }
    Ok(rows)
}
impl Package {
    fn next_object_id(&self)->Result<u32> {
        let mut max_id=0u32;
        for (name,data) in &self.parts {if is_section(name) {
            let xml=std::str::from_utf8(data).map_err(|_|"invalid_xml")?;
            let doc=roxmltree::Document::parse(xml).map_err(|_|"invalid_xml")?;
            for n in doc.descendants() {for a in n.attributes() {
                // Include opaque numeric references: allocating their value
                // would silently turn a dangling vendor reference into a link.
                if (["id","instid","fieldid"].contains(&a.name()) || a.name().ends_with("IDRef")) && !a.value().is_empty() && a.value().bytes().all(|b|b.is_ascii_digit()) {
                    max_id=max_id.max(a.value().parse().map_err(|_|"excess_object_ids")?);
                }
            }}
        }}
        max_id.checked_add(1).ok_or("excess_object_ids")
    }
    /// Parse and admit an HWPX package from bytes (no I/O).
    pub fn from_bytes(raw: Vec<u8>) -> Result<Self> { Self::from_raw(raw) }
    fn from_raw(raw: Vec<u8>) -> Result<Self> {
        if raw.len() as u64 > MAX_INPUT { return Err("input_too_large"); }
        let revision = format!("{:x}", Sha256::digest(&raw));
        let mut z = zip::ZipArchive::new(Cursor::new(&raw)).map_err(|_| "invalid_zip")?;
        if z.len() > 4096 { return Err("excess_entries"); }
        // zip 2.x indexes by name and silently collapses duplicate central entries.
        // Count the bounded raw central headers before trusting its deduplicated view.
        let mut cursor = usize::try_from(z.central_directory_start()).map_err(|_| "invalid_zip")?;
        let mut central_entries = 0;
        while raw.get(cursor..cursor.saturating_add(4)) == Some(b"PK\x01\x02") {
            let h = raw.get(cursor..cursor.checked_add(46).ok_or("invalid_zip")?).ok_or("invalid_zip")?;
            let length = |i| u16::from_le_bytes([h[i], h[i + 1]]) as usize;
            cursor = cursor.checked_add(46 + length(28) + length(30) + length(32)).ok_or("invalid_zip")?;
            if cursor > raw.len() { return Err("invalid_zip"); }
            central_entries += 1;
            if central_entries > 4096 { return Err("excess_entries"); }
        }
        if central_entries != z.len() { return Err("duplicate_or_invalid_central_entries"); }
        let mut names = HashSet::new();
        let mut total = 0u64;
        let mut parts = Vec::new();
        let mut nodes = Vec::new();
        let mut section_count = 0;
        for i in 0..z.len() {
            let mut entry = z.by_index(i).map_err(|_| "invalid_or_encrypted_zip")?;
            let name = entry.name().to_string();
            if !names.insert(name.clone()) { return Err("duplicate_entry"); }
            if name.is_empty() || name.starts_with('/') || name.contains(['\\', '\0'])
                || name.split('/').any(|s| s == "..") || entry.enclosed_name().is_none()
                || entry.is_symlink() {
                return Err("unsafe_member_path");
            }
            if entry.size() > MAX_ENTRY { return Err("entry_too_large"); }
            if entry.size() > entry.compressed_size().max(1).saturating_mul(200) {
                return Err("compression_ratio_exceeded");
            }
            total = total.checked_add(entry.size()).ok_or("expanded_package_too_large")?;
            if total > MAX_TOTAL { return Err("expanded_package_too_large"); }
            let mut bytes = Vec::new();
            (&mut entry).take(MAX_ENTRY + 1).read_to_end(&mut bytes).map_err(|_| "invalid_zip_data")?;
            if bytes.len() as u64 != entry.size() { return Err("invalid_zip_size"); }
            if is_section(&name) {
                nodes.extend(parse_section(&name, &bytes)?);
                section_count += 1;
            }
            parts.push((name, bytes));
        }
        if !parts.iter().any(|(n,b)| n == "mimetype" && b == b"application/hwp+zip") {
            return Err("not_hwpx");
        }
        if section_count == 0 { return Err("no_sections"); }
        let mut paragraphs=Vec::new();
        let mut tables=Vec::new();
        let mut cells=Vec::new();
        let mut images=Vec::new();
        let mut hyperlinks=Vec::new();
        let mut resources:BTreeMap<String,Vec<serde_json::Value>>=BTreeMap::new();
        if let Some((_,bytes))=parts.iter().find(|(n,_)|n=="Contents/content.hpf") {
            if let Ok(xml)=std::str::from_utf8(bytes) {if let Ok(doc)=roxmltree::Document::parse(xml) {
                for item in doc.descendants().filter(|n|n.has_tag_name((package_model::OPF,"item"))) {
                    if let Some(id)=item.attribute("id") {
                        let href=item.attribute("href").unwrap_or("");
                        let bytes=if package_model::safe_part(href) {parts.iter().find(|(n,_)|n==href).map(|(_,b)|b)}else{None};
                        let decoded=if item.attribute("media-type")==Some("image/jpeg") {bytes.map(|b|match jpeg::details(b) {
                            Ok(details)=>details,
                            Err(code)=>serde_json::json!({"validation":"rejected","code":code}),
                        })}else{None};
                        let mut resource=serde_json::json!({"part":href,
                            "sha256":bytes.map(|b|format!("{:x}",Sha256::digest(b))),"bytes":bytes.map(|b|b.len()),
                            "media_type":item.attribute("media-type"),"is_embedded":item.attribute("isEmbeded")});
                        if let Some(decoded)=decoded {resource["decoded_image"]=decoded;}
                        resources.entry(id.to_string()).or_default().push(resource);
                    }
                }
            }}
        }
        for (name,data) in &parts { if is_section(name) {
            paragraphs.extend(objects(name,data,"p","p")?);
            tables.extend(objects(name,data,"tbl","tbl")?);
            cells.extend(objects(name,data,"tc","tc")?);
            let xml=std::str::from_utf8(data).map_err(|_|"unsupported_xml_encoding")?;
            let doc=roxmltree::Document::parse(xml).map_err(|_|"invalid_section_xml")?;
            for (i,n) in doc.descendants().filter(|n|n.has_tag_name((HP,"pic"))).enumerate() {
                let item=n.children().find(|n|n.has_tag_name((package_model::HC,"img"))).and_then(|n|n.attribute("binaryItemIDRef"));
                images.push(serde_json::json!({"id":format!("{name}#pic{i}"),"part":name,
                    "binary_item_id":item,"resource":item.and_then(|id|resources.get(id)).filter(|v|v.len()==1).map(|v|&v[0]),
                    "width":n.children().find(|n|n.has_tag_name((HP,"sz"))).and_then(|n|n.attribute("width")),
                    "height":n.children().find(|n|n.has_tag_name((HP,"sz"))).and_then(|n|n.attribute("height"))}));
            }
            for (i,n) in doc.descendants().filter(|n|n.has_tag_name((HP,"fieldBegin")) && n.attribute("type")==Some("HYPERLINK")).enumerate() {
                hyperlinks.push(serde_json::json!({"id":format!("{name}#link{i}"),"part":name,
                    "url":n.descendants().find(|n|n.has_tag_name((HP,"stringParam")) && n.attribute("name")==Some("Path")).and_then(|n|n.text())}));
            }
        }}
        let mut picture_counts=BTreeMap::<String,usize>::new();
        for image in &images {if let Some(id)=image["binary_item_id"].as_str() {*picture_counts.entry(id.to_string()).or_default()+=1;}}
        for image in &mut images {image["picture_reference_count"]=serde_json::json!(image["binary_item_id"].as_str().and_then(|id|picture_counts.get(id)).copied().unwrap_or(0));}
        native_lists::bind(&mut paragraphs,&parts,&revision);
        let inspection = Inspection {
            revision, text_nodes: nodes, paragraphs, tables, cells, images, hyperlinks, section_count, member_count: parts.len(),
            limitations: vec!["Structural inspection only; native layout and full schema not validated.",
                "Rust handles are revision-bound and are not Python XPath handles."],
        };
        drop(z);
        Ok(Self {raw, parts, inspection})
    }

    /// `apply` text-replacement plan → prepared package bytes (no I/O).
    pub fn apply_bytes(&self, plan: Plan) -> Result<Prepared> {
        if plan.revision != self.inspection.revision { return Err("stale_document"); }
        if plan.operations.is_empty() || plan.operations.len() > 1000 { return Err("invalid_operations"); }
        let mut seen = HashSet::new();
        let mut edits: BTreeMap<String, Vec<(std::ops::Range<usize>, String)>> = BTreeMap::new();
        let parts: HashMap<_,_> = self.parts.iter().map(|(n,b)| (n.as_str(), b.as_slice())).collect();
        for op in plan.operations {
            if op.op != "replace_text" { return Err("invalid_operation"); }
            if !seen.insert(op.target.clone()) { return Err("duplicate_target"); }
            let n = self.inspection.text_nodes.iter().find(|n| n.id == op.target).ok_or("unknown_target")?;
            if !n.editable { return Err("inline_controls_require_structured_edit"); }
            if n.text != op.expected { return Err("stale_target"); }
            if !op.text.chars().all(valid_xml_char) { return Err("invalid_xml_character"); }
            if n.text == op.text { continue; }
            let xml = std::str::from_utf8(parts[n.part.as_str()]).map_err(|_| "unsupported_xml_encoding")?;
            let raw = &xml[n.range.clone()];
            let end = tag_end(raw).ok_or("invalid_section_xml")?;
            let start = &raw[..end];
            let text = escape_xml(&op.text);
            let replacement = if let Some(prefix) = start.strip_suffix("/>") {
                let name = start[1..].split([' ', '\t', '\r', '\n', '/', '>']).next().ok_or("invalid_section_xml")?;
                format!("{prefix}>{text}</{name}>")
            } else {
                let closing = raw.rfind("</").ok_or("invalid_section_xml")?;
                format!("{start}{text}{}", &raw[closing..])
            };
            edits.entry(n.part.clone()).or_default().push((n.range.clone(), replacement));
        }
        let mut changed = BTreeMap::new();
        for (part, mut spans) in edits {
            let mut xml = String::from_utf8(parts[part.as_str()].to_vec()).map_err(|_| "unsupported_xml_encoding")?;
            spans.sort_by_key(|(range,_)| range.start);
            if spans.windows(2).any(|s| s[0].0.end > s[1].0.start) { return Err("overlapping_targets"); }
            for (range, text) in spans.into_iter().rev() { xml.replace_range(range, &text); }
            parse_section(&part, xml.as_bytes())?;
            if xml.len() as u64 > MAX_ENTRY { return Err("entry_too_large"); }
            changed.insert(part, xml.into_bytes());
        }
        self.prepare_changed(changed, false)
    }

    /// Build, re-admit and describe the output package for `changed` parts (no I/O).
    fn prepare_changed(&self, changed:BTreeMap<String,Vec<u8>>,check_model:bool)->Result<Prepared> {
        if changed.values().any(|b| b.len() as u64>MAX_ENTRY) {return Err("entry_too_large");}
        let output = if changed.is_empty() { self.raw.clone() } else {
            let mut old = zip::ZipArchive::new(Cursor::new(&self.raw)).map_err(|_| "invalid_zip")?;
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer.set_raw_comment(old.comment().to_vec().into_boxed_slice());
            for i in 0..old.len() {
                let file = old.by_index(i).map_err(|_| "invalid_zip")?;
                if let Some(data) = changed.get(file.name()) {
                    // Store edited XML so valid repetitive text cannot become a ZIP bomb
                    // according to our own input compression-ratio policy.
                    let mut options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
                    if let Some(time) = file.last_modified() { options = options.last_modified_time(time); }
                    if let Some(mode) = file.unix_mode() { options = options.unix_permissions(mode); }
                    writer.start_file(file.name(), options).map_err(|_| "zip_write_error")?;
                    writer.write_all(data).map_err(|_| "zip_write_error")?;
                } else {
                    writer.raw_copy_file(file).map_err(|_| "zip_write_error")?;
                }
            }
            for (name,data) in &changed {if !self.parts.iter().any(|(n,_)|n==name) {
                if !package_model::safe_part(name) {return Err("unsafe_member_path");}
                writer.start_file(name,zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).map_err(|_|"zip_write_error")?;
                writer.write_all(data).map_err(|_|"zip_write_error")?;
            }}
            let generated = writer.finish().map_err(|_| "zip_write_error")?.into_inner();
            zip_preserve::restore(&self.raw, &generated, &changed)?
        };
        if output.len() as u64 > MAX_INPUT { return Err("output_too_large"); }
        // Reapply every input guard (including expanded total and CRC/XML parsing)
        // before any output is published. A success must always be reopenable.
        if !changed.is_empty() {
            let reopened=Self::from_raw(output.clone())?;
            if check_model && reopened.validate_supported()["valid_supported_structure"]!=true {return Err("invalid_media_output");}
        }
        let receipt = serde_json::json!({"sha256":format!("{:x}", Sha256::digest(&output)),
            "changed_parts":changed.keys().collect::<Vec<_>>(),
            "provenance":{"source_revision":self.inspection.revision,"changes":diagnostics::part_changes(&self.parts,&changed)},
            "native_validated":false,"full_schema_validated":false});
        Ok(Prepared { bytes: output, receipt })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub revision: String,
    pub operations: Vec<Operation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub op: String,
    pub target: String,
    pub expected: String,
    pub text: String,
}
pub(crate) fn valid_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}
pub(crate) fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\r', "&#13;")
}
fn tag_end(raw: &str) -> Option<usize> {
    let mut quote = None;
    for (i,c) in raw.char_indices() {
        if let Some(q) = quote { if c == q { quote = None; } }
        else if c == '\'' || c == '"' { quote = Some(c); }
        else if c == '>' { return Some(i + 1); }
    }
    None
}
