//! pbh-geoip —— GeoIP 查询（MaxMind City/ASN/GeoCN）。对应 Java `util/ipdb/**`。
//!
//! **可选注入**能力（守则第 9 条）：拿不到 mmdb 文件时降级——`GeoIpProvider` 不存在，
//! IPBlackList 的 ASN/region/city/net-type 检查全部跳过（pass），ip/port 仍生效。
//!
//! 当前实现标准 MaxMind City + ASN + GeoCN 读取。GeoCN 数据库（`GeoCN.mmdb`）提供
//! 中国网络类型/行政区划，映射到 `net_type`/`cn_province`/`cn_city` 字段。

pub mod download;

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

/// GeoIP 查询结果（前端契约）。对应 Java `IPGeoData`。
#[derive(Debug, Clone, Default, Serialize)]
pub struct IpGeoData {
    pub country_iso: Option<String>,
    pub country_name: Option<String>,
    pub city_name: Option<String>,
    pub asn: Option<u32>,
    pub as_organization: Option<String>,
    /// 中国网络类型（来自 GeoCN，需 `GeoCN.mmdb`）。
    pub net_type: Option<String>,
    pub cn_province: Option<String>,
    pub cn_city: Option<String>,
}

/// GeoIP 查询抽象（可选注入）。
pub trait GeoIpProvider: Send + Sync {
    fn query(&self, ip: IpAddr) -> Option<IpGeoData>;
}

/// GeoCN 记录（中国网络类型/行政区划）。字段均可缺。
#[derive(Debug, serde::Deserialize)]
pub struct GeoCnRecord {
    #[serde(default)]
    pub net: Option<String>,
    #[serde(default)]
    pub province: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
}

/// 基于 MaxMind mmdb 的查询实现（City + ASN + GeoCN，三者任一可缺）。
pub struct MaxmindProvider {
    city: Option<maxminddb::Reader<Vec<u8>>>,
    asn: Option<maxminddb::Reader<Vec<u8>>>,
    cn: Option<maxminddb::Reader<Vec<u8>>>,
}

impl MaxmindProvider {
    /// 尝试加载 City / ASN mmdb。两者都加载失败则返回 None（降级）。
    /// GeoCN 不通过此方法加载（cn 恒为 None）；请用 `load_from_dir` 同时加载 GeoCN。
    pub fn load(city_path: Option<&Path>, asn_path: Option<&Path>) -> Option<Self> {
        let city = city_path.and_then(|p| open(p, "City"));
        let asn = asn_path.and_then(|p| open(p, "ASN"));
        if city.is_none() && asn.is_none() {
            return None;
        }
        Some(MaxmindProvider { city, asn, cn: None })
    }

    /// 从一个目录约定加载：`<dir>/GeoLite2-City.mmdb` + `GeoLite2-ASN.mmdb`（或 GeoIP2 同名）+ `GeoCN.mmdb`。
    pub fn load_from_dir(dir: &Path) -> Option<Self> {
        let find = |names: &[&str]| -> Option<std::path::PathBuf> {
            names.iter().map(|n| dir.join(n)).find(|p| p.exists())
        };
        let city = find(&["GeoIP-City.mmdb", "GeoLite2-City.mmdb", "GeoIP2-City.mmdb", "City.mmdb"]);
        let asn = find(&["GeoIP-ASN.mmdb", "GeoLite2-ASN.mmdb", "GeoIP2-ASN.mmdb", "ASN.mmdb"]);
        let cn_path = find(&["GeoCN.mmdb"]);
        let city = city.as_deref().and_then(|p| open(p, "City"));
        let asn = asn.as_deref().and_then(|p| open(p, "ASN"));
        let cn = cn_path.as_deref().and_then(|p| open(p, "GeoCN"));
        if city.is_none() && asn.is_none() && cn.is_none() {
            return None;
        }
        Some(MaxmindProvider { city, asn, cn })
    }
}

fn open(path: &Path, label: &str) -> Option<maxminddb::Reader<Vec<u8>>> {
    match maxminddb::Reader::open_readfile(path) {
        Ok(r) => {
            tracing::info!("GeoIP {label} 库已加载: {}", path.display());
            Some(r)
        }
        Err(e) => {
            tracing::warn!("GeoIP {label} 库加载失败 ({}): {e}", path.display());
            None
        }
    }
}

impl GeoIpProvider for MaxmindProvider {
    fn query(&self, ip: IpAddr) -> Option<IpGeoData> {
        let mut d = IpGeoData::default();
        if let Some(c) = &self.city {
            if let Ok(city) = c.lookup::<maxminddb::geoip2::City>(ip) {
                d.country_iso = city
                    .country
                    .as_ref()
                    .and_then(|c| c.iso_code)
                    .map(|s| s.to_string());
                d.country_name = city
                    .country
                    .as_ref()
                    .and_then(|c| c.names.as_ref())
                    .and_then(|n| n.get("en"))
                    .map(|s| s.to_string());
                d.city_name = city
                    .city
                    .as_ref()
                    .and_then(|c| c.names.as_ref())
                    .and_then(|n| n.get("en"))
                    .map(|s| s.to_string());
            }
        }
        if let Some(a) = &self.asn {
            if let Ok(asn) = a.lookup::<maxminddb::geoip2::Asn>(ip) {
                d.asn = asn.autonomous_system_number;
                d.as_organization = asn.autonomous_system_organization.map(|s| s.to_string());
            }
        }
        if let Some(c) = &self.cn {
            if let Ok(rec) = c.lookup::<GeoCnRecord>(ip) {
                d.net_type = rec.net;
                d.cn_province = rec.province;
                d.cn_city = rec.city;
            }
        }
        Some(d)
    }
}

/// arc-swap 不直接支持 `dyn Trait`（需 Sized），用具体包装类型绕过。
struct ProviderBox(Arc<dyn GeoIpProvider>);

/// 可热替换的 GeoIP 句柄：后台下载完成后 `install` 新 provider，读取方立即生效。
#[derive(Clone)]
pub struct GeoIpHandle {
    inner: Arc<arc_swap::ArcSwapOption<ProviderBox>>,
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
        self.inner.store(Some(Arc::new(ProviderBox(p))));
    }
    pub fn is_loaded(&self) -> bool {
        self.inner.load().is_some()
    }
    pub fn query(&self, ip: std::net::IpAddr) -> Option<IpGeoData> {
        self.inner.load_full().and_then(|b| b.0.query(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_none_without_files() {
        // 不存在的 mmdb → 降级(None)。
        assert!(MaxmindProvider::load(
            Some(Path::new("/nonexistent/City.mmdb")),
            Some(Path::new("/nonexistent/ASN.mmdb")),
        )
        .is_none());
        // 空目录 → None。
        let dir = std::env::temp_dir().join("pbh-geoip-empty-test");
        let _ = std::fs::create_dir_all(&dir);
        assert!(MaxmindProvider::load_from_dir(&dir).is_none());
    }

    #[test]
    fn handle_starts_empty_and_installs() {
        let h = GeoIpHandle::new_empty();
        assert!(h.query("1.1.1.1".parse().unwrap()).is_none());
        assert!(!h.is_loaded());
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

    #[test]
    fn handle_clone_shares_state() {
        let h = GeoIpHandle::new_empty();
        let h2 = h.clone();
        struct Dummy;
        impl GeoIpProvider for Dummy {
            fn query(&self, _ip: std::net::IpAddr) -> Option<IpGeoData> { Some(IpGeoData::default()) }
        }
        h.install(std::sync::Arc::new(Dummy));
        assert!(h2.is_loaded()); // install on one clone visible on the other
    }

    #[test]
    fn geocn_record_deserializes() {
        // 模拟 GeoCN 记录的 JSON 形态映射到内部结构。
        let json = r#"{"net":"宽带","province":"上海","city":"上海"}"#;
        let r: GeoCnRecord = serde_json::from_str(json).unwrap();
        assert_eq!(r.net.as_deref(), Some("宽带"));
        assert_eq!(r.province.as_deref(), Some("上海"));
        assert_eq!(r.city.as_deref(), Some("上海"));
    }

    #[test]
    fn ipgeodata_serializes() {
        let d = IpGeoData {
            country_iso: Some("CN".into()),
            city_name: Some("Shanghai".into()),
            asn: Some(4134),
            ..Default::default()
        };
        let j = serde_json::to_string(&d).unwrap();
        assert!(j.contains("\"country_iso\":\"CN\""));
        assert!(j.contains("\"asn\":4134"));
    }
}
