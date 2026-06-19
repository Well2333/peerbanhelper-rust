//! pbh-geoip —— GeoIP 查询（MaxMind City/ASN）。对应 Java `util/ipdb/**`。
//!
//! **可选注入**能力（守则第 9 条）：拿不到 mmdb 文件时降级——`GeoIpProvider` 不存在，
//! IPBlackList 的 ASN/region/city/net-type 检查全部跳过（pass），ip/port 仍生效。
//!
//! 当前实现标准 MaxMind City + ASN 读取。GeoCN（中国网络类型/行政区划）未移植——
//! `net_type`/`cn_*` 字段保留但恒为 None（需 GeoCN 数据库,本环境无）。

use std::net::IpAddr;
use std::path::Path;

use serde::Serialize;

/// GeoIP 查询结果（前端契约）。对应 Java `IPGeoData`。
#[derive(Debug, Clone, Default, Serialize)]
pub struct IpGeoData {
    pub country_iso: Option<String>,
    pub country_name: Option<String>,
    pub city_name: Option<String>,
    pub asn: Option<u32>,
    pub as_organization: Option<String>,
    /// 中国网络类型（来自 GeoCN，本实现恒 None）。
    pub net_type: Option<String>,
    pub cn_province: Option<String>,
    pub cn_city: Option<String>,
}

/// GeoIP 查询抽象（可选注入）。
pub trait GeoIpProvider: Send + Sync {
    fn query(&self, ip: IpAddr) -> Option<IpGeoData>;
}

/// 基于 MaxMind mmdb 的查询实现（City + ASN，二者任一可缺）。
pub struct MaxmindProvider {
    city: Option<maxminddb::Reader<Vec<u8>>>,
    asn: Option<maxminddb::Reader<Vec<u8>>>,
}

impl MaxmindProvider {
    /// 尝试加载 City / ASN mmdb。两者都加载失败则返回 None（降级）。
    pub fn load(city_path: Option<&Path>, asn_path: Option<&Path>) -> Option<Self> {
        let city = city_path.and_then(|p| open(p, "City"));
        let asn = asn_path.and_then(|p| open(p, "ASN"));
        if city.is_none() && asn.is_none() {
            return None;
        }
        Some(MaxmindProvider { city, asn })
    }

    /// 从一个目录约定加载：`<dir>/GeoLite2-City.mmdb` + `GeoLite2-ASN.mmdb`（或 GeoIP2 同名）。
    pub fn load_from_dir(dir: &Path) -> Option<Self> {
        let find = |names: &[&str]| -> Option<std::path::PathBuf> {
            names.iter().map(|n| dir.join(n)).find(|p| p.exists())
        };
        let city = find(&["GeoLite2-City.mmdb", "GeoIP2-City.mmdb", "City.mmdb"]);
        let asn = find(&["GeoLite2-ASN.mmdb", "GeoIP2-ASN.mmdb", "ASN.mmdb"]);
        Self::load(city.as_deref(), asn.as_deref())
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
        Some(d)
    }
}
