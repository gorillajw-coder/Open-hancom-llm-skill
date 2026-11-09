//! Supported-structure diagnostics and hash-only package diff, not full schema validation.
use crate::{Package,HP,is_section};
use crate::styles::{HH,standalone};
use sha2::{Digest,Sha256};
use serde_json::{Value,json};
use std::collections::{BTreeMap,BTreeSet};
fn digest(bytes:&[u8])->String {format!("{:x}",Sha256::digest(bytes))}
pub(crate) fn part_changes(before:&[(String,Vec<u8>)],after:&BTreeMap<String,Vec<u8>>)->Vec<Value> {
    after.iter().map(|(part,bytes)| json!({"part":part,"before_sha256":before.iter().find(|(n,_)| n==part).map(|(_,b)|digest(b)),"after_sha256":digest(bytes)})).collect()
}
impl Package {
    pub fn diff(&self,other:&Self)->Value {
        let a:BTreeMap<_,_>=self.parts.iter().map(|(n,b)|(n,b)).collect();
        let b:BTreeMap<_,_>=other.parts.iter().map(|(n,b)|(n,b)).collect();
        let names:BTreeSet<_>=a.keys().chain(b.keys()).copied().collect();
        let changes:Vec<_>=names.into_iter().filter(|n| a.get(n)!=b.get(n)).map(|n| json!({
            "part":n,"before_sha256":a.get(n).map(|v|digest(v)),"after_sha256":b.get(n).map(|v|digest(v)),
            "before_bytes":a.get(n).map(|v|v.len()),"after_bytes":b.get(n).map(|v|v.len())})).collect();
        json!({"before_revision":self.inspection.revision,"after_revision":other.inspection.revision,
            "archive_bytes_changed":self.raw!=other.raw,"content_parts_changed":changes.len(),"changes":changes,
            "scope":"hash-only part payload diff; metadata/layout/native appearance not compared"})
    }
    pub fn validate_supported(&self)->Value {
        let mut diagnostics=self.model_diagnostics();
        diagnostics.extend(crate::native_lists::diagnostics(&self.parts));
        let mut error=|code:&str,part:&str| diagnostics.push(json!({"severity":"error","code":code,"part":part}));
        for name in ["version.xml","settings.xml","META-INF/container.xml","META-INF/manifest.xml","Contents/content.hpf","Contents/header.xml"] {
            if !self.parts.iter().any(|(n,_)|n==name) {error("missing_package_part",name);}
        }
        for (name,bytes) in &self.parts {if name.ends_with(".xml") || name.ends_with(".hpf") {
            if std::str::from_utf8(bytes).ok().and_then(|s| roxmltree::Document::parse(s).ok()).is_none() {error("invalid_package_xml",name);}
        }}
        let mut registry:BTreeMap<&str,BTreeSet<String>>=BTreeMap::new();
        if let Some((_,bytes))=self.parts.iter().find(|(n,_)| n=="Contents/header.xml") {
            if let Ok(xml)=std::str::from_utf8(bytes) {if let Ok(doc)=roxmltree::Document::parse(xml) {
                for (group,tag,offset) in [("charProperties","charPr",0),("paraProperties","paraPr",0),("styles","style",0),("borderFills","borderFill",1),("tabProperties","tabPr",0)] {
                    let groups:Vec<_>=doc.descendants().filter(|n|n.has_tag_name((HH,group))).collect();
                    if groups.len()!=1 {error("missing_or_duplicate_definition_registry","Contents/header.xml");continue;}
                    let g=groups[0];let defs:Vec<_>=g.children().filter(|n|n.is_element()).collect();
                    if g.attribute("itemCnt")!=Some(defs.len().to_string().as_str()) || defs.iter().enumerate().any(|(i,n)| !n.has_tag_name((HH,tag)) || n.attribute("id")!=Some((i+offset).to_string().as_str())) {error("invalid_definition_ids_or_count","Contents/header.xml");}
                    registry.insert(tag,defs.iter().filter_map(|n|n.attribute("id").map(str::to_string)).collect());
                }
                for n in doc.descendants().filter(|n|n.is_element() && n.tag_name().namespace()==Some(HH)) {
                    for (reference,kind) in [("charPrIDRef","charPr"),("paraPrIDRef","paraPr"),("nextStyleIDRef","style"),("borderFillIDRef","borderFill"),("tabPrIDRef","tabPr")] {
                        if let Some(id)=n.attribute(reference) {if !registry.get(kind).is_some_and(|ids| ids.contains(id)) {error("unresolved_header_reference","Contents/header.xml");}}
                    }
                }
            }}
        }
        let mut object_ids=BTreeSet::new();
        for (name,bytes) in &self.parts {if is_section(name) {
            if let Ok(xml)=std::str::from_utf8(bytes) {if let Ok(doc)=roxmltree::Document::parse(xml) {
                for n in doc.descendants().filter(|n|n.is_element() && n.tag_name().namespace()==Some(HP)) {
                    for (reference,kind) in [("charPrIDRef","charPr"),("paraPrIDRef","paraPr"),("styleIDRef","style"),("borderFillIDRef","borderFill")] {
                        if let Some(id)=n.attribute(reference) {if !registry.get(kind).is_some_and(|ids| ids.contains(id)) {error("unresolved_section_reference",name);}}
                    }
                    if ["p","tbl"].contains(&n.tag_name().name()) {if let Some(id)=n.attribute("id") {if !object_ids.insert(id.to_string()) {error("duplicate_paragraph_or_table_id",name);}}}
                    if n.has_tag_name((HP,"tbl")) {if standalone(xml,n).and_then(crate::table::validate_grid).is_err() {error("unsupported_or_invalid_table_structure",name);}}
                }
            }}
        }}
        json!({"revision":self.inspection.revision,"valid_supported_structure":diagnostics.is_empty(),"diagnostics":diagnostics,
            "native_validated":false,"full_schema_validated":false,"scope":"public-model subset: package XML/manifest/spine/container, font/definition references, PNG decode/bounded strict full JPEG entropy decode, hyperlink pairing/URL/Command, conservative table geometry; not full XSD",
            "unsupported_checks":["native pagination and rendering","full XSD required attributes/child order","images outside PNG and jfif-baseline-8bit-ycbcr-444-v1","arbitrary field/control semantics","cryptographic signatures"]})
    }
}
