//! Machine-readable configuration entry points for automation and AI clients.
use crate::{config::Config, external_search, win};
use serde_json::{Value, json};
use std::{io::Read, path::Path};

pub fn read_input(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|_| "无法读取配置文件")?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取配置文件")?;
    if bytes.len() > 1024 * 1024 {
        return Err("配置文件不能超过 1 MB".into());
    }
    String::from_utf8(bytes).map_err(|_| "配置文件需要使用 UTF-8 编码".into())
}

fn execute(args: &[String], position: usize) -> Result<Value, String> {
    let action = args
        .get(position + 1)
        .map(String::as_str)
        .unwrap_or("schema");
    if action == "schema" {
        return Ok(external_search::import_schema());
    }
    if !["validate", "import"].contains(&action) {
        return Err(
            "用法：--search-config schema|validate|import [配置.json] [--output 结果.json]".into(),
        );
    }
    let input = args
        .get(position + 2)
        .filter(|arg| !arg.starts_with("--"))
        .ok_or("请指定 JSON 配置文件")?;
    let text = read_input(Path::new(input))?;
    if action == "validate" {
        let sources = external_search::parse_import(&text, &[], false)?;
        return Ok(json!({"ok":true,"action":action,"count":sources.len()}));
    }
    let _lock = if action == "import" {
        Some(win::lock_configuration()?)
    } else {
        None
    };
    let mut config = Config::load();
    let sources = external_search::parse_import(
        &text,
        &config.settings.external_sources,
        action == "import",
    )?;
    let count = sources.len();
    if action == "import" {
        for source in sources {
            if let Some(old) = config
                .settings
                .external_sources
                .iter_mut()
                .find(|old| old.id == source.id)
            {
                *old = source;
            } else {
                config.settings.external_sources.push(source);
            }
        }
        config.try_save()?;
    }
    Ok(json!({"ok":true,"action":action,"count":count}))
}

pub fn run(args: &[String]) -> Option<i32> {
    let position = args.iter().position(|arg| arg == "--search-config")?;
    let outcome = execute(args, position);
    let success = outcome.is_ok();
    let value = outcome.unwrap_or_else(|error| json!({"ok":false,"error":error}));
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    if let Some(index) = args.iter().position(|arg| arg == "--output") {
        if !args
            .get(index + 1)
            .is_some_and(|path| std::fs::write(path, text.as_bytes()).is_ok())
        {
            eprintln!("无法写入配置操作结果");
            return Some(1);
        }
    } else {
        println!("{text}");
    }
    Some(if success { 0 } else { 1 })
}
