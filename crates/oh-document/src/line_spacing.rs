//! Explicit native advance intent; not Word line-box/clipping/layout parity.
use serde::Deserialize;
use crate::Result;

#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct LineSpacing {
    #[serde(rename="type")]
    kind:Kind,
    value:u32,
    unit:Unit,
}
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
enum Kind {Exact,Minimum}
#[derive(Deserialize,Clone,PartialEq,Eq)]
enum Unit {#[serde(rename="HWPUNIT")] Hwpunit}

/// An absent property retains the legacy 160% create default. Percent and
/// absolute intent cannot both be supplied, even if one is the default value.
pub(crate) fn resolve(percent:Option<u32>,absolute:Option<&LineSpacing>)->Result<(&'static str,u32)> {
    if percent.is_some() && absolute.is_some() {return Err("conflicting_line_spacing_properties");}
    if let Some(a)=absolute {
        if !(1..=1000000).contains(&a.value) {return Err("invalid_line_spacing_measure");}
        return Ok((match a.kind {Kind::Exact=>"FIXED",Kind::Minimum=>"AT_LEAST"},a.value));
    }
    let value=percent.unwrap_or(160);
    if !(1..=1000).contains(&value) {return Err("invalid_paragraph_measure");}
    Ok(("PERCENT",value))
}
