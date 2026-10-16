//! Start Menu import policy, shared by the picker and the final duplicate check.
use std::{
    collections::HashSet, os::windows::process::CommandExt, path::PathBuf, process::Command,
};

use crate::{config::Tool, search::SearchKey, win::InstalledApp};

pub fn target_key(target: &str) -> String {
    crate::win::expand_env(target.trim().trim_matches('"'))
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

pub struct PinnedScan {
    pub apps: Vec<InstalledApp>,
    pub warnings: Vec<String>,
}

/// Read the current user's pins through Windows' read-only layout exporter.
/// Never fall back to All Apps: that would silently change the import source.
pub fn scan_pinned() -> Result<PinnedScan, String> {
    let dir = std::env::temp_dir().join(format!("LovelyLauncher-pins-{}", crate::config::new_id()));
    std::fs::create_dir(&dir).map_err(|e| format!("无法创建固定布局临时目录：{e}"))?;
    let path = dir.join("pins.json");
    let result = (|| {
        let system = std::env::var_os("SystemRoot").ok_or("找不到 Windows 系统目录")?;
        let powershell =
            PathBuf::from(system).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let escaped = path.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); Export-StartLayout -Path '{escaped}'"
        );
        let output = Command::new(powershell)
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output()
            .map_err(|e| format!("无法读取开始菜单已固定布局：{e}"))?;
        if !output.status.success() {
            return Err(format!(
                "读取已固定图标失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("无法读取固定布局：{e}"))?;
        parse_pins(&text)
    })();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
    result
}

fn parse_pins(text: &str) -> Result<PinnedScan, String> {
    let layout: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("无法解析已固定布局：{e}"))?;
    let pins = layout
        .get("pinnedList")
        .and_then(|v| v.as_array())
        .ok_or("当前 Windows 未返回已固定图标列表（需要 Windows 11）")?;
    let mut scan = PinnedScan {
        apps: Vec::new(),
        warnings: Vec::new(),
    };
    for (index, pin) in pins.iter().enumerate() {
        let (name, target) = if let Some(link) = pin.get("desktopAppLink").and_then(|v| v.as_str())
        {
            let path = PathBuf::from(crate::win::expand_env(link));
            let name = crate::win::shortcut_name(&path).unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
            if !path.exists() {
                scan.warnings.push(format!(
                    "第 {} 个固定项目“{name}”的快捷方式已不存在",
                    index + 1
                ));
                continue;
            }
            (name, path)
        } else if let Some(id) = pin
            .get("packagedAppId")
            .or_else(|| pin.get("packagedAppID"))
            .or_else(|| pin.get("desktopAppId"))
            .or_else(|| pin.get("desktopAppID"))
            .and_then(|v| v.as_str())
        {
            let target = format!(r"shell:AppsFolder\{id}");
            let name = crate::win::shell_item_name(&target).unwrap_or_else(|| id.to_string());
            (name, PathBuf::from(target))
        } else {
            scan.warnings
                .push(format!("第 {} 个固定项目暂不支持导入", index + 1));
            continue;
        };
        // The export array is the user's pinned order, not an alphabetic list.
        scan.apps.push(InstalledApp { name, path: target });
    }
    Ok(scan)
}

pub struct ImportEntry {
    pub app: InstalledApp,
    pub key: SearchKey,
    pub added: bool,
    pub selected: bool,
}

pub fn entries(apps: Vec<InstalledApp>, tools: &[Tool]) -> Vec<ImportEntry> {
    let existing: HashSet<_> = tools.iter().map(|t| target_key(&t.target)).collect();
    let mut seen = HashSet::new();
    apps.into_iter()
        .filter_map(|app| {
            let key = target_key(&app.path.to_string_lossy());
            if !seen.insert(key.clone()) {
                return None;
            }
            let added = existing.contains(&key);
            Some(ImportEntry {
                key: SearchKey::new(&app.name),
                app,
                added,
                selected: !added,
            })
        })
        .collect()
}

/// Recheck duplicates at commit time, including duplicates in the same batch.
pub fn import(
    tools: &mut Vec<Tool>,
    apps: impl IntoIterator<Item = InstalledApp>,
    category: &str,
) -> usize {
    let mut seen: HashSet<_> = tools.iter().map(|t| target_key(&t.target)).collect();
    let mut count = 0;
    for app in apps {
        let target = app.path.to_string_lossy().into_owned();
        if !seen.insert(target_key(&target)) {
            continue;
        }
        tools.push(Tool {
            name: app.name,
            target,
            category: category.into(),
            color: "pink".into(),
            ..Default::default()
        });
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app(name: &str, path: &str) -> InstalledApp {
        InstalledApp {
            name: name.into(),
            path: path.into(),
        }
    }

    #[test]
    fn import_preserves_shortcuts_and_ignores_path_case() {
        let mut tools = vec![Tool {
            target: r"C:\Menu\Editor.lnk".into(),
            ..Default::default()
        }];
        let n = import(
            &mut tools,
            vec![
                app("Editor", "c:/menu/EDITOR.lnk"),
                app("终端", r"shell:AppsFolder\Vendor.Terminal!App"),
                app("终端重复", r"SHELL:AppsFolder\vendor.terminal!app"),
                app("工具", r"C:\Menu\工具.lnk"),
            ],
            "常用软件",
        );
        assert_eq!(n, 2);
        assert_eq!(tools[1].target, r"shell:AppsFolder\Vendor.Terminal!App");
        assert_eq!(tools[2].target, r"C:\Menu\工具.lnk");
        assert_eq!(tools[2].category, "常用软件");
        assert!(
            tools[2].icon.is_empty() && tools[2].args.is_empty() && tools[2].workdir.is_empty()
        );
    }

    #[test]
    fn every_pinned_item_is_selected_in_original_order() {
        let entries = entries(
            vec![
                app("编辑器", r"C:\Menu\Editor.lnk"),
                app("卸载编辑器", r"C:\Menu\Uninstall.lnk"),
                app("官网", r"C:\Menu\Site.url"),
            ],
            &[],
        );
        assert!(entries[0].selected);
        assert!(entries[1].selected);
        assert!(entries[2].selected);
        assert_eq!(entries[0].app.name, "编辑器");
        assert_eq!(entries[1].app.name, "卸载编辑器");
        assert_eq!(entries[2].app.name, "官网");
    }

    #[test]
    fn existing_targets_are_locked_and_same_name_apps_remain_distinct() {
        let tools = vec![Tool {
            target: r"C:\Menu\Editor.lnk".into(),
            ..Default::default()
        }];
        let entries = entries(
            vec![
                app("Editor", "c:/menu/editor.lnk"),
                app("Editor", r"C:\Other\Editor.lnk"),
                app("重复快捷方式", r"C:\MENU\Editor.lnk"),
            ],
            &tools,
        );
        assert_eq!(entries.len(), 2);
        assert!(entries[0].added && !entries[0].selected);
        assert!(!entries[1].added && entries[1].selected);
    }

    #[test]
    fn imported_tools_roundtrip_through_json() {
        let mut tools = Vec::new();
        import(
            &mut tools,
            vec![
                app("带参数的应用", r"C:\Start Menu\App.lnk"),
                app("计算器", r"shell:AppsFolder\Vendor.Calc!App"),
            ],
            "常用",
        );
        let json = serde_json::to_string(&tools).unwrap();
        let restored: Vec<Tool> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].target, tools[0].target);
        assert_eq!(restored[1].target, tools[1].target);
        assert_ne!(restored[0].id, restored[1].id);
        assert_eq!(
            import(
                &mut tools,
                restored.into_iter().map(|t| app(&t.name, &t.target)),
                "另一分类"
            ),
            0
        );
    }

    #[test]
    #[ignore = "requires a Windows desktop with installed Start Menu apps"]
    fn live_pinned_scan_and_icons() {
        let scan = scan_pinned().unwrap();
        assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
        let apps = scan.apps;
        let packaged: Vec<_> = apps
            .iter()
            .filter(|a| a.path.to_string_lossy().starts_with("shell:AppsFolder\\"))
            .collect();
        println!(
            "Pinned Start Menu: total={}, shell apps={}",
            apps.len(),
            packaged.len()
        );
        assert!(!apps.is_empty());
        assert!(
            !packaged.is_empty(),
            "expected packaged apps on this test machine"
        );
        let icons = packaged
            .iter()
            .filter(|a| crate::win::app_icon(&a.path.to_string_lossy()).is_some())
            .count();
        println!("Packaged shell icons: {icons}/{}", packaged.len());
        assert!(icons > 0);
    }

    #[test]
    fn layout_parser_preserves_shell_identity_and_reports_unsupported_pins() {
        let scan = parse_pins(r#"{"pinnedList":[{"packagedAppId":"Vendor.Z!App"},{"secondaryTile":{}},{"desktopAppId":"Vendor.A"}]}"#).unwrap();
        assert_eq!(scan.apps.len(), 2);
        assert_eq!(
            scan.apps[0].path.to_string_lossy(),
            r"shell:AppsFolder\Vendor.Z!App"
        );
        assert_eq!(
            scan.apps[1].path.to_string_lossy(),
            r"shell:AppsFolder\Vendor.A"
        );
        assert_eq!(scan.warnings.len(), 1);
        assert!(parse_pins("{}").is_err());
        assert!(parse_pins(r#"{"pinnedList":[]}"#).unwrap().apps.is_empty());
    }
}
