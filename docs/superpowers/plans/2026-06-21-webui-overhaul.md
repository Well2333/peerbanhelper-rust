# WebUI 重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 PeerBanHelper-Rust 补齐 BTN 设置、规则图形化、下载器并入仪表盘、检查更新、版本/GitHub 头部、GeoIP 自动下载、全局代理,并把 Release 包名改为 `pbh-rust`。

**Architecture:** 以新 crate `pbh-net`(代理感知 reqwest 客户端)为公共底座;GeoIP 改成可热替换的 `GeoIpHandle` + 镜像回退下载;BTN 改成可热启停的 `BtnManager`;新增 `config.yml` 读写与检查更新端点;前端单文件 SPA 重写头部、设置页、规则图形化、下载器模态。代理/BTN 改动热加载,无需重启。

**Tech Stack:** Rust(axum 0.7、reqwest 0.12 rustls、serde_yaml、maxminddb、arc-swap、tokio)、vanilla JS 单文件 SPA、GitHub Actions。

参考 spec:`docs/superpowers/specs/2026-06-21-webui-overhaul-design.md`

---

## 阶段总览

- 阶段 1(任务 1–3):网络代理底座 `pbh-net` + 配置 + 接入 BTN/订阅。
- 阶段 2(任务 4–7):GeoIP 热替换 handle + 镜像回退下载 + GeoCN + 端点。
- 阶段 3(任务 8–9):BTN 热启停 `BtnManager` + `config.yml` 读写端点 + 热加载副作用。
- 阶段 4(任务 10):检查更新(版本比较 + 端点)。
- 阶段 5(任务 11–14):前端(头部品牌/版本/GitHub/更新徽标、设置页、下载器并入仪表盘、规则图形化)。
- 阶段 6(任务 15):Release 改名 `pbh-rust`。

每完成一个任务即提交。Rust 任务走 TDD;前端任务以 `cargo build -p pbh-server` 编译 + 浏览器手动验证。

---

## 阶段 1:网络代理底座

### 任务 1:`NetworkConfig` 配置模型

**Files:**
- Modify: `crates/pbh-config/src/model.rs`

- [ ] **Step 1: 写失败测试**

在 `crates/pbh-config/src/model.rs` 的 `mod tests` 内追加:

```rust
    #[test]
    fn network_proxy_roundtrips() {
        let mut a = AppConfig::default();
        assert_eq!(a.network.proxy, "");
        a.network.proxy = "http://127.0.0.1:7890".into();
        let y = serde_yaml::to_string(&a).unwrap();
        assert!(y.contains("network:"));
        assert!(y.contains("proxy: http://127.0.0.1:7890"));
        let back: AppConfig = serde_yaml::from_str(&y).unwrap();
        assert_eq!(back.network.proxy, "http://127.0.0.1:7890");
    }
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test -p pbh-config network_proxy_roundtrips`
Expected: 编译失败(`AppConfig` 无 `network` 字段)。

- [ ] **Step 3: 实现**

在 `AppConfig` 结构体追加字段(放 `ip_database` 之后):

```rust
    pub ip_database: IpDatabaseConfig,
    pub network: NetworkConfig,
}
```

并新增结构体(放 `IpDatabaseConfig` 之后):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
pub struct NetworkConfig {
    /// 出站代理 URL(http/https/socks5);空字符串表示直连。
    pub proxy: String,
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test -p pbh-config`
Expected: 全绿(含新测试)。

- [ ] **Step 5: 提交**

```bash
git add crates/pbh-config/src/model.rs
git commit -m "feat(config): 新增 network.proxy 配置项"
```

---

### 任务 2:新 crate `pbh-net`(代理感知客户端)

**Files:**
- Create: `crates/pbh-net/Cargo.toml`
- Create: `crates/pbh-net/src/lib.rs`
- Modify: `Cargo.toml`(workspace members)

- [ ] **Step 1: 建 crate 骨架**

`crates/pbh-net/Cargo.toml`:

```toml
[package]
name = "pbh-net"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
reqwest = { workspace = true }
tracing = { workspace = true }
url = "2"
```

在根 `Cargo.toml` 的 `[workspace] members` 列表加入 `"crates/pbh-net"`。
确认根 `[workspace.dependencies]` 有 `tracing`;若 `url` 未在 workspace,直接用上面的 `url = "2"`。

- [ ] **Step 2: 写失败测试**

`crates/pbh-net/src/lib.rs`:

```rust
//! pbh-net —— 代理感知的 reqwest 客户端构造。
//!
//! 守则:所有出站请求(BTN/订阅/GeoIP/检查更新)统一经此构造;qBittorrent 下载器除外。
//! 代理为空 → 直连;非空但不可达 → 直连 + warn;非空且可达 → 走代理。

use std::time::Duration;

/// 探测代理 host:port 是否可 TCP 连接(~1s 超时)。proxy 为空返回 false。
pub fn proxy_reachable(proxy: &str) -> bool {
    if proxy.trim().is_empty() {
        return false;
    }
    let Ok(u) = url::Url::parse(proxy) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    let port = u.port_or_known_default().unwrap_or(1080);
    use std::net::ToSocketAddrs;
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs.any(|addr| std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(1000)).is_ok())
}

/// 构造 reqwest 客户端。proxy 为空或不可达 → 直连;否则走代理。
pub fn build_client(proxy: &str, timeout: Duration) -> reqwest::Client {
    let mut b = reqwest::Client::builder().timeout(timeout);
    if !proxy.trim().is_empty() {
        if proxy_reachable(proxy) {
            match reqwest::Proxy::all(proxy) {
                Ok(p) => {
                    tracing::info!("出站代理已启用: {proxy}");
                    b = b.proxy(p);
                }
                Err(e) => tracing::warn!("代理 URL 无效({proxy}),改直连: {e}"),
            }
        } else {
            tracing::warn!("代理不可达({proxy}),本次改直连");
        }
    }
    b.build().unwrap_or_else(|_| reqwest::Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_proxy_not_reachable() {
        assert!(!proxy_reachable(""));
        assert!(!proxy_reachable("   "));
    }

    #[test]
    fn garbage_proxy_not_reachable() {
        assert!(!proxy_reachable("not a url"));
    }

    #[test]
    fn build_client_empty_is_direct() {
        // 仅验证不 panic 且能构造。
        let _c = build_client("", Duration::from_secs(10));
    }

    #[test]
    fn build_client_unreachable_falls_back() {
        // 不可达端口 → 直连,不应 panic。
        let _c = build_client("http://127.0.0.1:1", Duration::from_secs(5));
    }
}
```

- [ ] **Step 3: 运行验证**

Run: `cargo test -p pbh-net`
Expected: 4 个测试通过。

- [ ] **Step 4: 提交**

```bash
git add crates/pbh-net Cargo.toml Cargo.lock
git commit -m "feat(net): 新增 pbh-net 代理感知 HTTP 客户端"
```

---

### 任务 3:BTN 与 IP 订阅接入 `pbh-net`

**Files:**
- Modify: `crates/pbh-btn/Cargo.toml`、`crates/pbh-btn/src/client.rs`(及调用处传入 proxy)
- Modify: `crates/pbh-engine/Cargo.toml`、`crates/pbh-engine/src/ip_rule_list.rs`

> 说明:本任务只把"客户端构造"换成 `pbh_net::build_client`,并让 proxy 可由调用方传入。
> BTN 调用方/模块构建方传 proxy 的完整管线在任务 8/9 接通;此处先让构造函数接受 proxy 形参,
> 调用处暂传 `""`(直连,与现状等价),保证可编译、行为不变。

- [ ] **Step 1: 改 `ip_rule_list.rs` 客户端构造**

先读 `crates/pbh-engine/src/ip_rule_list.rs:50-70` 确认现有 `reqwest::Client::builder()` 上下文。
把该处构造替换为(若该函数能拿到 proxy 则用之,否则先用 `""`):

```rust
        let http = pbh_net::build_client("", std::time::Duration::from_secs(30));
```

在 `crates/pbh-engine/Cargo.toml` 的 `[dependencies]` 加 `pbh-net = { path = "../pbh-net" }`。
保留 `reqwest` 依赖(其它类型仍用到)。

- [ ] **Step 2: 改 `pbh-btn/client.rs` 客户端构造**

读 `crates/pbh-btn/src/client.rs:25-52`。把 `reqwest::Client::builder()...build()` 改为基于 `pbh_net::build_client`。
若需要自定义 header(USER_AGENT/CONTENT_TYPE),保留:把这些 header 改成每请求附加,或保留 `default_headers`。
最小改动方案:新增可选 `proxy: &str` 参数到 `BtnClient::new(...)`,内部:

```rust
        let http = pbh_net::build_client(proxy, std::time::Duration::from_secs(30));
```

(若需 default_headers,改用 `reqwest::Client::builder()` 仍可,但代理逻辑改调用 `pbh_net::proxy_reachable` 决定是否 `.proxy(...)`。两种实现皆可,优先复用 `build_client` + 每请求附加 header。)

在 `crates/pbh-btn/Cargo.toml` 加 `pbh-net = { path = "../pbh-net" }`。
更新 `pbh-btn` 内所有 `BtnClient::new` 调用处传入 proxy(此阶段调用方传 `""`)。

- [ ] **Step 3: 编译验证**

Run: `cargo build -p pbh-engine -p pbh-btn`
Expected: 通过(可能需同步更新 `crates/pbh-server/src/main.rs` 里 BTN 调用,使其传 `""`,详见任务 8 会重做)。
若 server 暂不编译,先只 build 这两个 crate;server 在任务 8 修。

- [ ] **Step 4: 跑相关测试**

Run: `cargo test -p pbh-engine -p pbh-btn`
Expected: 原有测试仍通过。

- [ ] **Step 5: 提交**

```bash
git add crates/pbh-btn crates/pbh-engine
git commit -m "refactor(net): BTN/IP订阅客户端改用 pbh-net(proxy 形参待接通)"
```

---

## 阶段 2:GeoIP 热替换 + 自动下载

### 任务 4:`GeoIpHandle`(arc-swap)+ 全链路改用

**Files:**
- Modify: `crates/pbh-geoip/Cargo.toml`、`crates/pbh-geoip/src/lib.rs`
- Modify: `crates/pbh-engine/src/modules.rs`(`build_modules` 入参)、`crates/pbh-engine/src/ban_manager.rs`(字段)
- Modify: `crates/pbh-web/src/lib.rs`(`WebState.geoip`)、`crates/pbh-web/src/routes.rs`(查询调用)
- Modify: `crates/pbh-server/src/main.rs`(装配)

- [ ] **Step 1: 写失败测试**

在 `crates/pbh-geoip/src/lib.rs` 的 `mod tests` 追加:

```rust
    #[test]
    fn handle_starts_empty_and_installs() {
        let h = GeoIpHandle::new_empty();
        assert!(h.query("1.1.1.1".parse().unwrap()).is_none());
        assert!(!h.is_loaded());
        // 安装一个空 provider 后 is_loaded() 为真。
        struct Dummy;
        impl GeoIpProvider for Dummy {
            fn query(&self, _ip: std::net::IpAddr) -> Option<IpGeoData> {
                Some(IpGeoData::default())
            }
        }
        h.install(std::sync::Arc::new(Dummy));
        assert!(h.is_loaded());
        assert!(h.query("1.1.1.1".parse().unwrap()).is_some());
    }
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test -p pbh-geoip handle_starts_empty_and_installs`
Expected: 编译失败(无 `GeoIpHandle`)。

- [ ] **Step 3: 实现 `GeoIpHandle`**

`crates/pbh-geoip/Cargo.toml` 加依赖:`arc-swap = "1"`。
在 `crates/pbh-geoip/src/lib.rs` 末尾(tests 之前)加:

```rust
use std::sync::Arc;

/// 可热替换的 GeoIP 句柄:后台下载完成后 `install` 新 provider,读取方立即生效。
#[derive(Clone)]
pub struct GeoIpHandle {
    inner: Arc<arc_swap::ArcSwapOption<dyn GeoIpProvider>>,
}

impl GeoIpHandle {
    pub fn new_empty() -> Self {
        GeoIpHandle { inner: Arc::new(arc_swap::ArcSwapOption::empty()) }
    }
    pub fn from_provider(p: Arc<dyn GeoIpProvider>) -> Self {
        let h = Self::new_empty();
        h.install(p);
        h
    }
    pub fn install(&self, p: Arc<dyn GeoIpProvider>) {
        self.inner.store(Some(p));
    }
    pub fn is_loaded(&self) -> bool {
        self.inner.load().is_some()
    }
    pub fn query(&self, ip: std::net::IpAddr) -> Option<IpGeoData> {
        self.inner.load_full().and_then(|p| p.query(ip))
    }
}
```

- [ ] **Step 4: 测试通过**

Run: `cargo test -p pbh-geoip`
Expected: 全绿。

- [ ] **Step 5: 全链路改签名**

按以下顺序改(每改一处用编译器引导):
1. `crates/pbh-engine/src/ban_manager.rs`:把 geoip 字段从 `Option<Arc<dyn GeoIpProvider>>` 改为 `pbh_geoip::GeoIpHandle`;
   `BanManager::new` 的对应入参同改;查询处 `self.geoip.query(ip)`(去掉 `as_ref().and_then`)。
   在 `crates/pbh-engine/Cargo.toml` 确保依赖 `pbh-geoip`(已有)。
2. `crates/pbh-engine/src/modules.rs`:`build_modules` 的 `geoip: &Option<Arc<...>>` 改为 `geoip: &pbh_geoip::GeoIpHandle`;
   内部把 provider 传给需要的模块(`ip_blacklist`)处改为 clone handle 或按需 `query`。
   读 `ip_blacklist.rs` 看它如何持有 geoip;统一改为持 `GeoIpHandle`。
3. `crates/pbh-web/src/lib.rs`:`pub geoip: Option<Arc<dyn GeoIpProvider>>` → `pub geoip: pbh_geoip::GeoIpHandle`;
   在 `crates/pbh-web/Cargo.toml` 确保依赖 `pbh-geoip`(已有)。
4. `crates/pbh-web/src/routes.rs:287` 的 `st.geoip.as_ref().and_then(|g| g.query(...))` 改为 `st.geoip.query(m.peer.ip)`。
5. `crates/pbh-server/src/main.rs`:
   - 构造:`let geoip = MaxmindProvider::load_from_dir(&geoip_dir).map(|p| GeoIpHandle::from_provider(Arc::new(p) as Arc<dyn GeoIpProvider>)).unwrap_or_else(GeoIpHandle::new_empty);`
   - 之后所有 `&geoip` / `geoip.clone()` 入参保持(handle 可 clone)。

- [ ] **Step 6: 全量编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 通过(server 此时仍用 `BtnClient::new(..,"")`)。

- [ ] **Step 7: 提交**

```bash
git add crates
git commit -m "refactor(geoip): 引入可热替换 GeoIpHandle 并全链路改用"
```

---

### 任务 5:GeoIP 镜像回退下载器

**Files:**
- Create: `crates/pbh-geoip/src/download.rs`
- Modify: `crates/pbh-geoip/src/lib.rs`(`mod download;` + 加载搜索名补充)
- Modify: `crates/pbh-geoip/Cargo.toml`(加 `pbh-net`、`tokio`)

- [ ] **Step 1: 加载搜索名补充 `GeoIP-*`**

在 `lib.rs` 的 `load_from_dir` 里把搜索名补成与 PBH 一致(放在最前优先):

```rust
        let city = find(&["GeoIP-City.mmdb", "GeoLite2-City.mmdb", "GeoIP2-City.mmdb", "City.mmdb"]);
        let asn = find(&["GeoIP-ASN.mmdb", "GeoLite2-ASN.mmdb", "GeoIP2-ASN.mmdb", "ASN.mmdb"]);
```

- [ ] **Step 2: 写失败测试(URL 拼接 + 过期判定)**

`crates/pbh-geoip/src/download.rs`:

```rust
//! GeoIP 自动下载(对齐上游 util/ipdb/IPDB.java)。
//!
//! 三镜像按序回退;account-id/license-key 仅在镜像 401 时作 Basic 回退;
//! 文件缺失或(auto-update && 超 45 天)才重下。下载经 pbh-net 代理客户端。

use std::path::{Path, PathBuf};
use std::time::Duration;

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
```

- [ ] **Step 3: 验证失败 → 通过**

Run: `cargo test -p pbh-geoip url_join missing_file_needs_download fresh_file_no_download_when_autoupdate_off`
先确认编译失败(`mod download` 未声明),在 `lib.rs` 顶部加 `pub mod download;`,再跑 → 通过。

- [ ] **Step 4: 实现实际下载函数**

在 `download.rs` 追加(非 TDD,网络函数以手动验证):

```rust
/// 下载一个库到目标路径。逐镜像尝试;镜像 401 时带上 Basic 凭证重试该镜像。
/// 成功返回 true。
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
                Ok(resp) if resp.status().is_success() => {
                    match resp.bytes().await {
                        Ok(bytes) => {
                            if let Some(p) = dest.parent() {
                                let _ = std::fs::create_dir_all(p);
                            }
                            if std::fs::write(&dest, &bytes).is_ok() {
                                tracing::info!("GeoIP 已下载 {file}({}, {} bytes)", mirror, bytes.len());
                                return true;
                            }
                        }
                        Err(e) => tracing::warn!("GeoIP {file} 读取响应失败({mirror}): {e}"),
                    }
                }
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
        if needs_download(&path, auto_update) {
            if download_one(client, dir, file, account_id, license_key).await {
                any = true;
            }
        }
    }
    any
}
```

在 `crates/pbh-geoip/Cargo.toml` 加 `pbh-net = { path = "../pbh-net" }`、`reqwest = { workspace = true }`、`tokio = { workspace = true }`。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo test -p pbh-geoip`
Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add crates/pbh-geoip
git commit -m "feat(geoip): 镜像回退下载器(45天/401 Basic 回退)"
```

---

### 任务 6:GeoCN 解析(填 net_type/cn_*)

**Files:**
- Modify: `crates/pbh-geoip/src/lib.rs`

- [ ] **Step 1: 写失败测试(反序列化结构)**

在 `mod tests` 追加(仅验证结构体可被 serde 处理,不依赖真实库):

```rust
    #[test]
    fn geocn_record_deserializes() {
        // 模拟 GeoCN 记录的 JSON 形态映射到内部结构。
        let json = r#"{"net":"宽带","province":"上海","city":"上海"}"#;
        let r: GeoCnRecord = serde_json::from_str(json).unwrap();
        assert_eq!(r.net.as_deref(), Some("宽带"));
        assert_eq!(r.province.as_deref(), Some("上海"));
        assert_eq!(r.city.as_deref(), Some("上海"));
    }
```

(`pbh-geoip` 的 dev-deps 需有 `serde_json`;若无则在 `[dev-dependencies]` 加 `serde_json = { workspace = true }`。)

- [ ] **Step 2: 实现 GeoCN reader**

在 `MaxmindProvider` 增加第三个 reader 字段 `cn: Option<maxminddb::Reader<Vec<u8>>>`,并定义记录结构:

```rust
#[derive(Debug, serde::Deserialize)]
pub struct GeoCnRecord {
    #[serde(default)] pub net: Option<String>,
    #[serde(default)] pub province: Option<String>,
    #[serde(default)] pub city: Option<String>,
}
```

- `load`/`load_from_dir` 增加 `GeoCN.mmdb` 加载(搜索名 `["GeoCN.mmdb"]`),`cn` 任一可缺。
- 改判定:`if city.is_none() && asn.is_none() && cn.is_none() { return None; }`。
- `query` 末尾补 GeoCN 查询:

```rust
        if let Some(c) = &self.cn {
            if let Ok(rec) = c.lookup::<GeoCnRecord>(ip) {
                d.net_type = rec.net;
                d.cn_province = rec.province;
                d.cn_city = rec.city;
            }
        }
```

更新模块顶部 doc 注释:GeoCN 已支持。

- [ ] **Step 3: 测试通过 + 编译**

Run: `cargo test -p pbh-geoip`
Expected: 全绿。

- [ ] **Step 4: 提交**

```bash
git add crates/pbh-geoip
git commit -m "feat(geoip): GeoCN 解析(net_type/省市)"
```

---

### 任务 7:启动后台下载 + `/api/geoip/update` 端点

**Files:**
- Modify: `crates/pbh-server/src/main.rs`(启动 spawn 后台下载)
- Modify: `crates/pbh-web/src/routes.rs`(新增端点 + 路由)

- [ ] **Step 1: 启动时后台下载并热替换**

在 `main.rs` 构造 `geoip`(handle)之后、Web 启动之前,加后台任务:

```rust
    // GeoIP 自动下载(缺文件或过期);完成后热替换 provider。
    {
        let geoip = geoip.clone();
        let geoip_dir = paths.data_dir().join("geoip");
        let app_cfg = config.current().app.clone();
        tokio::spawn(async move {
            let client = pbh_net::build_client(&app_cfg.network.proxy, std::time::Duration::from_secs(60));
            let changed = pbh_geoip::download::ensure_databases(
                &client, &geoip_dir,
                app_cfg.ip_database.auto_update,
                &app_cfg.ip_database.account_id,
                &app_cfg.ip_database.license_key,
            ).await;
            if changed || !geoip.is_loaded() {
                if let Some(p) = pbh_geoip::MaxmindProvider::load_from_dir(&geoip_dir) {
                    geoip.install(std::sync::Arc::new(p) as std::sync::Arc<dyn pbh_geoip::GeoIpProvider>);
                    tracing::info!("GeoIP 库已就绪并热加载");
                }
            }
        });
    }
```

在 `crates/pbh-server/Cargo.toml` 确保依赖 `pbh-net`(加 `pbh-net = { path = "../pbh-net" }`)。

- [ ] **Step 2: 新增 `/api/geoip/update` 端点**

在 `routes.rs` 的 `protected` 路由加:`.route("/api/geoip/update", post(geoip_update))`。
并实现:

```rust
async fn geoip_update(State(st): State<WebState>) -> Response {
    let app = st.config.current().app.clone();
    let dir = st.paths.data_dir().join("geoip");
    let client = pbh_net::build_client(&app.network.proxy, std::time::Duration::from_secs(60));
    let changed = pbh_geoip::download::ensure_databases(
        &client, &dir, true, &app.ip_database.account_id, &app.ip_database.license_key,
    ).await;
    if changed {
        if let Some(p) = pbh_geoip::MaxmindProvider::load_from_dir(&dir) {
            st.geoip.install(std::sync::Arc::new(p) as std::sync::Arc<dyn pbh_geoip::GeoIpProvider>);
        }
    }
    ApiResp::ok(json!({ "changed": changed, "loaded": st.geoip.is_loaded() })).into_response()
}
```

> 需要 `st.paths`:`WebState` 当前无 `paths` 字段 → 在 `crates/pbh-web/src/lib.rs` 的 `WebState` 加 `pub paths: pbh_config::Paths`,
> 并在 `main.rs` 装配 `web_state` 时传 `ctx.paths.clone()`。`crates/pbh-web/Cargo.toml` 已依赖 `pbh-config`。
> 同时加 `pbh-net = { path = "../pbh-net" }`、`pbh-geoip`(已有)到 `pbh-web/Cargo.toml`。

- [ ] **Step 3: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 通过。

- [ ] **Step 4: 手动验证**

删除/清空 `data/geoip/` → `cargo run -p pbh-server`(或 `./build.sh run`)→ 日志应出现镜像下载尝试与"GeoIP 库已就绪并热加载"(网络可用时)。

- [ ] **Step 5: 提交**

```bash
git add crates/pbh-server crates/pbh-web
git commit -m "feat(geoip): 启动后台下载 + /api/geoip/update 热替换端点"
```

---

## 阶段 3:BTN 热启停 + config.yml 读写

### 任务 8:`BtnManager`(可热启停)+ 启动装配

**Files:**
- Modify: `crates/pbh-btn/src/lib.rs`(`spawn` 返回 `AbortHandle`)
- Create: `crates/pbh-server/src/btn_manager.rs`(或放 `crates/pbh-web`,见下)
- Modify: `crates/pbh-web/src/lib.rs`(`WebState.btn` 改 `Arc<BtnManager>`)
- Modify: `crates/pbh-server/src/main.rs`、`crates/pbh-server/src/context.rs`

> 决策:`BtnManager` 放 `pbh-web`(`crates/pbh-web/src/btn_manager.rs`),因为热加载副作用在 web 端点触发,
> 且需与 `build_modules`/`rebuild_modules` 协作。`pbh-web` 已依赖 `pbh-btn`/`pbh-engine`/`pbh-storage`/`pbh-config`。

- [ ] **Step 1: `pbh_btn::spawn` 返回可中止句柄**

读 `crates/pbh-btn/src/lib.rs` 找 `pub fn spawn(...)`。让其内部 `tokio::spawn` 的 `JoinHandle` 转 `abort_handle()` 返回:

```rust
pub fn spawn(cfg: BtnRuntimeConfig, db: Db, state: SharedBtnState) -> tokio::task::AbortHandle {
    let handle = tokio::spawn(async move { /* 原循环体 */ });
    handle.abort_handle()
}
```

并让 `BtnClient::new` 接收 proxy:在 `BtnRuntimeConfig` 增加 `pub proxy: String`,spawn 内构造 client 时传入。

- [ ] **Step 2: 写 `BtnManager`**

`crates/pbh-web/src/btn_manager.rs`:

```rust
//! BTN 热启停管理:保存 config.yml 后按 enabled/凭证/代理变化中止并重启后台调度。

use std::sync::Mutex;
use pbh_btn::{BtnRuntimeConfig, SharedBtnState};
use pbh_storage::Db;

pub struct BtnManager {
    db: Db,
    installation_id: String,
    inner: Mutex<Option<(tokio::task::AbortHandle, SharedBtnState)>>,
}

impl BtnManager {
    pub fn new(db: Db, installation_id: String) -> Self {
        BtnManager { db, installation_id, inner: Mutex::new(None) }
    }

    /// 当前共享状态(仅启用且运行时为 Some),供 build_modules 决定是否构建 BtnNetworkOnline。
    pub fn current_state(&self) -> Option<SharedBtnState> {
        self.inner.lock().unwrap().as_ref().map(|(_, s)| s.clone())
    }

    /// 停止现有调度。
    pub fn stop(&self) {
        if let Some((handle, _)) = self.inner.lock().unwrap().take() {
            handle.abort();
            tracing::info!("BTN 调度已停止");
        }
    }

    /// 按新配置应用:enabled=false → 停;否则停旧再以新 proxy/凭证起新。
    pub fn apply(&self, app: &pbh_config::AppConfig, proxy: &str, ban_duration: i64) {
        self.stop();
        if !app.btn.enabled {
            return;
        }
        let state = pbh_btn::new_state();
        let handle = pbh_btn::spawn(
            BtnRuntimeConfig {
                config_url: app.btn.config_url.clone(),
                app_id: app.btn.app_id.clone(),
                app_secret: app.btn.app_secret.clone(),
                installation_id: self.installation_id.clone(),
                submit: app.btn.submit,
                ban_duration,
                proxy: proxy.to_string(),
            },
            self.db.clone(),
            state.clone(),
        );
        *self.inner.lock().unwrap() = Some((handle, state));
        tracing::info!("BTN 调度已启动");
    }
}
```

在 `crates/pbh-web/src/lib.rs` 顶部 `mod btn_manager;` 并 `pub use btn_manager::BtnManager;`。
`WebState`:`pub btn_state: Option<SharedBtnState>` → `pub btn: std::sync::Arc<BtnManager>`。

- [ ] **Step 3: `routes.rs` 内 build_modules 调用改用 `st.btn.current_state()`**

`get`/`put_profile`、`save_and_rebuild` 中 `&st.btn_state` → `&st.btn.current_state()`。

- [ ] **Step 4: `main.rs` 装配**

替换原 btn_state 装配块为:

```rust
    let btn_mgr = std::sync::Arc::new(pbh_web::BtnManager::new(db.clone(), installation_id.clone()));
    btn_mgr.apply(&app_cfg, &app_cfg.network.proxy, profile.ban_duration);
    let btn_state = btn_mgr.current_state(); // 供首次 build_modules
```

`build_modules(..., &btn_state)` 保持(此处是 `&Option<SharedBtnState>`)。
`web_state` 装配:`btn: btn_mgr.clone()`(去掉旧 `btn_state`)。
`track_swarm` 逻辑保留(基于 `app_cfg.btn`)。

- [ ] **Step 5: 全量编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 通过。

- [ ] **Step 6: 提交**

```bash
git add crates
git commit -m "feat(btn): BtnManager 可热启停 + spawn 返回 AbortHandle + proxy 接通"
```

---

### 任务 9:`/api/config/app` 读写 + 热加载副作用

**Files:**
- Modify: `crates/pbh-web/src/routes.rs`

- [ ] **Step 1: 加路由**

在 `protected` 加:`.route("/api/config/app", get(get_app_config).put(put_app_config))`。

- [ ] **Step 2: 实现 GET**

```rust
async fn get_app_config(State(st): State<WebState>) -> Response {
    let a = st.config.current().app.clone();
    ApiResp::ok(json!({
        "btn": {
            "enabled": a.btn.enabled,
            "config_url": a.btn.config_url,
            "submit": a.btn.submit,
            "app_id": a.btn.app_id,
            "app_secret": a.btn.app_secret,
        },
        "ip_database": {
            "account_id": a.ip_database.account_id,
            "license_key": a.ip_database.license_key,
            "auto_update": a.ip_database.auto_update,
        },
        "network": { "proxy": a.network.proxy },
        "persist": { "ban_logs_keep_days": a.persist.ban_logs_keep_days },
    })).into_response()
}
```

- [ ] **Step 3: 实现 PUT(合并写回 + 热加载)**

```rust
#[derive(Deserialize)]
struct AppConfigBody {
    btn: Option<BtnBody>,
    ip_database: Option<IpDbBody>,
    network: Option<NetBody>,
    persist: Option<PersistBody>,
}
#[derive(Deserialize)] struct BtnBody { enabled: bool, config_url: String, submit: bool, app_id: String, app_secret: String }
#[derive(Deserialize)] struct IpDbBody { account_id: String, license_key: String, auto_update: bool }
#[derive(Deserialize)] struct NetBody { proxy: String }
#[derive(Deserialize)] struct PersistBody { ban_logs_keep_days: i64 }

async fn put_app_config(State(st): State<WebState>, Json(b): Json<AppConfigBody>) -> Response {
    let mut app = st.config.current().app.clone();
    if let Some(x) = b.btn {
        app.btn.enabled = x.enabled; app.btn.config_url = x.config_url; app.btn.submit = x.submit;
        app.btn.app_id = x.app_id; app.btn.app_secret = x.app_secret;
    }
    if let Some(x) = b.ip_database {
        app.ip_database.account_id = x.account_id; app.ip_database.license_key = x.license_key; app.ip_database.auto_update = x.auto_update;
    }
    if let Some(x) = b.network { app.network.proxy = x.proxy; }
    if let Some(x) = b.persist { app.persist.ban_logs_keep_days = x.ban_logs_keep_days; }
    if let Err(e) = st.config.save_app(&app) {
        return bad_request(format!("保存失败: {e}"));
    }
    // 热加载:重启 BTN(应用新 enabled/凭证/代理),再重建模块。
    let dur = st.config.current().profile.ban_duration;
    st.btn.apply(&app, &app.network.proxy, dur);
    let p = st.config.current().profile.clone();
    let modules = pbh_engine::build_modules(&p, p.ban_duration, st.ban_manager.ban_list(), &st.db, &st.geoip, &st.btn.current_state());
    let n = modules.len();
    st.ban_manager.rebuild_modules(modules);
    ApiResp::ok(json!({ "modules": n })).into_response()
}
```

- [ ] **Step 4: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 通过。

- [ ] **Step 5: 手动验证(curl)**

启动后用 token:
```bash
TOKEN=<日志里的 token>
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:9898/api/config/app | head
curl -s -X PUT -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"network":{"proxy":"http://127.0.0.1:1"}}' http://127.0.0.1:9898/api/config/app
```
Expected:GET 返回当前 BTN/代理;PUT 返回 `{modules:N}`,日志显示 BTN 重启 + 代理不可达直连 warn。

- [ ] **Step 6: 提交**

```bash
git add crates/pbh-web
git commit -m "feat(web): /api/config/app 读写 + BTN/代理热加载"
```

---

## 阶段 4:检查更新

### 任务 10:版本比较 + `/api/update/check`

**Files:**
- Modify: `crates/pbh-web/src/routes.rs`(版本比较 util + 端点)

- [ ] **Step 1: 写失败测试(版本比较)**

在 `routes.rs` 末尾加 `mod tests`(或追加到现有):

```rust
#[cfg(test)]
mod update_tests {
    use super::version_newer;
    #[test]
    fn semver_compare() {
        assert!(version_newer("0.1.0", "0.2.0"));
        assert!(version_newer("0.1.0", "1.0.0"));
        assert!(!version_newer("0.2.0", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.1.0"));
        // 容忍 v 前缀
        assert!(version_newer("v0.1.0", "v0.2.0"));
    }
}
```

- [ ] **Step 2: 实现 `version_newer`**

```rust
/// latest 是否比 current 新(语义化:major.minor.patch,容忍前导 v 与多余段)。
fn version_newer(current: &str, latest: &str) -> bool {
    fn parse(s: &str) -> Vec<u64> {
        s.trim().trim_start_matches('v')
            .split('.')
            .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .map(|d| d.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parse(current), parse(latest));
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if y != x { return y > x; }
    }
    false
}
```

- [ ] **Step 3: 测试通过**

Run: `cargo test -p pbh-web semver_compare`
Expected: 通过。

- [ ] **Step 4: 实现端点**

路由加:`.route("/api/update/check", get(update_check))`。

```rust
async fn update_check(State(st): State<WebState>) -> Response {
    let current = env!("CARGO_PKG_VERSION");
    let app = st.config.current().app.clone();
    let client = pbh_net::build_client(&app.network.proxy, std::time::Duration::from_secs(15));
    let url = "https://api.github.com/repos/Well2333/peerbanhelper-rust/releases/latest";
    let resp = client.get(url).header("User-Agent", "peerbanhelper-rust").send().await;
    match resp {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(j) => {
                let latest = j.get("tag_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let html_url = j.get("html_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let newer = !latest.is_empty() && version_newer(current, &latest);
                ApiResp::ok(json!({ "current": current, "latest": latest, "newer": newer, "html_url": html_url })).into_response()
            }
            Err(e) => bad_request(format!("解析失败: {e}")),
        },
        Ok(r) => bad_request(format!("GitHub 返回 {}", r.status())),
        Err(e) => bad_request(format!("请求失败: {e}")),
    }
}
```

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 通过。

- [ ] **Step 6: 提交**

```bash
git add crates/pbh-web
git commit -m "feat(web): /api/update/check 检查更新(走代理)"
```

---

## 阶段 5:前端(`crates/pbh-web/assets/index.html`)

> 前端为单文件 SPA。每个任务改完后 `cargo build -p pbh-server`(`include_str!` 嵌入)→ 浏览器硬刷新验证。
> 保持现有 CSS 变量与配色风格;新元素复用既有 class(`.card`/`.btn`/`.pill`/`.row`/`.grid` 等)。

### 任务 11:头部品牌 / 版本 / GitHub / 退出图标 / 更新徽标

**Files:**
- Modify: `crates/pbh-web/assets/index.html`(header 区 + JS)

- [ ] **Step 1: 改 header 标记**

把 `crates/pbh-web/assets/index.html:125-138` 的 header 替换为:

```html
<header id="topbar" class="hidden">
  <div class="logo"><span class="dot"></span>PeerBanHelper-Rust<span class="pill gray" id="verTag" style="margin-left:4px">…</span></div>
  <nav id="nav">
    <a href="#dashboard">仪表盘</a>
    <a href="#bans">封禁列表</a>
    <a href="#history">封禁历史</a>
    <a href="#rules">规则配置</a>
    <a href="#settings">设置</a>
    <a href="#logs">实时日志</a>
  </nav>
  <a id="updBadge" class="pill red hidden" href="#" target="_blank" rel="noopener" style="text-decoration:none">有新版本</a>
  <span class="hstat" id="hstat">—</span>
  <a class="hbtn" id="ghBtn" href="https://github.com/Well2333/peerbanhelper-rust" target="_blank" rel="noopener" title="GitHub" aria-label="GitHub">
    <svg width="17" height="17" viewBox="0 0 16 16" fill="currentColor"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z"/></svg>
  </a>
  <button class="hbtn" id="themeBtn" title="切换主题" aria-label="切换主题"></button>
  <button class="hbtn" id="logoutBtn" title="退出" aria-label="退出">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/></svg>
  </button>
</header>
```

- [ ] **Step 2: 主题图标改 SVG 切换 + 版本/更新 JS**

把 `applyTheme` 改为切换内嵌 SVG(月亮/太阳),替换 `index.html:314`:

```js
function applyTheme(t){
  document.documentElement.setAttribute('data-theme',t);
  localStorage.setItem('pbh_theme',t);
  el('themeBtn').innerHTML = t==='dark'
    ? '<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>'
    : '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5L19 19M19 5l-1.5 1.5M6.5 17.5L5 19"/></svg>';
}
```

在 `refreshDashboard` 内设置版本标签(status 已返回 `version`):在拿到 `d` 后加
`el('verTag').textContent='v'+(d.version||'');`。

新增检查更新函数,并在登录成功 / 启动轮询后调用一次:

```js
async function checkUpdate(){
  try{ const j=await api('/api/update/check'); const d=j.data;
    if(d&&d.newer){ const b=el('updBadge'); b.href=d.html_url||'#'; b.textContent='有新版本 '+d.latest; b.classList.remove('hidden'); }
  }catch(e){}
}
```
在 `startPoll()` 末尾加 `checkUpdate();`。

- [ ] **Step 3: 路由表更新(去 downloaders、加 settings)**

`index.html:319` 的 `PAGES` 改为:

```js
const PAGES = ['dashboard','bans','history','rules','settings','logs'];
```
`route()` 内把 `if(h==='downloaders') loadDownloaders();` 改为 `if(h==='settings') loadAppConfig();`(`loadAppConfig` 在任务 12 定义;此步可先留空函数 `function loadAppConfig(){}` 占位避免报错)。

- [ ] **Step 4: 编译 + 验证**

Run: `cargo build -p pbh-server`
浏览器硬刷新:标题为 `PeerBanHelper-Rust vX.Y.Z`;右上 GitHub/主题/退出均有图标;退出图标不再缺字。

- [ ] **Step 5: 提交**

```bash
git add crates/pbh-web/assets/index.html
git commit -m "feat(ui): 头部统一品牌/版本号/GitHub按钮/SVG图标/更新徽标"
```

---

### 任务 12:设置页(BTN / GeoIP / 代理)

**Files:**
- Modify: `crates/pbh-web/assets/index.html`(新增 `page-settings` + JS)

- [ ] **Step 1: 加 settings 页面 DOM**

在 `page-logs` 之前插入:

```html
  <!-- 设置 -->
  <section class="page" id="page-settings">
    <h1 class="ptitle">设置</h1>
    <p class="pdesc">基础设施配置(<code>config.yml</code>)。保存即热生效,无需重启。</p>

    <div class="card" style="margin-bottom:16px">
      <h3>☁️ BTN 云端威胁情报</h3>
      <p class="mut" style="margin-top:-4px">加入 BTN 共享封禁情报。需在 <a href="https://github.com/PBH-BTN/PeerBanHelper" target="_blank">BTN</a> 申请 app-id / app-secret。</p>
      <label class="row" style="gap:6px;margin-bottom:8px"><input type="checkbox" id="cfg_btn_enabled"> 启用 BTN</label>
      <div class="grid">
        <input id="cfg_btn_appid" placeholder="app-id">
        <input id="cfg_btn_secret" type="password" placeholder="app-secret" autocomplete="new-password">
        <input id="cfg_btn_url" placeholder="config-url" style="grid-column:1/3">
      </div>
      <label class="row" style="gap:6px;margin-top:8px"><input type="checkbox" id="cfg_btn_submit"> 上报本地封禁/swarm 数据(贡献网络)</label>
      <div class="row" style="margin-top:10px"><span class="sp"></span><button onclick="saveAppConfig('btn')">保存 BTN</button></div>
    </div>

    <div class="card" style="margin-bottom:16px">
      <h3>🌍 GeoIP / IP 库</h3>
      <p class="mut" style="margin-top:-4px">自动从镜像下载 GeoIP-City / GeoIP-ASN / GeoCN(超 45 天自动更新)。MaxMind 账号仅在镜像需要鉴权时使用。</p>
      <div class="grid">
        <input id="cfg_ip_account" placeholder="MaxMind account-id(可选)">
        <input id="cfg_ip_license" type="password" placeholder="MaxMind license-key(可选)" autocomplete="new-password">
      </div>
      <label class="row" style="gap:6px;margin-top:8px"><input type="checkbox" id="cfg_ip_auto"> 自动更新(超 45 天)</label>
      <div class="row" style="margin-top:10px">
        <span id="geoipStat" class="mut"></span><span class="sp"></span>
        <button class="ghost" onclick="updateGeoip()">立即更新 GeoIP</button>
        <button onclick="saveAppConfig('ip')">保存 IP 库设置</button>
      </div>
    </div>

    <div class="card">
      <h3>🛰️ 网络代理</h3>
      <p class="mut" style="margin-top:-4px">应用于 BTN / IP 订阅 / GeoIP 下载 / 检查更新。留空=直连;代理不可达时自动直连。下载器连接不走代理。</p>
      <input id="cfg_proxy" placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080" style="width:100%">
      <div class="row" style="margin-top:10px"><span class="sp"></span><button onclick="saveAppConfig('net')">保存代理</button></div>
      <div id="cfgMsg" class="mut" style="margin-top:9px"></div>
    </div>
  </section>
```

- [ ] **Step 2: JS 加载/保存逻辑(替换占位 `loadAppConfig`)**

```js
let appCfg={};
async function loadAppConfig(){
  const j=await api('/api/config/app'); appCfg=j.data||{};
  const b=appCfg.btn||{}, ip=appCfg.ip_database||{}, n=appCfg.network||{};
  el('cfg_btn_enabled').checked=!!b.enabled; el('cfg_btn_appid').value=b.app_id||'';
  el('cfg_btn_secret').value=b.app_secret||''; el('cfg_btn_url').value=b.config_url||''; el('cfg_btn_submit').checked=b.submit!==false;
  el('cfg_ip_account').value=ip.account_id||''; el('cfg_ip_license').value=ip.license_key||''; el('cfg_ip_auto').checked=ip.auto_update!==false;
  el('cfg_proxy').value=n.proxy||'';
}
async function saveAppConfig(which){
  const body={};
  if(which==='btn') body.btn={enabled:el('cfg_btn_enabled').checked,config_url:el('cfg_btn_url').value.trim(),submit:el('cfg_btn_submit').checked,app_id:el('cfg_btn_appid').value.trim(),app_secret:el('cfg_btn_secret').value};
  if(which==='ip') body.ip_database={account_id:el('cfg_ip_account').value.trim(),license_key:el('cfg_ip_license').value,auto_update:el('cfg_ip_auto').checked};
  if(which==='net') body.network={proxy:el('cfg_proxy').value.trim()};
  el('cfgMsg').textContent='保存中…';
  const j=await api('/api/config/app',{method:'PUT',body:JSON.stringify(body)});
  if(j.ok){ toast('已保存并生效'); el('cfgMsg').innerHTML='<span class="ok">✓ 已保存,'+ (j.data&&j.data.modules!=null?j.data.modules+' 个模块已重建':'已生效') +'</span>'; }
  else el('cfgMsg').innerHTML='<span class="bad">✗ '+esc(j.error)+'</span>';
}
async function updateGeoip(){
  el('geoipStat').textContent='下载中…';
  const j=await api('/api/geoip/update',{method:'POST'});
  if(j.ok){ el('geoipStat').textContent = j.data.loaded?('✓ 已加载'+(j.data.changed?'(已更新)':'')):'✗ 未能加载'; toast('GeoIP 更新完成'); }
  else { el('geoipStat').textContent='✗ 失败'; toast(j.error||'失败',true); }
}
```

- [ ] **Step 3: 编译 + 验证**

Run: `cargo build -p pbh-server`
浏览器进「设置」:三块表单加载出当前值;改代理保存 → toast「已保存并生效」;改 BTN enabled 保存 → 日志显示 BTN 启停;点「立即更新 GeoIP」→ 状态变化。

- [ ] **Step 4: 提交**

```bash
git add crates/pbh-web/assets/index.html
git commit -m "feat(ui): 设置页(BTN/GeoIP/代理,热生效)"
```

---

### 任务 13:下载器并入仪表盘(模态增删改)

**Files:**
- Modify: `crates/pbh-web/assets/index.html`(删 `page-downloaders`,仪表盘加按钮 + 模态 + JS)

- [ ] **Step 1: 删除独立下载器页**

删除 `index.html:187-217` 的整个 `<section ... id="page-downloaders">...</section>`。

- [ ] **Step 2: 仪表盘下载器区加"添加"按钮**

把 `index.html:147-150`(下载器状态 card)的 `<h3>` 行改为带按钮的标题行:

```html
    <div class="card">
      <div class="row" style="margin-bottom:10px"><h3 style="margin:0">📥 下载器状态</h3><span class="sp"></span>
        <button class="sm" onclick="openDlModal()">+ 添加下载器</button></div>
      <div class="dlgrid" id="dlCards"></div>
    </div>
```

- [ ] **Step 3: 添加模态 DOM(放 `</main>` 之后、`toast` 之前)**

```html
<div id="dlModal" class="hidden" style="position:fixed;inset:0;z-index:60;background:rgba(0,0,0,.45);display:flex;align-items:center;justify-content:center">
  <div class="card" style="width:min(560px,92vw);max-height:90vh;overflow:auto">
    <div class="row" style="margin-bottom:10px"><h3 id="dlModalTitle" style="margin:0">➕ 添加下载器</h3><span class="sp"></span><button class="ghost sm" onclick="closeDlModal()">✕</button></div>
    <div class="grid">
      <input id="d_name" placeholder="名称">
      <select id="d_type"><option value="qbittorrent">qBittorrent</option><option value="qbittorrentee">qBittorrentEE</option></select>
      <input id="d_endpoint" placeholder="http://127.0.0.1:8080">
      <input id="d_user" placeholder="用户名" autocomplete="off">
      <input id="d_pass" type="password" placeholder="密码" autocomplete="new-password">
      <input id="d_id" placeholder="id(留空=自动生成)">
    </div>
    <div class="row" style="margin-top:11px">
      <label class="row" style="gap:5px"><input type="checkbox" id="d_inc"> 增量封禁</label>
      <label class="row" style="gap:5px"><input type="checkbox" id="d_shadow"> EE 影子封禁</label>
      <label class="row" style="gap:5px"><input type="checkbox" id="d_verify" checked> 校验 TLS</label>
      <label class="row" style="gap:5px"><input type="checkbox" id="d_ignpriv"> 排除私有种子</label>
    </div>
    <div class="row" style="margin-top:12px"><span id="dlMsg" class="mut"></span><span class="sp"></span>
      <button class="ghost" onclick="testDl()">测试连接</button>
      <button onclick="saveDl()">保存</button></div>
  </div>
</div>
```

- [ ] **Step 4: JS — 模态控制 + 复用既有增删改**

新增并改造(保留原 `editDl/resetDlForm/dlForm/saveDl/delDl/testDl` 逻辑,改为操作模态;`loadDownloaders` 不再需要,数据来自 dashboard 的 `downloader_list`):

```js
let dlConfigs=[];
function openDlModal(){ resetDlForm(); el('dlModalTitle').textContent='➕ 添加下载器'; el('dlModal').classList.remove('hidden'); }
function closeDlModal(){ el('dlModal').classList.add('hidden'); }
function editDl(id){
  const c=dlConfigs.find(x=>x.id===id); if(!c) return;
  el('d_id').value=c.id; el('d_name').value=c.name||''; el('d_type').value=c.type; el('d_endpoint').value=c.endpoint||'';
  el('d_user').value=c.username||''; el('d_pass').value=c.password||''; el('d_inc').checked=!!c['increment-ban'];
  el('d_shadow').checked=!!c['use-shadow-ban']; el('d_verify').checked=c['verify-ssl']!==false; el('d_ignpriv').checked=!!c['ignore-private'];
  el('dlModalTitle').textContent='✏️ 编辑 — '+(c.name||c.id); el('dlModal').classList.remove('hidden');
}
function resetDlForm(){ ['d_id','d_name','d_endpoint','d_user','d_pass'].forEach(i=>el(i).value=''); el('d_type').value='qbittorrent';
  el('d_inc').checked=false; el('d_shadow').checked=false; el('d_verify').checked=true; el('d_ignpriv').checked=false; el('dlMsg').textContent=''; }
function dlForm(){ return { id:el('d_id').value.trim(), type:el('d_type').value, name:el('d_name').value.trim(),
  endpoint:el('d_endpoint').value.trim(), username:el('d_user').value, password:el('d_pass').value,
  'increment-ban':el('d_inc').checked, 'use-shadow-ban':el('d_shadow').checked,
  'verify-ssl':el('d_verify').checked, 'ignore-private':el('d_ignpriv').checked }; }
async function saveDl(){
  const f=dlForm(); if(!f.endpoint){ el('dlMsg').innerHTML='<span class="bad">请填写端点</span>'; return; }
  const j=await api('/api/downloaders',{method:'PUT',body:JSON.stringify(f)});
  if(j.ok){ toast('已保存'); closeDlModal(); refreshDashboard(); } else el('dlMsg').innerHTML='<span class="bad">错误:'+esc(j.error)+'</span>';
}
async function delDl(id){ if(!confirm('删除该下载器?'))return; await api('/api/downloaders/'+id,{method:'DELETE'}); toast('已删除'); refreshDashboard(); }
async function testDl(){ el('dlMsg').textContent='测试中…';
  const j=await api('/api/downloaders/test',{method:'POST',body:JSON.stringify(dlForm())});
  el('dlMsg').innerHTML = j.data&&j.data.success ? '<span class="ok">✓ 连接成功</span>' : '<span class="bad">✗ '+esc((j.data&&j.data.message)||j.error||'')+'</span>'; }
```

`refreshDashboard` 内构建 `dlCards` 时:把 `list` 存入 `dlConfigs`(用于编辑),并把"编辑"按钮 `onclick` 指向 `editDl`(已是),删除指向 `delDl`。在 `refreshDashboard` 拿到 `list` 后加 `dlConfigs=list;`,并把卡片里 `gotoEdit('${c.id}')` 改为 `editDl('${c.id}')`。
删除 `gotoEdit`、`loadDownloaders` 函数及 `route()` 里对它的调用。

- [ ] **Step 5: 编译 + 验证**

Run: `cargo build -p pbh-server`
仪表盘:点「+ 添加下载器」弹模态→填写→测试连接→保存→卡片出现;点卡片「编辑」改名保存;「删除」生效。导航无「下载器」项。

- [ ] **Step 6: 提交**

```bash
git add crates/pbh-web/assets/index.html
git commit -m "feat(ui): 下载器并入仪表盘(模态增删改),移除独立页"
```

---

### 任务 14:规则配置图形化(全部模块 + YAML 回退)

**Files:**
- Create: `crates/pbh-web/assets/js-yaml.min.js`(内嵌 js-yaml,MIT)
- Modify: `crates/pbh-web/assets/index.html`(规则页重写 + 引入 yaml 库)

- [ ] **Step 1: 放入 js-yaml 单文件**

获取 js-yaml 4.x 浏览器版(MIT)单文件,保存为 `crates/pbh-web/assets/js-yaml.min.js`。
来源:`https://cdn.jsdelivr.net/npm/js-yaml@4.1.0/dist/js-yaml.min.js`(实现时本地保存该文件内容,不在运行时引用 CDN)。
在 `index.html` `<script>` 主块之前加内联引入:

```html
<script>/* js-yaml 4.1.0 (MIT) 内嵌 */</script>
```
实现方式:把文件内容直接粘进一个 `<script>...</script>`,或在 `routes.rs` 增加 `/js-yaml.min.js` 静态路由用 `include_str!`。
推荐后者更干净:`routes.rs` 加 `.route("/js-yaml.min.js", get(js_yaml))`,
```rust
async fn js_yaml() -> Response {
    ([(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
     include_str!("../assets/js-yaml.min.js")).into_response()
}
```
并在 `index.html` `<head>` 加 `<script src="/js-yaml.min.js"></script>`。

- [ ] **Step 2: 规则页 DOM 重写**

把 `page-rules`(`index.html:219-250`)的 `profile.yml` 卡片替换为图形化容器 + 折叠 YAML:

```html
  <section class="page" id="page-rules">
    <h1 class="ptitle">规则配置</h1>
    <p class="pdesc">图形化编辑封禁规则。保存后<b>规则模块即时生效</b>(check-interval / 全局 ban-duration 改动需重启)。</p>

    <div class="card" style="margin-bottom:16px">
      <h3>⚙️ 全局</h3>
      <div class="grid">
        <label>检查间隔(秒)<span class="mut" title="每轮 ban-wave 间隔,改动需重启">ⓘ</span><input id="g_interval" type="number" min="1"></label>
        <label>默认封禁时长(分钟)<input id="g_bandur" type="number" min="1"></label>
      </div>
      <label style="display:block;margin-top:10px">忽略来源地址(CIDR,每行一个)<textarea id="g_ignore" rows="3" placeholder="10.0.0.0/8"></textarea></label>
    </div>

    <div id="ruleCards"></div>

    <div class="card" style="margin-top:16px">
      <div class="row"><span class="sp"></span><button class="ghost" onclick="loadProfile()">重新载入</button><button onclick="saveRules()">保存并生效</button></div>
      <div id="ruleMsg" class="mut" style="margin-top:9px"></div>
    </div>

    <details class="card" style="margin-top:16px">
      <summary style="cursor:pointer;font-weight:600">🔧 高级:直接编辑 YAML</summary>
      <textarea id="profileYaml" rows="20" spellcheck="false" style="margin-top:10px"></textarea>
      <div class="row" style="margin-top:9px"><span class="sp"></span><button class="ghost" onclick="saveProfileRaw()">保存原始 YAML</button></div>
    </details>

    <div class="card" style="margin-top:16px">
      <h3>🌐 IP 黑名单订阅</h3>
      <table><thead><tr><th>规则ID</th><th>名称</th><th>条数</th><th>最后更新</th><th>状态</th><th></th></tr></thead><tbody id="subBody"></tbody></table>
      <h2 style="font-size:14px;margin:16px 0 8px">添加 / 编辑订阅</h2>
      <div class="grid">
        <input id="s_id" placeholder="规则ID(不含点,如 all-in-one)">
        <input id="s_name" placeholder="显示名称">
        <input id="s_url" placeholder="订阅 URL(https://...)" style="grid-column:1/3">
      </div>
      <div class="row" style="margin-top:8px"><label class="row" style="gap:5px"><input type="checkbox" id="s_enabled" checked> 启用</label>
        <span class="sp"></span><button class="ghost" onclick="resetSubForm()">清空</button><button onclick="saveSub()">保存订阅(自动下载)</button></div>
      <div id="subMsg" class="mut" style="margin-top:8px"></div>
    </div>
  </section>
```

(订阅管理 JS `loadSubs/editSub/resetSubForm/saveSub/delSub` 保留不变。)

- [ ] **Step 3: 模块定义表 + 渲染**

在 `<script>` 内新增模块描述表与渲染。字段类型支持:`toggle`(布尔)、`num`(数字,分钟→ms 由 scale 控制)、`lines`(多行→数组)。

```js
// 模块图形化定义。key=profile.module.<key>;fields 描述可编辑字段。
const RULE_MODULES = [
  { key:'progress-cheat-blocker', name:'进度作弊检测 (PCB)', defOn:true,
    desc:'反吸血核心:对比你给 peer 的实际上传量与它自报进度,识破谎报进度/过量下载/进度回退。',
    fields:[
      {k:'ban-duration', t:'num', scale:60000, label:'封禁时长(分钟,0=用全局)'},
      {k:'max-difference', t:'num', label:'最大进度差异(0-1,如 0.1)', step:'0.01'},
    ]},
  { key:'peer-id-blacklist', name:'PeerID 黑名单', defOn:true,
    desc:'按 PeerID 前缀封禁已知吸血/离线下载客户端。每行一条,格式 method:content(如 STARTS_WITH:-XL)。',
    fields:[{k:'ban-duration', t:'num', scale:60000, label:'封禁时长(分钟)'},
            {k:'__peerid', t:'lines', label:'规则(每行 method:content)', map:'banned-peer-id'}]},
  { key:'client-name-blacklist', name:'客户端名黑名单', defOn:true,
    desc:'按客户端名封禁。每行一条 method:content(如 CONTAINS:Xunlei)。',
    fields:[{k:'ban-duration', t:'num', scale:60000, label:'封禁时长(分钟)'},
            {k:'__cname', t:'lines', label:'规则(每行 method:content)', map:'banned-client-name'}]},
  { key:'ip-address-blocker', name:'IP 黑名单', defOn:false,
    desc:'按 IP / 端口 / ASN / 地区 / 城市 / 网络类型封禁(地区/ASN 需 GeoIP 库)。每行一个 IP/CIDR。',
    fields:[{k:'ban-duration', t:'num', scale:60000, label:'封禁时长(分钟)'},
            {k:'__ips', t:'lines', label:'IP/CIDR(每行一个)', map:'ips'}]},
  { key:'multi-dialing-blocker', name:'多拨号封禁', defOn:false,
    desc:'检测同一 /24(或自定义)网段下多个相同 torrent 的 peer(多拨叠加上传)。',
    fields:[{k:'ban-duration', t:'num', scale:60000, label:'封禁时长(分钟)'},
            {k:'subnet-mask-length', t:'num', label:'IPv4 子网掩码长度(如 24)'},
            {k:'tolerate-num', t:'num', label:'容忍数量'}]},
  { key:'idle-connection-dos-protection', name:'空闲连接 DoS 防护', defOn:false,
    desc:'封禁长期连接但不传输数据的空闲 peer(疑似 DoS)。',
    fields:[{k:'ban-duration', t:'num', scale:60000, label:'封禁时长(分钟)'}]},
  { key:'ptr-blacklist', name:'PTR 反向 DNS 黑名单', defOn:false,
    desc:'对 peer IP 做反向 DNS,命中关键字则封禁。每行一个关键字。',
    fields:[{k:'ban-duration', t:'num', scale:60000, label:'封禁时长(分钟)'},
            {k:'__ptr', t:'lines', label:'关键字(每行一个)', map:'rules'}]},
  { key:'auto-range-ban', name:'自动段封禁', defOn:false,
    desc:'某 peer 被封后,自动连带封禁其所在网段(扩大封禁面,谨慎)。',
    fields:[{k:'ipv4-prefix-length', t:'num', label:'IPv4 段前缀(如 24)'},
            {k:'ipv6-prefix-length', t:'num', label:'IPv6 段前缀(如 60)'}]},
];

let profileObj={}; // 当前 profile 解析结果(保留未知键)
function renderRuleCards(){
  const mod = profileObj.module || (profileObj.module={});
  el('ruleCards').innerHTML = RULE_MODULES.map(m=>{
    const sec = mod[m.key]||{};
    const on = sec.enabled!==undefined ? !!sec.enabled : m.defOn;
    const fields = m.fields.map(f=>{
      const id='rf_'+m.key+'_'+f.k;
      if(f.t==='toggle') return `<label class="row" style="gap:5px"><input type="checkbox" id="${id}" ${sec[f.k]?'checked':''}> ${f.label}</label>`;
      if(f.t==='lines'){ const arr=sec[f.map]||[]; const txt=Array.isArray(arr)?arr.map(linesFmt).join('\n'):''; return `<label style="display:block">${f.label}<textarea id="${id}" rows="4">${esc(txt)}</textarea></label>`; }
      // num
      let v=sec[f.k]; if(v!=null&&f.scale) v=v/f.scale;
      return `<label>${f.label}<input id="${id}" type="number" ${f.step?('step='+f.step):''} value="${v!=null?v:''}"></label>`;
    }).join('');
    return `<div class="card" style="margin-bottom:12px">
      <div class="row"><h3 style="margin:0">${esc(m.name)}</h3><span class="sp"></span>
        <label class="row" style="gap:6px"><input type="checkbox" id="ren_${m.key}" ${on?'checked':''}> 启用</label></div>
      <p class="mut" style="margin:6px 0 10px">${esc(m.desc)}</p>
      <div class="grid">${fields}</div></div>`;
  }).join('');
}
// banned-peer-id 等是对象数组 {method,content};行内用 method:content 表示。
function linesFmt(x){ return (x&&typeof x==='object')? ((x.method||'')+':'+(x.content||'')) : String(x); }
function linesParse(line, isObj){ if(!isObj) return line.trim();
  const i=line.indexOf(':'); return i<0?{method:'STARTS_WITH',content:line.trim()}:{method:line.slice(0,i).trim(),content:line.slice(i+1).trim()}; }
```

- [ ] **Step 4: 载入与保存(图形化 ↔ profileObj ↔ YAML)**

```js
async function loadProfile(){
  const j=await api('/api/config/profile');
  if(j.ok){ el('profileYaml').value=j.data.yaml; try{ profileObj=jsyaml.load(j.data.yaml)||{}; }catch(e){ profileObj={}; }
    if(!profileObj.module) profileObj.module={};
    el('g_interval').value=(profileObj['check-interval']||5000)/1000;
    el('g_bandur').value=(profileObj['ban-duration']||1209600000)/60000;
    el('g_ignore').value=(profileObj['ignore-peers-from-addresses']||[]).join('\n');
    renderRuleCards(); el('ruleMsg').textContent=''; }
  loadSubs();
}
function collectRules(){
  profileObj['check-interval']=Math.round((parseFloat(el('g_interval').value)||5)*1000);
  profileObj['ban-duration']=Math.round((parseFloat(el('g_bandur').value)||0)*60000)||1209600000;
  profileObj['ignore-peers-from-addresses']=el('g_ignore').value.split('\n').map(s=>s.trim()).filter(Boolean);
  const mod=profileObj.module||(profileObj.module={});
  RULE_MODULES.forEach(m=>{
    const sec=mod[m.key]||(mod[m.key]={});
    sec.enabled=el('ren_'+m.key).checked;
    m.fields.forEach(f=>{
      const id='rf_'+m.key+'_'+f.k; const node=el(id); if(!node) return;
      if(f.t==='toggle'){ sec[f.k]=node.checked; }
      else if(f.t==='lines'){ const isObj = (f.map==='banned-peer-id'||f.map==='banned-client-name');
        sec[f.map]=node.value.split('\n').map(s=>s.trim()).filter(Boolean).map(l=>linesParse(l,isObj)); }
      else { const v=node.value.trim(); if(v===''){ delete sec[f.k]; } else { let num=parseFloat(v); if(f.scale) num=Math.round(num*f.scale); sec[f.k]=num; } }
    });
  });
}
async function saveRules(){
  collectRules();
  const yaml=jsyaml.dump(profileObj,{lineWidth:-1});
  el('profileYaml').value=yaml;
  await putProfileYaml(yaml);
}
async function saveProfileRaw(){ await putProfileYaml(el('profileYaml').value); }
async function putProfileYaml(yaml){
  el('ruleMsg').textContent='保存中…';
  const j=await api('/api/config/profile',{method:'PUT',body:JSON.stringify({yaml})});
  if(j.ok){ el('ruleMsg').innerHTML='<span class="ok">✓ 已保存,'+j.data.modules+' 个规则模块已生效</span>'; toast('规则已保存生效'); try{profileObj=jsyaml.load(yaml)||{};}catch(e){} refreshDashboard(); }
  else el('ruleMsg').innerHTML='<span class="bad">✗ '+esc(j.error)+'</span>';
}
```

删除旧的 `saveProfile`(被 `saveRules`/`saveProfileRaw` 取代);`route()` 里 `if(h==='rules') loadProfile();` 保留。

- [ ] **Step 5: 编译 + 验证**

Run: `cargo build -p pbh-server`
规则页:全局项与 9 个模块卡片渲染出当前值;改某模块开关/字段→保存并生效→toast;展开「高级 YAML」可见同步后的 YAML;订阅区照常工作。
重点验证:`peer-id-blacklist` 的 `banned-peer-id` 往返(method:content ↔ 对象数组)不丢数据。

- [ ] **Step 6: 提交**

```bash
git add crates/pbh-web/assets/index.html crates/pbh-web/assets/js-yaml.min.js crates/pbh-web/src/routes.rs
git commit -m "feat(ui): 规则配置图形化(全部模块+中文解释)+ YAML 高级回退"
```

---

## 阶段 6:Release 改名

### 任务 15:Release 包名 `pbh` → `pbh-rust`

**Files:**
- Modify: `.github/workflows/release.yml`、`build.sh`、`README.md`

- [ ] **Step 1: release.yml unix 打包改名**

把 `Package (unix)` 步骤(`release.yml:50-61`)中:
- `name="pbh-${ver}-${{ matrix.target }}"` → `name="pbh-rust-${ver}-${{ matrix.target }}"`
- `cp "target/${{ matrix.target }}/release/pbh" "dist/$name/"` →
  `cp "target/${{ matrix.target }}/release/pbh" "dist/$name/pbh-rust"`

- [ ] **Step 2: release.yml windows 打包改名**

`Package (windows)`(`release.yml:63-74`):
- `$name = "pbh-$ver-..."` → `$name = "pbh-rust-$ver-..."`
- `Copy-Item "...release/pbh.exe" "dist/$name/"` → `Copy-Item "...release/pbh.exe" "dist/$name/pbh-rust.exe"`

- [ ] **Step 3: build.sh package 改名**

读 `build.sh` 的 `package` 分支,把产物目录/压缩包前缀与拷入的二进制名改为 `pbh-rust`(与 release.yml 一致)。

- [ ] **Step 4: README 同步**

`README.md` 中下载/解压/运行示例里的可执行名 `pbh` → `pbh-rust`(运行命令、产物说明处);
"从源码构建"里 `target/release/pbh` 保持(cargo bin 名不变),仅打包产物说明改 `pbh-rust`。

- [ ] **Step 5: 本地验证打包**

Run: `./build.sh package`
Expected: 产出 `dist/pbh-rust-<ver>-<os>-<arch>.tar.gz`,内含可执行 `pbh-rust`。

- [ ] **Step 6: 提交**

```bash
git add .github/workflows/release.yml build.sh README.md
git commit -m "build: Release 包名与可执行改为 pbh-rust"
```

---

## 收尾

- [ ] 全量回归:`cargo build --workspace && cargo test --workspace && cargo clippy --workspace`
- [ ] 端到端手动验证清单(逐条勾):
  - [ ] 标题 `PeerBanHelper-Rust vX.Y.Z`;GitHub/主题/退出图标均显示;退出图标不再缺字。
  - [ ] 设置页:BTN 开关/凭证保存→日志 BTN 启停;代理保存→不可达直连 warn;均无需重启。
  - [ ] GeoIP:清空 `data/geoip` 启动→后台下载;「立即更新 GeoIP」热加载;封禁列表地理信息出现。
  - [ ] 规则页:9 模块图形化往返正确;高级 YAML 同步;订阅增删改正常。
  - [ ] 仪表盘:下载器模态增/改/删 + 测试连接;导航无「下载器」页。
  - [ ] 检查更新:有新版本时头部徽标显示并跳转。
  - [ ] `./build.sh package` 产物名 `pbh-rust`。
- [ ] 更新 `memory/STATUS.md` 与 `README.md`「网页界面」表(去掉独立下载器页、加设置页)。
- [ ] 合并分支 `feat/webui-overhaul` → `main`(或开 PR)。

## 自检备注(写计划时已核对)

- 阶段顺序保证每步可编译:任务 3 让 BTN/订阅客户端接受 proxy 形参但调用方暂传 `""`;任务 8 才接通真实 proxy 与热启停;任务 4 的 handle 重构在 GeoIP 下载(任务 5/7)之前完成。
- 类型一致:`GeoIpHandle`(query/install/is_loaded/from_provider/new_empty)、`BtnManager`(apply/stop/current_state)、`version_newer(current,latest)` 在引用处签名一致。
- spec 全部 11 项均有对应任务(1+:代理;2/3 接入;4-7 GeoIP;8/9 BTN+config;10 更新;11 头部品牌/版本/GitHub/退出图标;12 设置/BTN;13 下载器;14 规则;15 改名)。
