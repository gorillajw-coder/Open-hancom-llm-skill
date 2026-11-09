//! Admission policy for a deliberately small JFIF subset, followed by a real
//! strict full entropy decode. This marker walk is NOT the JPEG decoder.
//! Public JFIF1.02 and zune APIs/licenses: validation/jpeg-media-format/RESEARCH.md.
use crate::Result;
use zune_core::{bytestream::ZCursor,colorspace::ColorSpace,options::DecoderOptions};
use zune_jpeg::JpegDecoder;
use sha2::{Digest,Sha256};
use serde_json::{json,Value};

pub(crate) const SUBSET:&str="jfif-baseline-8bit-ycbcr-444-422-420-v2";
pub(crate) const DECODER:&str="zune-jpeg-0.5.15+open-hancom-completeness-v1";

// Matches the baseline MCU-row allocations, not a process RSS/allocator quota.
// Trace: vendor src/{misc.rs,components.rs,decoder.rs,mcu.rs,worker.rs}.
fn buffer_plan(w:usize,h:usize,hs:usize,vs:usize)->Result<Value> {
    let mul=|a:usize,b:usize|a.checked_mul(b).ok_or("jpeg_allocation_limit");
    let add=|a:usize,b:usize|a.checked_add(b).ok_or("jpeg_allocation_limit");
    let mx=add(w,8*hs-1)?/(8*hs);let my=add(h,8*vs-1)?/(8*vs);
    let pw=mul(mx,8*hs)?;let ph=mul(my,8*vs)?;
    let output=mul(mul(w,h)?,3)?;
    if pw>4096 || ph>4096 || output>12000000 {return Err("jpeg_allocation_limit");}
    let mut working=0;let mut components=Vec::new();
    for (id,ch,cv) in [(1,hs,vs),(2,1,1),(3,1,1)] {
        let stride=mul(mul(mx,ch)?,8)?;let ratio=(hs/ch)*(vs/cv);
        let raw=mul(mul(mul(stride,cv)?,8)?,2)?;
        let up=if hs>1 {mul(add(add(mul(mul(stride,cv)?,2)?,mul(mul(cv,stride)?,ratio)?)?,mul(mul(stride,ratio)?,8)?)?,2)?}else{0};
        working=add(working,add(raw,up)?)?;
        components.push(json!({"id":id,"horizontal_factor":ch,"vertical_factor":cv,
            "pixel_width":add(mul(w,ch)?,hs-1)?/hs,"pixel_height":add(mul(h,cv)?,vs-1)?/vs,
            "width_stride":stride,"raw_coeff_bytes":raw,"upsample_buffer_bytes":up}));
    }
    let scratch=if hs>1 {mul(mul(pw,8)?,2)?}else{0};working=add(working,scratch)?;
    if working>1048576 {return Err("jpeg_allocation_limit");}
    // Each baseline block requires at least one DC and one AC Huffman bit.
    let minimum=mul(mul(mul(mx,my)?,hs*vs+2)?,2)?;
    Ok(json!({"mcu_columns":mx,"mcu_rows":my,"padded_width":pw,"padded_height":ph,
        "components":components,"upsampler_scratch_bytes":scratch,"working_buffer_bytes":working,
        "output_rgb_bytes":output,"minimum_entropy_bits":minimum}))
}

pub(crate) fn info(raw:&[u8])->Result<(u32,u32)> {
    let details=details(raw)?;
    Ok((details["pixel_width"].as_u64().unwrap() as u32,details["pixel_height"].as_u64().unwrap() as u32))
}

pub(crate) fn details(raw:&[u8])->Result<Value> {
    if raw.len()>1024*1024 {return Err("jpeg_input_limit");}
    if raw.len()<24 || !raw.starts_with(b"\xff\xd8\xff\xe0") {return Err("jpeg_requires_jfif");}
    let mut pos=2usize;let mut jfif=false;let mut dimensions=None;let mut scan=false;let mut factors=(1usize,1usize);let mut entropy_bytes=0usize;
    while pos<raw.len() {
        if raw.get(pos)!=Some(&255) {return Err("invalid_jpeg_marker");}
        let marker=*raw.get(pos+1).ok_or("truncated_jpeg")?;pos+=2;
        let lenbytes=raw.get(pos..pos+2).ok_or("truncated_jpeg")?;
        let len=u16::from_be_bytes([lenbytes[0],lenbytes[1]]) as usize;
        if len<2 {return Err("invalid_jpeg_segment");}
        let end=pos.checked_add(len).ok_or("jpeg_allocation_limit")?;
        let data=raw.get(pos+2..end).ok_or("truncated_jpeg")?;
        match marker {
            0xe0=>{
                if jfif || pos!=4 || data.len()!=14 || &data[..5]!=b"JFIF\0" || data[5]!=1 || data[6]>2 || data[7]>2 || data[12..]!=[0,0] {return Err("unsupported_jpeg_jfif_metadata");}
                let x=u16::from_be_bytes([data[8],data[9]]);let y=u16::from_be_bytes([data[10],data[11]]);
                if x==0 || x!=y {return Err("unsupported_jpeg_pixel_aspect");}jfif=true;
            },
            0xe1=>return Err("unsupported_jpeg_exif_or_xmp"),
            0xe2=>return Err("unsupported_jpeg_icc_or_mpo"),
            0xee=>return Err("unsupported_jpeg_adobe_color"),
            0xe3..=0xef|0xfe=>return Err("unsupported_jpeg_metadata"),
            0xc0=>{
                if dimensions.is_some() || data.len()!=15 || data[0]!=8 || data[5]!=3 {return Err("unsupported_jpeg_frame");}
                let h=u16::from_be_bytes([data[1],data[2]]) as u32;let w=u16::from_be_bytes([data[3],data[4]]) as u32;
                if w==0 || h==0 || w>4096 || h>4096 || u64::from(w)*u64::from(h)>4000000 {return Err("jpeg_pixel_limit");}
                // Conventional JFIF YCbCr; 4:4:4, horizontal 4:2:2 or 4:2:0.
                for i in 0..3 {let sampling=data[7+3*i];if data[6+3*i]!=(i+1) as u8 || (if i==0 {sampling!=0x11 && sampling!=0x21 && sampling!=0x22} else {sampling!=0x11}) || data[8+3*i]>3 {return Err("unsupported_jpeg_components_or_sampling");}}
                factors=((data[7]>>4) as usize,(data[7]&15) as usize);
                dimensions=Some((w,h));
            },
            0xc2=>return Err("unsupported_jpeg_progressive"),
            0xdb|0xc4=>{}, // quantization/Huffman tables validated by decoder
            0xda=>{
                if dimensions.is_none() || data.len()!=10 || data[0]!=3 || data[1]!=1 || data[3]!=2 || data[5]!=3 || data[7..]!=[0,63,0] {return Err("unsupported_jpeg_scan");}
                // Exactly one interleaved baseline scan; EOI must terminate file.
                pos=end;
                while pos<raw.len() {
                    if raw[pos]!=255 {entropy_bytes+=1;pos+=1;continue;}
                    match raw.get(pos+1) {
                        Some(0)=>{entropy_bytes+=1;pos+=2;},
                        Some(0xd9) if pos+2==raw.len()=>{scan=true;break;},
                        Some(0xd9)=>return Err("unsupported_jpeg_trailing_data"),
                        Some(_)=>return Err("unsupported_jpeg_entropy_marker"),
                        None=>return Err("truncated_jpeg"),
                    }
                }
                break;
            },
            _=>return Err("unsupported_jpeg_marker_or_process"),
        }
        pos=end;
    }
    if !jfif || !scan {return Err("truncated_jpeg");}
    let (w,h)=dimensions.ok_or("unsupported_jpeg_frame")?;
    // No full-frame coefficient allocation: exactly one three-component scan.
    // Preflight every rounded component/upsampler buffer before decoder allocation.
    let (hs,vs)=factors;let plan=buffer_plan(w as usize,h as usize,hs,vs)?;
    if entropy_bytes*8<plan["minimum_entropy_bits"].as_u64().unwrap() as usize {return Err("invalid_jpeg_entropy");}
    let expected=plan["output_rgb_bytes"].as_u64().unwrap() as usize;
    let options=DecoderOptions::new_safe().set_strict_mode(true).set_max_width(4096).set_max_height(4096).jpeg_set_max_scans(1).jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder=JpegDecoder::new_with_options(ZCursor::new(raw),options);
    decoder.decode_headers().map_err(|_|"invalid_jpeg_headers")?;
    if decoder.dimensions()!=Some((w as usize,h as usize)) || decoder.input_colorspace()!=Some(ColorSpace::YCbCr) || decoder.output_buffer_size()!=Some(expected) {return Err("unsupported_jpeg_decoder_geometry_or_color");}
    let mut pixels=Vec::new();pixels.try_reserve_exact(expected).map_err(|_|"jpeg_allocation_limit")?;pixels.resize(expected,0);
    decoder.decode_into(&mut pixels).map_err(|_|"invalid_jpeg_entropy")?;
    Ok(json!({"pixel_width":w,"pixel_height":h,"sampling":match factors {(1,1)=>"4:4:4",(2,1)=>"4:2:2",_=>"4:2:0"},
        "validation":"bounded_full_decode","subset":SUBSET,"decoder":DECODER,"buffer_plan":plan,
        "decoded_rgb_bytes":pixels.len(),"decoded_rgb_sha256":format!("{:x}",Sha256::digest(&pixels)),"native_render_verified":false}))
}
