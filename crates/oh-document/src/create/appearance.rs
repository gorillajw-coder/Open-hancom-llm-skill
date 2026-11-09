//! New-document cell appearance from pinned Hancom border/fill models.
use crate::Result;
use serde::Deserialize;
#[derive(Deserialize,Default,Clone,PartialEq,Eq)]
#[serde(default,deny_unknown_fields)]
pub(super) struct Appearance {pub fill_color:Option<String>,borders:Borders}
#[derive(Deserialize,Default,Clone,PartialEq,Eq)]
#[serde(default,deny_unknown_fields)]
struct Borders {left:Side,right:Side,top:Side,bottom:Side}
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct Side {style:String,width:String,color:String}
impl Default for Side {fn default()->Self {Self{style:"NONE".into(),width:"0.12 mm".into(),color:"#000000".into()}}}
impl Appearance {
    pub fn normalize(&mut self)->Result<()> {
        if let Some(c)=&mut self.fill_color {
            if c.len()!=7 || !c.starts_with('#') || !c[1..].bytes().all(|b|b.is_ascii_hexdigit()) {return Err("invalid_cell_fill_color");}
            *c=c.to_ascii_uppercase();
        }
        for s in [&mut self.borders.left,&mut self.borders.right,&mut self.borders.top,&mut self.borders.bottom] {
            if !["NONE","SOLID"].contains(&s.style.as_str()) || !crate::border::WIDTHS.contains(&s.width.as_str()) {return Err("invalid_cell_border_style_or_width");}
            if s.color.len()!=7 || !s.color.starts_with('#') || !s.color[1..].bytes().all(|b|b.is_ascii_hexdigit()) {return Err("invalid_cell_border_color");}
            s.color=s.color.to_ascii_uppercase();
        }
        Ok(())
    }
    pub fn xml(&self,id:usize)->String {
        let mut sides=String::new();
        for (name,s) in [("left",&self.borders.left),("right",&self.borders.right),("top",&self.borders.top),("bottom",&self.borders.bottom)] {sides.push_str(&format!("<hh:{name}Border type=\"{}\" width=\"{}\" color=\"{}\"/>",s.style,s.width,s.color));}
        let fill=self.fill_color.as_ref().map(|c|format!("<hc:fillBrush><hc:winBrush faceColor=\"{c}\" hatchColor=\"#999999\" alpha=\"0\"/></hc:fillBrush>")).unwrap_or_default();
        format!("<hh:borderFill id=\"{id}\" threeD=\"0\" shadow=\"0\" centerLine=\"NONE\" breakCellSeparateLine=\"0\"><hh:slash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/><hh:backSlash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/>{sides}<hh:diagonal type=\"NONE\" width=\"0.1 mm\" color=\"#000000\"/>{fill}</hh:borderFill>")
    }
}
