//! Paragraph property clone/reuse. Missing/ambiguous schema variants reject.
use crate::{Package, Result, HP};
use crate::styles::{HH, standalone, signature, attr, set_attr, patch};
use serde::Deserialize;
use std::collections::BTreeMap;
const HC: &str="http://www.hancom.co.kr/hwpml/2011/core";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParagraphSpec { revision:String, target:String, properties:Properties }
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Properties {
    align:Option<String>, indent:Option<i32>, left:Option<u32>, right:Option<u32>,
    space_before:Option<u32>, space_after:Option<u32>,
    keep_with_next:Option<bool>, keep_lines:Option<bool>, line_spacing_percent:Option<u32>,
    line_spacing:Option<crate::line_spacing::LineSpacing>,
}
fn update(xml:&str, ns:&str, name:&str, key:&str, value:&str) -> Result<String> {
    let doc=roxmltree::Document::parse(xml).map_err(|_| "invalid_header_xml")?;
    let root=doc.root_element();
    let branch=|n:roxmltree::Node| n==root || (n.tag_name().namespace()==Some(HP)
        && ["case","default"].contains(&n.tag_name().name())
        && n.parent().is_some_and(|p| p.has_tag_name((HP,"switch")) && p.parent()==Some(root)));
    let nodes:Vec<_>=doc.descendants().filter(|n| n.has_tag_name((ns,name)) && n.parent().is_some_and(|p| {
        if ns==HC {p.has_tag_name((HH,"margin")) && p.parent().is_some_and(branch)}
        else if name=="lineSpacing" {branch(p)} else {p==root}
    })).collect();
    if nodes.is_empty() { return Err("unsupported_paragraph_structure"); }
    if nodes.iter().enumerate().any(|(i,n)| nodes[..i].iter().any(|p| p.parent()==n.parent())) {return Err("ambiguous_paragraph_structure");}
    let edits=nodes.iter().map(|n| Ok((n.range(),set_attr(xml,*n,key,value)?))).collect::<Result<Vec<_>>>()?;
    patch(xml,edits)
}
impl Package {
    pub fn paragraph_bytes(&self,spec:ParagraphSpec)->Result<crate::Prepared> {
        if spec.revision!=self.inspection.revision { return Err("stale_document"); }
        let p=spec.properties;
        if p.align.is_none() && p.indent.is_none() && p.left.is_none() && p.right.is_none()
            && p.space_before.is_none() && p.space_after.is_none() && p.keep_with_next.is_none()
            && p.keep_lines.is_none() && p.line_spacing_percent.is_none() && p.line_spacing.is_none() { return Err("empty_paragraph_properties"); }
        if p.align.as_ref().is_some_and(|s| !["LEFT","RIGHT","CENTER","JUSTIFY","DISTRIBUTE","DISTRIBUTE_SPACE"].contains(&s.as_str())) { return Err("invalid_alignment"); }
        if p.indent.is_some_and(|v| v.unsigned_abs()>1000000) || [p.left,p.right,p.space_before,p.space_after].iter().any(|v| v.is_some_and(|v| v>1000000))
            || p.line_spacing_percent.is_some_and(|v| !(1..=1000).contains(&v)) { return Err("invalid_paragraph_measure"); }
        let line=crate::line_spacing::resolve(p.line_spacing_percent,p.line_spacing.as_ref())?;
        let obj=self.inspection.paragraphs.iter().find(|n| n.id==spec.target).ok_or("unknown_target")?;
        let xml=std::str::from_utf8(&self.parts.iter().find(|(k,_)| k==&obj.part).ok_or("unknown_target")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let doc=roxmltree::Document::parse(xml).map_err(|_| "invalid_section_xml")?;
        let index=spec.target.rsplit_once("#p").ok_or("unknown_target")?.1.parse::<usize>().map_err(|_| "unknown_target")?;
        let node=doc.descendants().filter(|n| n.has_tag_name((HP,"p"))).nth(index).ok_or("unknown_target")?;
        let old=node.attribute("paraPrIDRef").ok_or("unresolved_paragraph_reference")?;
        let header=std::str::from_utf8(&self.parts.iter().find(|(k,_)| k=="Contents/header.xml").ok_or("missing_header")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let hd=roxmltree::Document::parse(header).map_err(|_| "invalid_header_xml")?;
        let sets:Vec<_>=hd.descendants().filter(|n| n.has_tag_name((HH,"paraProperties"))).collect();
        if sets.len()!=1 { return Err("missing_paragraph_definitions"); }
        let set=sets[0];
        let defs:Vec<_>=set.children().filter(|n| n.is_element()).collect();
        if set.attribute("itemCnt")!=Some(defs.len().to_string().as_str()) || defs.iter().enumerate().any(|(i,n)| !n.has_tag_name((HH,"paraPr")) || n.attribute("id")!=Some(i.to_string().as_str())) { return Err("nonordinal_paragraph_definitions"); }
        let original=defs.iter().find(|n| n.attribute("id")==Some(old)).ok_or("unresolved_paragraph_reference")?;
        let mut candidate=standalone(header,*original)?;
        if let Some(a)=p.align { candidate=update(&candidate,HH,"align","horizontal",&a)?; }
        for (name,value) in [("intent",p.indent.map(|v| v.to_string())),("left",p.left.map(|v| v.to_string())),("right",p.right.map(|v| v.to_string())),("prev",p.space_before.map(|v| v.to_string())),("next",p.space_after.map(|v| v.to_string()))] {
            if let Some(v)=value { candidate=update(&candidate,HC,name,"value",&v)?; }
        }
        for (name,value) in [("keepWithNext",p.keep_with_next),("keepLines",p.keep_lines)] {
            if let Some(v)=value { candidate=update(&candidate,HH,"breakSetting",name,if v {"1"}else{"0"})?; }
        }
        if p.line_spacing_percent.is_some() || p.line_spacing.is_some() {
            candidate=update(&candidate,HH,"lineSpacing","type",line.0)?;
            candidate=update(&candidate,HH,"lineSpacing","value",&line.1.to_string())?;
            candidate=update(&candidate,HH,"lineSpacing","unit","HWPUNIT")?;
        }
        let cd=roxmltree::Document::parse(&candidate).map_err(|_| "invalid_header_xml")?;
        let desired=signature(cd.root_element());
        let chosen=defs.iter().find_map(|d| {
            let s=standalone(header,*d).ok()?;
            let d=roxmltree::Document::parse(&s).ok()?;
            (signature(d.root_element())==desired).then(|| d.root_element().attribute("id").unwrap().to_string())
        });
        let mut changed=BTreeMap::new();
        let id=if let Some(id)=chosen {id} else {
            if defs.len()>=10000 { return Err("excess_style_definitions"); }
            let id=defs.len().to_string();
            candidate=attr(&candidate,"id",&id)?;
            let mut new_set=set_attr(header,set,"itemCnt",&(defs.len()+1).to_string())?;
            let at=new_set.rfind("</").ok_or("invalid_header_xml")?;
            new_set.insert_str(at,&candidate);
            changed.insert("Contents/header.xml".to_string(),patch(header,vec![(set.range(),new_set)])?.into_bytes());
            id
        };
        if id!=old { changed.insert(obj.part.clone(),patch(xml,vec![(node.range(),set_attr(xml,node,"paraPrIDRef",&id)?)])?.into_bytes()); }
        let mut result=self.prepare_changed(changed,false)?;
        result.receipt["paragraph_definition"]=serde_json::json!(id);
        Ok(result)
    }
}
