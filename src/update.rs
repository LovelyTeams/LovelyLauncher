//! Self-update from the project's GitHub Releases. Blocking; run on a background thread.
//!
//! Install strategy: Windows lets a running exe be *renamed* (not overwritten), so the
//! current exe is moved aside to `*.old`, the new one is copied into its place, and a
//! fresh process is started with `--after-update <pid>`; it waits for the old process
//! to exit, then deletes the `*.old` file.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;
use windows::Win32::{
    Foundation::CloseHandle,
    Security::Cryptography::{
        BCRYPT_ALG_HANDLE, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS, BCRYPT_SHA256_ALGORITHM,
        BCryptCloseAlgorithmProvider, BCryptHash, BCryptOpenAlgorithmProvider,
    },
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

use crate::{
    download::{self, Progress},
    http,
};

pub const REPO: &str = "LovelyTeams/LovelyLauncher";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const ASSET_SUFFIX: &str = "-windows-x64.exe";

#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    /// Version without the leading `v`.
    pub version: String,
    pub notes: String,
    pub page_url: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    pub sums_url: Option<String>,
}

/// Fetch the latest (non-pre-release) release; `Ok(None)` when we're up to date.
pub fn check() -> Result<Option<ReleaseInfo>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let (status, body) = http::fetch(
        &http::Request {
            method: "GET",
            url: &url,
            headers: &[
                ("Accept", "application/vnd.github+json"),
                ("X-GitHub-Api-Version", "2022-11-28"),
            ],
            body: None,
            receive_timeout_ms: 20_000,
        },
        4 * 1024 * 1024,
    )?;
    match status {
        200 => {}
        404 => return Err("还没有发布任何正式版本".into()),
        403 | 429 => return Err("GitHub 接口请求过于频繁，请稍后再试".into()),
        _ => return Err(format!("检查更新失败：HTTP {status}")),
    }
    let release: Value = serde_json::from_slice(&body).map_err(|e| format!("无法解析版本信息：{e}"))?;
    let tag = release["tag_name"].as_str().unwrap_or_default();
    let version = tag.trim_start_matches('v').to_string();
    if !is_newer(&version, CURRENT_VERSION) {
        return Ok(None);
    }
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    let exe = assets
        .iter()
        .find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(ASSET_SUFFIX)))
        .ok_or_else(|| format!("v{version} 没有可下载的 Windows 程序"))?;
    let sums_url = assets
        .iter()
        .find(|a| a["name"].as_str() == Some("SHA256SUMS.txt"))
        .and_then(|a| a["browser_download_url"].as_str().map(str::to_string));
    Ok(Some(ReleaseInfo {
        version,
        notes: release["body"].as_str().unwrap_or_default().to_string(),
        page_url: release["html_url"].as_str().unwrap_or_default().to_string(),
        asset_name: exe["name"].as_str().unwrap_or_default().to_string(),
        asset_url: exe["browser_download_url"].as_str().unwrap_or_default().to_string(),
        asset_size: exe["size"].as_u64().unwrap_or(0),
        sums_url,
    }))
}

/// Compare dotted numeric versions ("0.10.0" > "0.9.3"); a suffix like "-beta" sorts lower.
pub fn is_newer(remote: &str, local: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre)
    }
    let ((mut r, r_pre), (mut l, l_pre)) = (parts(remote), parts(local));
    let len = r.len().max(l.len());
    r.resize(len, 0);
    l.resize(len, 0);
    match r.cmp(&l) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => l_pre && !r_pre,
    }
}

fn staging_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("LovelyLauncher")
        .join("update")
}

/// Download the new exe and verify it against `SHA256SUMS.txt`.
pub fn download(info: &ReleaseInfo, progress: &Progress) -> Result<PathBuf, String> {
    let dest = staging_dir().join(&info.asset_name);
    download::download(&info.asset_url, &dest, progress)?;
    if let Some(sums_url) = &info.sums_url {
        let (status, body) = http::fetch(
            &http::Request {
                method: "GET",
                url: sums_url,
                headers: &[],
                body: None,
                receive_timeout_ms: 20_000,
            },
            64 * 1024,
        )?;
        if status != 200 {
            return Err(format!("无法获取校验文件：HTTP {status}"));
        }
        let sums = String::from_utf8_lossy(&body);
        let expected = sums
            .lines()
            .filter_map(|line| line.split_once(char::is_whitespace))
            .find(|(_, name)| name.trim().trim_start_matches('*') == info.asset_name)
            .map(|(hash, _)| hash.to_ascii_lowercase())
            .ok_or("校验文件中没有这个程序的记录")?;
        let actual = sha256_file(&dest)?;
        if actual != expected {
            let _ = fs::remove_file(&dest);
            return Err("下载的文件校验失败，已删除，请重试".into());
        }
    }
    Ok(dest)
}

/// Swap the running exe for `staged` and start the new version.
/// On success the caller must quit promptly.
pub fn install_and_restart(staged: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let old = exe.with_extension("old");
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|e| {
        format!("无法替换程序文件（{e}）。如果程序在受保护的目录里，请手动下载新版本")
    })?;
    if let Err(e) = fs::copy(staged, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(format!("写入新版本失败：{e}"));
    }
    let _ = fs::remove_file(staged);
    Command::new(&exe)
        .args(["--after-update", &std::process::id().to_string()])
        .spawn()
        .map_err(|e| format!("启动新版本失败：{e}"))?;
    Ok(())
}

/// Called by the new process: wait for the old one to exit, then tidy up.
pub fn finish_after_update(old_pid: u32) {
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, old_pid) {
            WaitForSingleObject(handle, 15_000);
            let _ = CloseHandle(handle);
        }
    }
    remove_leftover();
}

/// Delete the previous exe kept aside by an update. Also run on every start,
/// since a version that predates the updater won't clean up after itself.
pub fn remove_leftover() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(exe.with_extension("old"));
    }
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let data = fs::read(path).map_err(|e| format!("无法读取文件：{e}"))?;
    let mut out = [0u8; 32];
    unsafe {
        let mut alg = BCRYPT_ALG_HANDLE::default();
        BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_SHA256_ALGORITHM, None, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0))
            .ok()
            .map_err(|e| format!("无法计算校验值：{}", e.message()))?;
        let status = BCryptHash(alg, None, &data, &mut out);
        let _ = BCryptCloseAlgorithmProvider(alg, 0);
        status.ok().map_err(|e| format!("无法计算校验值：{}", e.message()))?;
    }
    Ok(out.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
    }

    /// Hits the real GitHub API: `cargo test live_check -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_check() {
        let result = check();
        println!("{result:?}");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn sha256_known_value() {
        let path = std::env::temp_dir().join("lovely-sha256-test.txt");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = fs::remove_file(path);
    }
}
