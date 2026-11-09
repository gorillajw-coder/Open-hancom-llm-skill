//! Clone/reuse native borders and solid cell fills; never rewrite shared definitions.
use crate::{Package,Result,HP};
use crate::styles::{HH,standalone,signature,attr,set_attr,patch};
use serde::Deserialize;
use std::collections::BTreeMap;
const HC:&str="http://www.hancom.co.kr/hwpml/2011/core";
pub(crate) const WIDTHS:&[&str]=&["0.1 mm","0.12 mm","0.15 mm","0.2 mm","0.25 mm","0.3 mm","0.4 mm","0.5 mm","0.6 mm","0.7 mm","1.0 mm","1.5 mm","2.0 mm","3.0 mm","4.0 mm","5.0 mm"];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BorderSpec {revision:String,target:String,properties:Properties}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Properties {border_color:Option<String>,border_width:Option<String>,fill_color:Option<String>}
fn color(s:&str)->bool {s.len()==7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())}
fn update(xml:&str,ns:&str,tag:&str,key:&str,value:&str)->Result<String> {
    let d=roxmltree::Document::parse(xml).map_err(|_| "invalid_header_xml")?;
    let nodes:Vec<_>=d.root_element().children().filter(|n| n.has_tag_name((ns,tag))).collect();
    if nodes.len()!=1 {return Err("unsupported_border_structure");}
    patch(xml,vec![(nodes[0].range(),set_attr(xml,nodes[0],key,value)?)])
}
impl Package {
    pub fn border_bytes(&self,spec:BorderSpec)->Result<crate::Prepared> {
        if spec.revision!=self.inspection.revision {return Err("stale_document");}
        let p=spec.properties;
        if p.border_color.is_none() && p.border_width.is_none() && p.fill_color.is_none() {return Err("empty_border_properties");}
        if [p.border_color.as_ref(),p.fill_color.as_ref()].iter().flatten().any(|s| !color(s)) {return Err("invalid_border_color");}
        if p.border_width.as_ref().is_some_and(|s| !["0.1 mm","0.12 mm","0.15 mm","0.2 mm","0.25 mm","0.3 mm","0.4 mm","0.5 mm","0.6 mm","0.7 mm","1.0 mm","1.5 mm","2.0 mm","3.0 mm","4.0 mm","5.0 mm"].contains(&s.as_str())) {return Err("invalid_border_width");}
        let object=self.inspection.cells.iter().find(|n| n.id==spec.target).ok_or("unknown_target")?;
        let index=spec.target.rsplit_once("#tc").ok_or("unknown_target")?.1.parse::<usize>().map_err(|_| "unknown_target")?;
        let xml=std::str::from_utf8(&self.parts.iter().find(|(n,_)| n==&object.part).ok_or("unknown_target")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let d=roxmltree::Document::parse(xml).map_err(|_| "invalid_section_xml")?;
        let cell=d.descendants().filter(|n| n.has_tag_name((HP,"tc"))).nth(index).ok_or("unknown_target")?;
        let old=cell.attribute("borderFillIDRef").ok_or("unresolved_border_reference")?;
        let header=std::str::from_utf8(&self.parts.iter().find(|(n,_)| n=="Contents/header.xml").ok_or("missing_header")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let hd=roxmltree::Document::parse(header).map_err(|_| "invalid_header_xml")?;
        let sets:Vec<_>=hd.descendants().filter(|n| n.has_tag_name((HH,"borderFills"))).collect();
        if sets.len()!=1 {return Err("missing_border_definitions");}
        let set=sets[0];let defs:Vec<_>=set.children().filter(|n| n.is_element()).collect();
        if set.attribute("itemCnt")!=Some(defs.len().to_string().as_str()) || defs.iter().enumerate().any(|(i,n)| !n.has_tag_name((HH,"borderFill")) || n.attribute("id")!=Some((i+1).to_string().as_str())) {return Err("nonordinal_border_definitions");}
        let original=defs.iter().find(|n| n.attribute("id")==Some(old)).ok_or("unresolved_border_reference")?;
        let mut candidate=standalone(header,*original)?;
        for side in ["leftBorder","rightBorder","topBorder","bottomBorder"] {
            if let Some(color)=&p.border_color {candidate=update(&candidate,HH,side,"color",&color.to_ascii_uppercase())?;candidate=update(&candidate,HH,side,"type","SOLID")?;}
            if let Some(width)=&p.border_width {candidate=update(&candidate,HH,side,"width",width)?;}
        }
        if let Some(color)=p.fill_color {
            let cd=roxmltree::Document::parse(&candidate).map_err(|_| "invalid_header_xml")?;
            let fills:Vec<_>=cd.root_element().children().filter(|n| n.has_tag_name((HC,"fillBrush"))).collect();
            if fills.len()>1 {return Err("unsupported_existing_fill");}
            if let Some(fill)=fills.first() {
                let children:Vec<_>=fill.children().filter(|n| n.is_element()).collect();
                if children.len()!=1 || !children[0].has_tag_name((HC,"winBrush")) {return Err("unsupported_existing_fill");}
                let changed=set_attr(&candidate,children[0],"faceColor",&color.to_ascii_uppercase())?;
                candidate=patch(&candidate,vec![(children[0].range(),changed)])?;
            } else {
                let fill=format!("<hc:fillBrush xmlns:hc=\"{HC}\"><hc:winBrush faceColor=\"{}\" hatchColor=\"#999999\" alpha=\"0\"/></hc:fillBrush>",color.to_ascii_uppercase());
                let at=candidate.rfind("</").ok_or("unsupported_empty_border_definition")?;
                candidate.insert_str(at,&fill);
            }
        }
        let cd=roxmltree::Document::parse(&candidate).map_err(|_| "invalid_header_xml")?;
        let desired=signature(cd.root_element());
        let chosen=defs.iter().find_map(|d| {let s=standalone(header,*d).ok()?;let n=roxmltree::Document::parse(&s).ok()?;
            (signature(n.root_element())==desired).then(|| d.attribute("id").unwrap().to_string())});
        let mut changed=BTreeMap::new();
        let id=if let Some(id)=chosen {id} else {
            if defs.len()>=10000 {return Err("excess_style_definitions");}
            let id=(defs.len()+1).to_string();candidate=attr(&candidate,"id",&id)?;
            let mut replacement=set_attr(header,set,"itemCnt",&(defs.len()+1).to_string())?;
            let at=replacement.rfind("</").ok_or("invalid_header_xml")?;replacement.insert_str(at,&candidate);
            changed.insert("Contents/header.xml".to_string(),patch(header,vec![(set.range(),replacement)])?.into_bytes());id
        };
        if id!=old {changed.insert(object.part.clone(),patch(xml,vec![(cell.range(),set_attr(xml,cell,"borderFillIDRef",&id)?)])?.into_bytes());}
        let mut result=self.prepare_changed(changed,false)?;result.receipt["border_definition"]=serde_json::json!(id);Ok(result)
    }
}
