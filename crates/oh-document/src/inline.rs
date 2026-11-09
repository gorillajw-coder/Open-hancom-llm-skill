//! Editable inline PNG and hyperlink controls, a bounded public-model subset.
//! Model provenance: validation/offline-continuation-format/sources.json.
use crate::{Result,escape_xml,valid_xml_char};
use serde::Deserialize;
use std::io::Cursor;
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct PngImageSpec {pub png_hex:String,pub width:u32,pub height:u32}
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct JpegImageSpec {pub format:JpegFormat,pub jpeg_hex:String,pub width:u32,pub height:u32}
#[derive(Deserialize,Clone,PartialEq,Eq)]
pub enum JpegFormat {#[serde(rename="jpeg")] Jpeg}
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(untagged)]
pub enum ImageSpec {Png(PngImageSpec),Jpeg(JpegImageSpec)}
impl ImageSpec {
    pub(crate) fn width(&self)->u32 {match self {Self::Png(s)=>s.width,Self::Jpeg(s)=>s.width}}
    pub(crate) fn height(&self)->u32 {match self {Self::Png(s)=>s.height,Self::Jpeg(s)=>s.height}}
}
pub(crate) fn image_kind(raw:&[u8])->(&'static str,&'static str) {
    if raw.starts_with(b"\xff\xd8") {("jpg","image/jpeg")}else{("png","image/png")}
}
pub(crate) fn url_ok(s:&str)->bool {
    if s.len()>4096 || !s.chars().all(valid_xml_char) || s.chars().any(|c|c.is_control() || c.is_whitespace() || c=='\\') {return false;}
    url::Url::parse(s).is_ok_and(|u|matches!(u.scheme(),"http"|"https") && u.host_str().is_some() && u.username().is_empty() && u.password().is_none())
}
pub(crate) fn png_info(raw:&[u8])->Result<(u32,u32)> {
    if raw.len()>1024*1024 || raw.len()<33 {return Err("invalid_or_unsupported_png");}
    let mut decoder=png::Decoder::new(Cursor::new(raw));
    decoder.set_limits(png::Limits{bytes:16*1024*1024});
    let mut reader=decoder.read_info().map_err(|_|"invalid_or_unsupported_png")?;
    let info=reader.info();let (w,h)=(info.width,info.height);
    if w==0 || h==0 || w>4096 || h>4096 || w as u64*h as u64>4000000 || info.animation_control.is_some() || info.bit_depth!=png::BitDepth::Eight || !matches!(info.color_type,png::ColorType::Rgb|png::ColorType::Rgba) {return Err("invalid_or_unsupported_png");}
    let mut buf=vec![0;reader.output_buffer_size()];
    reader.next_frame(&mut buf).map_err(|_|"invalid_or_unsupported_png")?;
    reader.finish().map_err(|_|"invalid_or_unsupported_png")?;
    Ok((w,h))
}
pub(crate) fn image_data(spec:&ImageSpec)->Result<Vec<u8>> {
    if spec.width()==0 || spec.height()==0 || spec.width()>1000000 || spec.height()>1000000 {return Err("invalid_image_geometry");}
    let (s,error)=match spec {ImageSpec::Png(s)=>(s.png_hex.as_bytes(),"invalid_or_unsupported_png"),ImageSpec::Jpeg(s)=>(s.jpeg_hex.as_bytes(),"invalid_or_unsupported_jpeg")};
    if s.len()>2*1024*1024 || s.len()%2!=0 {return Err(error);}
    let digit=|b:u8|->Result<u8>{match b {b'0'..=b'9'=>Ok(b-b'0'),b'a'..=b'f'=>Ok(b-b'a'+10),b'A'..=b'F'=>Ok(b-b'A'+10),_=>Err(error)}};
    let raw=s.chunks_exact(2).map(|p|Ok(digit(p[0])?*16+digit(p[1])?)).collect::<Result<Vec<_>>>()?;
    match spec {ImageSpec::Png(_)=>{png_info(&raw)?;},ImageSpec::Jpeg(_)=>{crate::jpeg::info(&raw)?;}}
    Ok(raw)
}
pub(crate) fn hyperlink(id:u32,cid:usize,text:&str,url:&str)->Result<String> {
    if !url_ok(url) {return Err("unsafe_hyperlink");}
    let mut command=String::new();
    for c in url.chars() {if [':','?',';','#'].contains(&c) {command.push('\\');}command.push(c);}
    command.push_str(";1;0;0;");
    let url=escape_xml(url);let command=escape_xml(&command);let text=escape_xml(text);
    Ok(format!("<hp:run charPrIDRef=\"{cid}\"><hp:ctrl><hp:fieldBegin id=\"{id}\" type=\"HYPERLINK\" name=\"\" editable=\"0\" dirty=\"1\" zorder=\"-1\" fieldid=\"{id}\"><hp:parameters cnt=\"6\" name=\"\"><hp:integerParam name=\"Prop\">0</hp:integerParam><hp:stringParam name=\"Command\">{command}</hp:stringParam><hp:stringParam name=\"Path\">{url}</hp:stringParam><hp:stringParam name=\"Category\">HWPHYPERLINK_TYPE_URL</hp:stringParam><hp:stringParam name=\"TargetType\">HWPHYPERLINK_TARGET_BOOKMARK</hp:stringParam><hp:stringParam name=\"DocOpenType\">HWPHYPERLINK_JUMP_CURRENTTAB</hp:stringParam></hp:parameters></hp:fieldBegin></hp:ctrl></hp:run><hp:run charPrIDRef=\"{cid}\"><hp:t>{text}</hp:t></hp:run><hp:run charPrIDRef=\"{cid}\"><hp:ctrl><hp:fieldEnd beginIDRef=\"{id}\" fieldid=\"{id}\"/></hp:ctrl></hp:run>"))
}
pub(crate) fn picture(id:u32,item:&str,w:u32,h:u32)->String {
    format!("<hp:pic id=\"{id}\" zOrder=\"0\" numberingType=\"PICTURE\" textWrap=\"TOP_AND_BOTTOM\" textFlow=\"BOTH_SIDES\" lock=\"0\" dropcapstyle=\"None\" href=\"\" groupLevel=\"0\" instid=\"{id}\" reverse=\"0\"><hp:offset x=\"0\" y=\"0\"/><hp:orgSz width=\"{w}\" height=\"{h}\"/><hp:curSz width=\"{w}\" height=\"{h}\"/><hp:flip horizontal=\"0\" vertical=\"0\"/><hp:rotationInfo angle=\"0\" centerX=\"{}\" centerY=\"{}\" rotateimage=\"1\"/><hp:renderingInfo><hc:transMatrix e1=\"1\" e2=\"0\" e3=\"0\" e4=\"0\" e5=\"1\" e6=\"0\"/><hc:scaMatrix e1=\"1\" e2=\"0\" e3=\"0\" e4=\"0\" e5=\"1\" e6=\"0\"/><hc:rotMatrix e1=\"1\" e2=\"0\" e3=\"0\" e4=\"0\" e5=\"1\" e6=\"0\"/></hp:renderingInfo><hp:imgRect><hc:pt0 x=\"0\" y=\"0\"/><hc:pt1 x=\"{w}\" y=\"0\"/><hc:pt2 x=\"{w}\" y=\"{h}\"/><hc:pt3 x=\"0\" y=\"{h}\"/></hp:imgRect><hp:imgClip left=\"0\" right=\"{w}\" top=\"0\" bottom=\"{h}\"/><hp:inMargin left=\"0\" right=\"0\" top=\"0\" bottom=\"0\"/><hc:img binaryItemIDRef=\"{item}\" bright=\"0\" contrast=\"0\" effect=\"REAL_PIC\" alpha=\"0\"/><hp:imgDim dimwidth=\"{w}\" dimheight=\"{h}\"/><hp:sz width=\"{w}\" widthRelTo=\"ABSOLUTE\" height=\"{h}\" heightRelTo=\"ABSOLUTE\" protect=\"0\"/><hp:pos treatAsChar=\"1\" affectLSpacing=\"0\" flowWithText=\"1\" allowOverlap=\"0\" holdAnchorAndSO=\"0\" vertRelTo=\"PARA\" horzRelTo=\"COLUMN\" vertAlign=\"TOP\" horzAlign=\"LEFT\" vertOffset=\"0\" horzOffset=\"0\"/><hp:outMargin left=\"0\" right=\"0\" top=\"0\" bottom=\"0\"/></hp:pic>",w/2,h/2)
}
