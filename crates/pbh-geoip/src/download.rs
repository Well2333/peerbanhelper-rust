//! GeoIP 自动下载(对齐上游 util/ipdb/IPDB.java)。
//!
//! 三镜像按序回退;account-id/license-key 仅在镜像 401 时作 Basic 回退;
//! 文件缺失或(auto-update && 超 45 天)才重下。下载经 pbh-net 代理客户端。

use std::path::{Path, PathBuf};

/// 45 天(上游 updateInterval = 3888000000 ms)。
pub const UPDATE_INTERVAL_MS: u128 = 3_888_000_000;

/// 三个镜像源(按序回退)。
pub const MIRRORS: &[&str] = &[
    "https://github.com/PBH-BTN/GeoLite.mmdb/releases/latest/download/",
    "https://pbh-static.paulzzh.com/ipdb/",
    "https://pbh-static.ghostchu.com/ipdb/",
];

/// 需要的库文件名(对齐上游)。
pub const FILES: &[&str] = &["GeoIP-City.mmdb", "GeoIP-ASN.mmdb", "GeoCN.mmdb"];

/// 某文件是否需要(重新)下载:不存在 → true;存在且 auto_update 且 mtime 超 45 天 → true。
pub fn needs_download(path: &Path, auto_update: bool) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return true; // 不存在
    };
    if !auto_update {
        return false;
    }
    let Ok(modified) = meta.modified() else { return false };
    match modified.elapsed() {
        Ok(age) => age.as_millis() > UPDATE_INTERVAL_MS,
        Err(_) => false,
    }
}

/// 拼接镜像 base + 文件名。
pub fn url_for(mirror: &str, file: &str) -> String {
    format!("{mirror}{file}")
}

/// 下载一个库到目标路径。逐镜像尝试;镜像 401 时带上 Basic 凭证重试该镜像。成功返回 true。
pub async fn download_one(
    client: &reqwest::Client,
    dir: &Path,
    file: &str,
    account_id: &str,
    license_key: &str,
) -> bool {
    let dest: PathBuf = dir.join(file);
    for mirror in MIRRORS {
        let url = url_for(mirror, file);
        for with_auth in [false, true] {
            if with_auth && (account_id.is_empty() || license_key.is_empty()) {
                break; // 无凭证不必再试 auth
            }
            let mut req = client.get(&url);
            if with_auth {
                req = req.basic_auth(account_id, Some(license_key));
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    Ok(bytes) => {
                        if let Some(p) = dest.parent() {
                            let _ = std::fs::create_dir_all(p);
                        }
                        if std::fs::write(&dest, &bytes).is_ok() {
                            tracing::info!("GeoIP 已下载 {file}({mirror}, {} bytes)", bytes.len());
                            return true;
                        }
                    }
                    Err(e) => tracing::warn!("GeoIP {file} 读取响应失败({mirror}): {e}"),
                },
                Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED && !with_auth => {
                    continue; // 进入 with_auth 重试
                }
                Ok(resp) => tracing::warn!("GeoIP {file} 镜像 {mirror} 返回 {}", resp.status()),
                Err(e) => tracing::warn!("GeoIP {file} 镜像 {mirror} 失败: {e}"),
            }
        }
    }
    tracing::warn!("GeoIP {file} 所有镜像均失败");
    false
}

/// 确保全部库就绪:对每个需要的文件按需下载。返回是否有任一成功下载(用于决定是否热替换)。
pub async fn ensure_databases(
    client: &reqwest::Client,
    dir: &Path,
    auto_update: bool,
    account_id: &str,
    license_key: &str,
) -> bool {
    let mut any = false;
    for file in FILES {
        let path = dir.join(file);
        if needs_download(&path, auto_update)
            && download_one(client, dir, file, account_id, license_key).await
        {
            any = true;
        }
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_join() {
        assert_eq!(
            url_for(MIRRORS[0], "GeoIP-City.mmdb"),
            "https://github.com/PBH-BTN/GeoLite.mmdb/releases/latest/download/GeoIP-City.mmdb"
        );
    }

    #[test]
    fn missing_file_needs_download() {
        assert!(needs_download(Path::new("/nonexistent/x.mmdb"), false));
        assert!(needs_download(Path::new("/nonexistent/x.mmdb"), true));
    }

    #[test]
    fn fresh_file_no_download_when_autoupdate_off() {
        let dir = std::env::temp_dir().join("pbh-geoip-fresh-test");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("fresh.mmdb");
        std::fs::write(&f, b"x").unwrap();
        assert!(!needs_download(&f, false));
        assert!(!needs_download(&f, true)); // 刚写,未超 45 天
    }
}
