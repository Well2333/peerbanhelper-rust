//! pbh-geoip —— GeoIP 查询（MaxMind City/ASN + GeoCN 叠加）。对应 Java `util/ipdb/**`。
//!
//! M6 实现：下载（三镜像、basic-auth、45 天更新判定）、xz 解压、原子替换、
//! GeoCN2/1 解析 + 行政区划 CSV trie、`IPGeoData`（**前端契约**）+ 叠加 + TW/HK/MO 命名特例、moka 缓存。
//!
//! 设计（守则第 9 条）：作为**可选注入**能力——拿不到 mmdb 时降级（IPBlackList 的 ASN/region 检查跳过）。
//! 骨架阶段给出 `IPGeoData` 形状占位（M6 加 serde，须与前端字段一致）。

/// GeoIP 查询结果（前端契约占位）。对应 Java `IPGeoData`。
#[derive(Debug, Clone, Default)]
pub struct IpGeoData {
    pub country_iso: Option<String>,
    pub country_name: Option<String>,
    pub city_name: Option<String>,
    pub asn: Option<u32>,
    pub as_organization: Option<String>,
    /// 中国网络类型（来自 GeoCN）。
    pub net_type: Option<String>,
    pub cn_province: Option<String>,
    pub cn_city: Option<String>,
}

/// GeoIP 查询抽象（可选注入）。
pub trait GeoIpProvider: Send + Sync {
    fn query(&self, ip: std::net::IpAddr) -> Option<IpGeoData>;
}
