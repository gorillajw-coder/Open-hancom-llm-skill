//! Strict `--flag value` parsing for the new subcommands (mcp/install/uninstall/doctor).
use std::collections::BTreeMap;

/// `valued` flags take one value; `switches` take none. Unknown, duplicate, valueless
/// flags and stray positionals are errors (`invalid_arguments`).
pub fn parse(args: &[String], valued: &[&str], switches: &[&str]) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let Some(name) = a.strip_prefix("--") else { return Err(format!("알 수 없는 인수: {a}")) };
        if out.contains_key(name) {
            return Err(format!("중복 플래그: --{name}"));
        }
        if switches.contains(&name) {
            out.insert(name.to_string(), String::new());
            i += 1;
        } else if valued.contains(&name) {
            let v = args.get(i + 1).filter(|v| !v.starts_with("--") && !v.is_empty()).ok_or_else(|| format!("--{name} 값 필요"))?;
            out.insert(name.to_string(), v.clone());
            i += 2;
        } else {
            return Err(format!("알 수 없는 플래그: --{name}"));
        }
    }
    Ok(out)
}
