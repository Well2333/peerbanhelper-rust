//! 自更新：下载对应平台的 release 资产（gzip 压缩的裸二进制），替换当前可执行文件并重启。
//!
//! 资产命名约定（见 .github/workflows/release.yml）：`pbh-rust-<target>.gz`，
//! 例如 `pbh-rust-x86_64-unknown-linux-gnu.gz`。仅需 flate2 解压（无需 tar）。
//!
//! 安全性：
//! - 写临时文件 → 校验大小 → 原子 rename 替换；失败不动原文件。
//! - 需要对可执行文件所在目录有写权限（root 安装位置下的非 root 进程会失败并返回错误）。
//! - 替换后通过后台任务延迟 ~1.2s（让 HTTP 响应先回）再重启：unix 用 exec 原地替换进程映像，
//!   其它平台 spawn 新进程并退出（依赖端口快速释放 / serve 的绑定重试）。

use std::io::Read;
use std::time::Duration;

/// 当前平台对应的 release target 三元组（与 release.yml 矩阵一致）。无匹配返回 None。
pub fn current_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        _ => None,
    }
}

/// 本平台自更新资产名（无对应平台返回 None）。
pub fn asset_name() -> Option<String> {
    current_target().map(|t| format!("pbh-rust-{t}.gz"))
}

/// 下载 gzip 资产并解压为二进制字节；校验大小（>1MB）防止损坏/错包。
pub async fn download_and_extract(
    client: &reqwest::Client,
    asset_url: &str,
) -> Result<Vec<u8>, String> {
    let resp = client
        .get(asset_url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let gz = resp.bytes().await.map_err(|e| format!("读取响应失败: {e}"))?;
    let mut dec = flate2::read::GzDecoder::new(&gz[..]);
    let mut bin = Vec::new();
    dec.read_to_end(&mut bin)
        .map_err(|e| format!("解压失败: {e}"))?;
    if bin.len() < 1024 * 1024 {
        return Err(format!("解压结果过小（{} 字节），疑似损坏", bin.len()));
    }
    Ok(bin)
}

/// 把新二进制原子替换到 `exe` 路径：写 `.new` → 设可执行 → 把原文件移到 `.old` → 新文件就位。
/// 失败尽力回滚，不破坏原文件。
pub fn replace_exe(bin: &[u8], exe: &std::path::Path) -> Result<(), String> {
    let new = exe.with_extension("new");
    let old = exe.with_extension("old");
    std::fs::write(&new, bin).map_err(|e| format!("写入新文件失败（检查写权限）: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("设置可执行权限失败: {e}"))?;
    }
    let _ = std::fs::remove_file(&old);
    // 把正在运行的二进制移开（unix 允许 rename 运行中的文件），再把新文件就位。
    std::fs::rename(exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(&new);
        format!("替换失败（无法移动原文件，检查写权限）: {e}")
    })?;
    if let Err(e) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&old, exe); // 尽力回滚
        let _ = std::fs::remove_file(&new);
        return Err(format!("替换失败（无法就位新文件）: {e}"));
    }
    let _ = std::fs::remove_file(&old); // unix 可删；windows 运行中被占用则留存，无碍
    Ok(())
}

/// 下载 gzip 资产 → 解压 → 替换当前可执行文件。成功返回替换后的可执行路径。
pub async fn download_and_replace(
    client: &reqwest::Client,
    asset_url: &str,
) -> Result<std::path::PathBuf, String> {
    let bin = download_and_extract(client, asset_url).await?;
    let exe = std::env::current_exe().map_err(|e| format!("无法定位当前程序: {e}"))?;
    replace_exe(&bin, &exe)?;
    Ok(exe)
}

/// 延迟后重启进程以加载新二进制。后台执行，不阻塞调用方（让 HTTP 响应先返回）。
pub fn spawn_restart(exe: std::path::PathBuf) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        tracing::warn!("自更新完成，正在重启以加载新版本…");
        let args: Vec<String> = std::env::args().skip(1).collect();
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // exec 原地替换进程映像；仅在失败时返回。
            let err = std::process::Command::new(&exe).args(&args).exec();
            tracing::error!("重启 exec 失败: {err}；请手动重启程序。");
            std::process::exit(1);
        }
        #[cfg(not(unix))]
        {
            let _ = std::process::Command::new(&exe).args(&args).spawn();
            std::process::exit(0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_matches_release_convention() {
        // 当前测试主机即 release 矩阵之一时，资产名应为 pbh-rust-<target>.gz。
        if let Some(t) = current_target() {
            assert_eq!(asset_name().unwrap(), format!("pbh-rust-{t}.gz"));
            assert!(t.contains('-'));
        }
    }

    #[test]
    fn linux_x86_64_target() {
        // 本仓库 CI 主力平台。
        if std::env::consts::OS == "linux" && std::env::consts::ARCH == "x86_64" {
            assert_eq!(current_target(), Some("x86_64-unknown-linux-gnu"));
        }
    }

    #[test]
    fn replace_exe_swaps_content_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("pbh-selfupd-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let exe = dir.join("pbh-fake");
        std::fs::write(&exe, b"OLD-BINARY-CONTENT").unwrap();
        let new_bin = b"NEW-BINARY-CONTENT-1234567890".to_vec();
        replace_exe(&new_bin, &exe).unwrap();
        // 内容已替换
        assert_eq!(std::fs::read(&exe).unwrap(), new_bin);
        // 临时文件清理
        assert!(!exe.with_extension("new").exists());
        assert!(!exe.with_extension("old").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "应可执行");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    #[tokio::test]
    async fn download_extract_roundtrip_and_size_guard() {
        use axum::routing::get;
        use axum::Router;

        // >1MB 合法负载 + <1MB 过小负载，分别用 gzip 提供。
        let big = vec![0xABu8; 1_200_000];
        let small = vec![0x01u8; 1000];
        let big_gz = gzip(&big);
        let small_gz = gzip(&small);

        let app = Router::new()
            .route("/ok.gz", get(move || { let g = big_gz.clone(); async move { g } }))
            .route("/small.gz", get(move || { let g = small_gz.clone(); async move { g } }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = pbh_net::build_client("", std::time::Duration::from_secs(10));
        // 正常：下载 + 解压还原原始字节。
        let got = download_and_extract(&client, &format!("http://{addr}/ok.gz"))
            .await
            .expect("应成功");
        assert_eq!(got.len(), big.len());
        assert_eq!(got, big);
        // 过小：被大小守卫拒绝。
        let err = download_and_extract(&client, &format!("http://{addr}/small.gz"))
            .await
            .unwrap_err();
        assert!(err.contains("过小"), "应因过小被拒: {err}");
    }
}
