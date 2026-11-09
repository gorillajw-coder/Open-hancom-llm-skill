//! Explicit named paragraph style creation/reuse and application.
use crate::{Package,Result,HP,valid_xml_char};
use crate::styles::{HH,standalone,attr,set_attr,patch};
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleSpec {revision:String,target:String,name:String,paragraph_definition:Option<u32>,character_definition:Option<u32>}
impl Package {
    pub fn style_bytes(&self,spec:StyleSpec)->Result<crate::Prepared> {
        if spec.revision!=self.inspection.revision {return Err("stale_document");}
        if spec.name.trim().is_empty() || spec.name.len()>256 || !spec.name.chars().all(valid_xml_char) {return Err("invalid_style_name");}
        if spec.paragraph_definition.is_some()!=spec.character_definition.is_some() {return Err("incomplete_style_definition");}
        let object=self.inspection.paragraphs.iter().find(|n| n.id==spec.target).ok_or("unknown_target")?;
        let xml=std::str::from_utf8(&self.parts.iter().find(|(n,_)| n==&object.part).ok_or("unknown_target")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let d=roxmltree::Document::parse(xml).map_err(|_| "invalid_section_xml")?;
        let index=spec.target.rsplit_once("#p").ok_or("unknown_target")?.1.parse::<usize>().map_err(|_| "unknown_target")?;
        let p=d.descendants().filter(|n| n.has_tag_name((HP,"p"))).nth(index).ok_or("unknown_target")?;
        let header=std::str::from_utf8(&self.parts.iter().find(|(n,_)| n=="Contents/header.xml").ok_or("missing_header")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let hd=roxmltree::Document::parse(header).map_err(|_| "invalid_header_xml")?;
        let sets:Vec<_>=hd.descendants().filter(|n| n.has_tag_name((HH,"styles"))).collect();
        if sets.len()!=1 {return Err("missing_named_styles");}
        let set=sets[0];let defs:Vec<_>=set.children().filter(|n| n.is_element()).collect();
        if set.attribute("itemCnt")!=Some(defs.len().to_string().as_str()) || defs.iter().enumerate().any(|(i,n)| !n.has_tag_name((HH,"style")) || n.attribute("id")!=Some(i.to_string().as_str())) {return Err("nonordinal_named_styles");}
        let matches:Vec<_>=defs.iter().filter(|n| n.attribute("name")==Some(spec.name.as_str())).collect();
        if matches.len()>1 {return Err("ambiguous_style_name");}
        let mut changed=BTreeMap::new();
        let (id,pid,cid)=if let Some(style)=matches.first() {
            if style.attribute("type")!=Some("PARA") {return Err("unsupported_style_type");}
            let pid=style.attribute("paraPrIDRef").ok_or("unresolved_style_reference")?.to_string();
            let cid=style.attribute("charPrIDRef").ok_or("unresolved_style_reference")?.to_string();
            if spec.paragraph_definition.is_some_and(|n| n.to_string()!=pid) || spec.character_definition.is_some_and(|n| n.to_string()!=cid) {return Err("named_style_conflict");}
            (style.attribute("id").unwrap().to_string(),pid,cid)
        } else {
            let pid=spec.paragraph_definition.ok_or("unknown_named_style")?.to_string();
            let cid=spec.character_definition.ok_or("unknown_named_style")?.to_string();
            if defs.is_empty() || defs.len()>=10000 {return Err("unsupported_named_style_registry");}
            let original=defs[0];
            // Clone only a known paragraph style; preserve unspecified native attributes.
            if original.attribute("type")!=Some("PARA") {return Err("unsupported_style_type");}
            let mut candidate=standalone(header,original)?;
            let id=defs.len().to_string();
            for (key,value) in [("id",id.as_str()),("name",spec.name.as_str()),("engName",spec.name.as_str()),("paraPrIDRef",pid.as_str()),("charPrIDRef",cid.as_str()),("nextStyleIDRef","0")] {candidate=attr(&candidate,key,value)?;}
            let mut replacement=set_attr(header,set,"itemCnt",&(defs.len()+1).to_string())?;
            let at=replacement.rfind("</").ok_or("invalid_header_xml")?;replacement.insert_str(at,&candidate);
            changed.insert("Contents/header.xml".to_string(),patch(header,vec![(set.range(),replacement)])?.into_bytes());
            (id,pid,cid)
        };
        for (tag,reference) in [("paraPr",pid.as_str()),("charPr",cid.as_str())] {
            if !hd.descendants().any(|n| n.has_tag_name((HH,tag)) && n.attribute("id")==Some(reference)) {return Err("unresolved_style_reference");}
        }
        let mut local=standalone(xml,p)?;
        let pd=roxmltree::Document::parse(&local).map_err(|_| "invalid_section_xml")?;
        let edits=pd.root_element().children().filter(|n| n.has_tag_name((HP,"run")))
            .map(|n| Ok((n.range(),set_attr(&local,n,"charPrIDRef",&cid)?))).collect::<Result<Vec<_>>>()?;
        local=patch(&local,edits)?;
        local=attr(&local,"styleIDRef",&id)?;local=attr(&local,"paraPrIDRef",&pid)?;
        // Namespace injection is only needed for parsing; when semantics already match,
        // retain exact bytes rather than rewrite for lexical namespace differences.
        let already=p.attribute("styleIDRef")==Some(id.as_str()) && p.attribute("paraPrIDRef")==Some(pid.as_str())
            && p.children().filter(|n| n.has_tag_name((HP,"run"))).all(|n| n.attribute("charPrIDRef")==Some(cid.as_str()));
        if !already {changed.insert(object.part.clone(),patch(xml,vec![(p.range(),local)])?.into_bytes());}
        let mut result=self.prepare_changed(changed,false)?;result.receipt["named_style"]=serde_json::json!(id);Ok(result)
    }
}
