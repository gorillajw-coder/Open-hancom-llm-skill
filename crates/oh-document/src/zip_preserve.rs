//! Preserve raw local records and central metadata that zip::raw_copy_file drops.
//! Only ordinary single-disk ZIP32 is rewritten; ZIP64 edits fail closed.
use crate::Result;
use std::{collections::BTreeMap, io::Cursor};

fn u16_at(b: &[u8], i: usize) -> Result<u16> {
    let s = b.get(i..i + 2).ok_or("invalid_zip_metadata")?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], i: usize) -> Result<u32> {
    let s = b.get(i..i + 4).ok_or("invalid_zip_metadata")?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn check_extra(extra: &[u8]) -> Result<()> {
    let mut i = 0;
    while i < extra.len() {
        let id = u16_at(extra, i)?;
        let size = u16_at(extra, i + 2)? as usize;
        if id == 1 { return Err("zip64_edit_unsupported"); }
        i = i.checked_add(4 + size).ok_or("invalid_zip_metadata")?;
        if i > extra.len() { return Err("invalid_zip_metadata"); }
    }
    Ok(())
}

/// New compression/CRC/sizes come from the ZIP library; opaque original records
/// are copied verbatim and central offsets are the only change to their headers.
pub fn restore(original: &[u8], generated: &[u8], changed: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut old = zip::ZipArchive::new(Cursor::new(original)).map_err(|_| "invalid_zip")?;
    let mut new = zip::ZipArchive::new(Cursor::new(generated)).map_err(|_| "invalid_zip")?;
    let central_start = usize::try_from(old.central_directory_start()).map_err(|_| "invalid_zip_metadata")?;
    let mut position = central_start;
    let mut headers = Vec::new();
    let mut offsets = Vec::new();
    for i in 0..old.len() {
        let f = old.by_index(i).map_err(|_| "invalid_zip")?;
        let offset = usize::try_from(f.header_start()).map_err(|_| "invalid_zip_metadata")?;
        let local = original.get(offset..offset + 30).ok_or("invalid_zip_metadata")?;
        if &local[..4] != b"PK\x03\x04" { return Err("invalid_zip_metadata"); }
        if [u32_at(local, 18)?, u32_at(local, 22)?].contains(&u32::MAX) {
            return Err("zip64_edit_unsupported");
        }
        let name_len = u16_at(local, 26)? as usize;
        let extra_len = u16_at(local, 28)? as usize;
        let extra_start = offset.checked_add(30 + name_len).ok_or("invalid_zip_metadata")?;
        let extra_end = extra_start.checked_add(extra_len).ok_or("invalid_zip_metadata")?;
        check_extra(original.get(extra_start..extra_end).ok_or("invalid_zip_metadata")?)?;
        offsets.push(offset);
    }
    let first = offsets.iter().copied().min().ok_or("invalid_zip_metadata")?;
    let mut out = original.get(..first).ok_or("invalid_zip_metadata")?.to_vec();
    for (i, offset) in offsets.iter().copied().enumerate() {
        let fixed = original.get(position..position + 46).ok_or("invalid_zip_metadata")?;
        if &fixed[..4] != b"PK\x01\x02" { return Err("invalid_zip_metadata"); }
        let name_len = u16_at(fixed, 28)? as usize;
        let extra_len = u16_at(fixed, 30)? as usize;
        let comment_len = u16_at(fixed, 32)? as usize;
        if u16_at(fixed, 34)? != 0 { return Err("multidisk_edit_unsupported"); }
        if [u32_at(fixed, 20)?, u32_at(fixed, 24)?, u32_at(fixed, 42)?].contains(&u32::MAX) {
            return Err("zip64_edit_unsupported");
        }
        let end = position.checked_add(46 + name_len + extra_len + comment_len).ok_or("invalid_zip_metadata")?;
        let mut central = original.get(position..end).ok_or("invalid_zip_metadata")?.to_vec();
        check_extra(&central[46 + name_len..46 + name_len + extra_len])?;
        let published_offset = u32::try_from(out.len()).map_err(|_| "output_too_large")?;
        let old_file = old.by_index(i).map_err(|_| "invalid_zip")?;
        if changed.contains_key(old_file.name()) {
            let new_file = new.by_name(old_file.name()).map_err(|_| "invalid_zip")?;
            let new_offset = usize::try_from(new_file.header_start()).map_err(|_| "invalid_zip_metadata")?;
            let new_header = generated.get(new_offset..new_offset + 30).ok_or("invalid_zip_metadata")?;
            let mut local = original.get(offset..offset + 30).ok_or("invalid_zip_metadata")?.to_vec();
            if &local[..4] != b"PK\x03\x04" { return Err("invalid_zip_metadata"); }
            let local_name = u16_at(&local, 26)? as usize;
            let local_extra = u16_at(&local, 28)? as usize;
            let local_end = offset.checked_add(30 + local_name + local_extra).ok_or("invalid_zip_metadata")?;
            let variable = original.get(offset + 30..local_end).ok_or("invalid_zip_metadata")?;
            check_extra(&variable[local_name..])?;
            if local_name != name_len || variable[..local_name] != central[46..46 + name_len] {
                return Err("inconsistent_zip_names");
            }
            // Keep original raw name, UTF-8 flag, timestamps, local/central extras,
            // comments and attributes. Stored payload needs no data descriptor.
            let flags = u16_at(&central, 8)? & !8;
            local[4..6].copy_from_slice(&new_header[4..6]);
            local[6..8].copy_from_slice(&flags.to_le_bytes());
            local[8..10].copy_from_slice(&new_header[8..10]);
            local[14..26].copy_from_slice(&new_header[14..26]);
            central[6..8].copy_from_slice(&new_header[4..6]);
            central[8..10].copy_from_slice(&flags.to_le_bytes());
            central[10..12].copy_from_slice(&new_header[8..10]);
            central[16..28].copy_from_slice(&new_header[14..26]);
            out.extend_from_slice(&local);
            out.extend_from_slice(variable);
            let start = usize::try_from(new_file.data_start()).map_err(|_| "invalid_zip_metadata")?;
            let end = start.checked_add(usize::try_from(new_file.compressed_size()).map_err(|_| "invalid_zip_metadata")?).ok_or("invalid_zip_metadata")?;
            out.extend_from_slice(generated.get(start..end).ok_or("invalid_zip_metadata")?);
        } else {
            // Includes the original local extra and data descriptor (if present).
            let end = offsets.iter().copied().filter(|x| *x > offset).min().unwrap_or(central_start);
            out.extend_from_slice(original.get(offset..end).ok_or("invalid_zip_metadata")?);
        }
        central[42..46].copy_from_slice(&published_offset.to_le_bytes());
        headers.push(central);
        position = end;
    }
    // Fail closed instead of discarding ZIP64/digital-signature directory records.
    if original.get(position..position + 4) != Some(b"PK\x05\x06") {
        return Err("unsupported_zip_directory_records");
    }
    // Append only genuinely new, caller-admitted members. Old local records and
    // central metadata above remain untouched except required payload/offsets.
    let mut new_position=usize::try_from(new.central_directory_start()).map_err(|_|"invalid_zip_metadata")?;
    for i in 0..new.len() {
        let f=new.by_index(i).map_err(|_|"invalid_zip")?;
        let fixed=generated.get(new_position..new_position+46).ok_or("invalid_zip_metadata")?;
        if &fixed[..4]!=b"PK\x01\x02" {return Err("invalid_zip_metadata");}
        let end=new_position.checked_add(46+u16_at(fixed,28)? as usize+u16_at(fixed,30)? as usize+u16_at(fixed,32)? as usize).ok_or("invalid_zip_metadata")?;
        if old.by_name(f.name()).is_err() {
            if !changed.contains_key(f.name()) || u16_at(fixed,8)? & 8!=0 {return Err("unsupported_added_member");}
            let mut central=generated.get(new_position..end).ok_or("invalid_zip_metadata")?.to_vec();
            let offset=u32::try_from(out.len()).map_err(|_|"output_too_large")?;
            let a=usize::try_from(f.header_start()).map_err(|_|"invalid_zip_metadata")?;
            let b=usize::try_from(f.data_start().checked_add(f.compressed_size()).ok_or("invalid_zip_metadata")?).map_err(|_|"invalid_zip_metadata")?;
            out.extend_from_slice(generated.get(a..b).ok_or("invalid_zip_metadata")?);
            central[42..46].copy_from_slice(&offset.to_le_bytes());headers.push(central);
        }
        new_position=end;
    }
    let count = u16::try_from(headers.len()).map_err(|_| "excess_entries")?;
    let start = u32::try_from(out.len()).map_err(|_| "output_too_large")?;
    for header in headers { out.extend_from_slice(&header); }
    let size = u32::try_from(out.len() - start as usize).map_err(|_| "output_too_large")?;
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&(old.comment().len() as u16).to_le_bytes());
    out.extend_from_slice(old.comment());
    Ok(out)
}
