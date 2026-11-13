//! `oh` CLI: legacy `open-hancom-core` grammar, exit codes and error codes.
//! Host I/O (oh-host) → pure byte processing (oh-document) → host publication.
mod args;
mod doctor;
mod install;
mod library;

use oh_document::{Package, Prepared, Result, parse_contract};
use serde_json::{Value, json};
use std::path::Path;

/// Host read of an HWPX package followed by pure byte admission.
fn open(path: &str) -> Result<Package> {
    Package::from_bytes(oh_host::bounded_read(Path::new(path), oh_document::MAX_INPUT)?)
}

/// Read a JSON contract (plan/spec) with the legacy 2 MiB cap.
fn contract(path: &str) -> Result<Vec<u8>> {
    oh_host::bounded_read(Path::new(path), oh_document::MAX_CONTRACT)
}

/// Host publication of prepared bytes; the receipt gains the output path only
/// after the no-clobber commit succeeded.
fn publish(prepared: Prepared, target: &str) -> Result<Value> {
    let Prepared { bytes, mut receipt } = prepared;
    oh_host::publish_no_clobber(Path::new(target), &bytes)?;
    receipt["path"] = json!(Path::new(target));
    Ok(receipt)
}

fn run(args: &[String]) -> Result<Value> {
    // NOTE: evaluation order (spec read → source open → spec parse, or spec read →
    // spec parse → source open) mirrors the legacy binary so that the first
    // reported error code is identical for inputs with several defects.
    match args {
        [cmd] if cmd == "schema" => Ok(oh_document::discovery()),
        [cmd, source] if cmd == "validate" => Ok(open(source)?.validate_supported()),
        [cmd, source] if cmd == "inspect" => {
            let p = open(source)?;
            serde_json::to_value(p.inspection).map_err(|_| "serialization_error")
        }
        [cmd, before, after] if cmd == "diff" => Ok(open(before)?.diff(&open(after)?)),
        [cmd, flag, spec, out, target] if cmd == "create" && flag == "--spec" && out == "--output" => {
            let raw = contract(spec)?;
            let spec = parse_contract(&raw, "invalid_create_spec")?;
            publish(oh_document::create_bytes(spec)?, target)
        }
        [cmd, source, flag, spec, out, target] if flag == "--spec" && out == "--output" => {
            match cmd.as_str() {
                // legacy order: read spec, open source, parse spec
                "image-insert" => { let raw = contract(spec)?; let p = open(source)?; publish(p.image_insert_bytes(parse_contract(&raw, "invalid_image_insert_spec")?)?, target) }
                "image-replace" => { let raw = contract(spec)?; let p = open(source)?; publish(p.image_replace_bytes(parse_contract(&raw, "invalid_image_replace_spec")?)?, target) }
                "image" => { let raw = contract(spec)?; let p = open(source)?; publish(p.image_bytes(parse_contract(&raw, "invalid_image_spec")?)?, target) }
                "link" => { let raw = contract(spec)?; let p = open(source)?; publish(p.link_bytes(parse_contract(&raw, "invalid_link_spec")?)?, target) }
                // legacy order: read spec, parse spec, open source
                "format" => { let s = parse_contract(&contract(spec)?, "invalid_format_spec")?; publish(open(source)?.format_bytes(s)?, target) }
                "paragraph" => { let s = parse_contract(&contract(spec)?, "invalid_paragraph_spec")?; publish(open(source)?.paragraph_bytes(s)?, target) }
                "cell-text" => { let s = parse_contract(&contract(spec)?, "invalid_cell_text_spec")?; publish(open(source)?.cell_text_bytes(s)?, target) }
                "table" => { let s = parse_contract(&contract(spec)?, "invalid_table_spec")?; publish(open(source)?.table_bytes(s)?, target) }
                "style" => { let s = parse_contract(&contract(spec)?, "invalid_style_spec")?; publish(open(source)?.style_bytes(s)?, target) }
                "border" => { let s = parse_contract(&contract(spec)?, "invalid_border_spec")?; publish(open(source)?.border_bytes(s)?, target) }
                _ => Err("unsupported_command"),
            }
        }
        [cmd, source, flag, plan, out, target] if cmd == "apply" && flag == "--plan" && out == "--output" => {
            let p = parse_contract(&contract(plan)?, "invalid_plan")?;
            publish(open(source)?.apply_bytes(p)?, target)
        }
        _ => Err("unsupported_command"),
    }
}

fn path_flag(f: &std::collections::BTreeMap<String, String>, k: &str) -> Option<std::path::PathBuf> {
    f.get(k).map(std::path::PathBuf::from)
}

fn admin_error(code: &str, detail: &str) -> ! {
    println!("{}", json!({"error": code, "detail": detail, "written": false}));
    std::process::exit(2)
}

/// mcp / install / uninstall / doctor. These do not use the legacy `run` grammar.
fn admin(cmd: &str, rest: &[String]) -> Value {
    let flags = |valued: &[&str], sw: &[&str]| args::parse(rest, valued, sw).unwrap_or_else(|d| admin_error("invalid_arguments", &d));
    let or_die = |r: std::result::Result<std::path::PathBuf, install::Fail>| r.unwrap_or_else(|(c, d)| admin_error(c, &d));
    match cmd {
        "mcp" => {
            let f = flags(&["library", "workdir"], &[]);
            let library = path_flag(&f, "library").unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("library"))).unwrap_or_default());
            let workdir = path_flag(&f, "workdir").unwrap_or_else(|| or_die(install::default_workdir()));
            let mut server = oh_mcp::Server::new(oh_mcp::Config { library, workdir });
            let code = match server.serve(std::io::stdin().lock(), std::io::stdout().lock()) {
                Ok(()) => 0,
                Err(_) => 1,
            };
            std::process::exit(code)
        }
        "library" => {
            let Some(sub) = rest.first() else { admin_error("invalid_arguments", "oh library list|add|remove|check|scan|anonymize|manifest") };
            let f = args::parse(&rest[1..], &["root", "path", "name", "terms", "map", "output", "version"], &[]).unwrap_or_else(|d| admin_error("invalid_arguments", &d));
            let root = path_flag(&f, "root").unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(std::path::Path::to_path_buf)).unwrap_or_default());
            let only = |allowed: &[&str]| if let Some(k) = f.keys().find(|k| !allowed.contains(&k.as_str())) { admin_error("invalid_arguments", &format!("oh library {sub}에는 --{k}를 쓰지 않는다")) };
            let need = |k: &str| path_flag(&f, k).unwrap_or_else(|| admin_error("invalid_arguments", &format!("--{k} 필요")));
            // authoring commands print a report and exit 2 when it is not clean
            let report = |r: std::result::Result<(Value, bool), library::Fail>| -> ! {
                let (v, ok) = r.unwrap_or_else(|(c, d)| admin_error(c, &d));
                println!("{v}");
                std::process::exit(if ok { 0 } else { 2 })
            };
            match sub.as_str() {
                "scan" => {
                    only(&["path", "terms"]);
                    report(library::scan(&need("path"), &need("terms")))
                }
                "anonymize" => {
                    only(&["path", "map", "output", "terms"]);
                    report(library::anonymize(&need("path"), &need("map"), &need("output"), path_flag(&f, "terms").as_deref()))
                }
                "manifest" => {
                    only(&["path", "version", "name", "terms"]);
                    report(library::manifest(&need("path"), f.get("version").map(String::as_str), f.get("name").map(String::as_str), path_flag(&f, "terms").as_deref()))
                }
                "check" => {
                    only(&["path", "name", "terms", "root"]);
                    report(library::check(&root, &need("path"), f.get("name").map(String::as_str), path_flag(&f, "terms").as_deref()))
                }
                _ => {}
            }
            only(&["root", "path", "name"]);
            let r = match sub.as_str() {
                "list" if !f.contains_key("path") && !f.contains_key("name") => library::list(&root),
                "add" => match path_flag(&f, "path") {
                    Some(p) => library::add(&root, &p, f.get("name").map(String::as_str)),
                    None => admin_error("invalid_arguments", "--path <팩 폴더나 zip> 필요"),
                },
                "remove" if !f.contains_key("path") => match f.get("name") {
                    Some(n) => library::remove(&root, n),
                    None => admin_error("invalid_arguments", "--name <팩 이름> 필요"),
                },
                _ => admin_error("invalid_arguments", "oh library list [--root R] | add --path P [--name N] [--root R] | remove --name N [--root R] | check --path P [--terms T] [--name N] [--root R] | scan --path P --terms T | anonymize --path IN --map M --output OUT [--terms T] | manifest --path DIR [--version V] [--name N] [--terms T]"),
            };
            r.unwrap_or_else(|(c, d)| admin_error(c, &d))
        }
        "doctor" => {
            let f = flags(&["library", "workdir"], &[]);
            let library = path_flag(&f, "library").unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("library"))).unwrap_or_default());
            let workdir = path_flag(&f, "workdir").unwrap_or_else(|| or_die(install::default_workdir()));
            let (report, ok) = doctor::run(&library, &workdir);
            println!("{report}");
            std::process::exit(if ok { 0 } else { 2 })
        }
        _ => {
            let f = flags(&["client", "config", "root", "workdir", "library"], &["remove-files", "force-running"]);
            let client = f.get("client").and_then(|c| install::Client::parse(c)).unwrap_or_else(|| admin_error("invalid_arguments", "--client <claude-desktop|chatgpt-desktop|codex|claude-code> 필요"));
            let o = install::Opts { client, config: path_flag(&f, "config"), root: path_flag(&f, "root"), workdir: path_flag(&f, "workdir"), library: path_flag(&f, "library"), remove_files: f.contains_key("remove-files"), force_running: f.contains_key("force-running") };
            if cmd == "install" && o.remove_files { admin_error("invalid_arguments", "--remove-files는 uninstall 전용"); }
            if cmd == "uninstall" && (o.workdir.is_some() || o.library.is_some()) { admin_error("invalid_arguments", "--workdir/--library는 install 전용"); }
            let r = if cmd == "install" { install::install(&o) } else { install::uninstall(&o) };
            r.unwrap_or_else(|(c, d)| admin_error(c, &d))
        }
    }
}

fn main() {
    // On Windows args_os is UTF-16; only unpaired surrogates fail conversion.
    let args: Result<Vec<_>> = std::env::args_os().skip(1)
        .map(|arg| arg.into_string().map_err(|_| "non_utf8_argument")).collect();
    if let Ok(a) = &args {
        if let Some(cmd) = a.first().filter(|c| ["mcp", "install", "uninstall", "doctor", "library"].contains(&c.as_str())) {
            println!("{}", admin(cmd, &a[1..]));
            return;
        }
    }
    match args.and_then(|args| run(&args)) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", json!({"error": error, "written": false}));
            std::process::exit(2);
        }
    }
}
