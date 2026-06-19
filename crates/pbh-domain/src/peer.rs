//! Peer 与 PeerFlag。对应 Java `bittorrent/peer/{Peer,PeerFlag}.java`、`wrapper/PeerAddress.java`。

use std::net::IpAddr;

/// peer 网络地址。`raw_ip` 是下载器返回的原始 `ip:port` 键，封禁回传时必须用它。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerAddress {
    pub ip: IpAddr,
    pub port: u16,
    /// 下载器（如 qB `/sync/torrentPeers` 的 map 键）给出的原始字符串，形如 `1.2.3.4:6881`。
    pub raw_ip: String,
}

impl PeerAddress {
    pub fn new(ip: IpAddr, port: u16) -> Self {
        let raw_ip = match ip {
            IpAddr::V4(_) => format!("{ip}:{port}"),
            IpAddr::V6(_) => format!("[{ip}]:{port}"),
        };
        PeerAddress { ip, port, raw_ip }
    }

    /// 缓存键 `ip:port`（对应 Java `Peer.getCacheKey()`）。
    pub fn cache_key(&self) -> String {
        self.raw_ip.clone()
    }
}

/// 一个连接中的 peer 的快照。对应 Java `Peer`。
///
/// 注意：`uploaded`/`downloaded` 为 `-1` 表示下载器无法报告（保持 Java 语义）。
#[derive(Debug, Clone)]
pub struct Peer {
    pub address: PeerAddress,
    /// BT PeerID，如 `-qB4250-`。
    pub peer_id: Option<String>,
    /// 客户端名，如 `qBittorrent/4.2.5`。
    pub client_name: Option<String>,
    pub download_speed: i64,
    pub upload_speed: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    /// peer 自报进度 0.0–1.0。
    pub progress: f64,
    pub flags: Option<PeerFlag>,
}

impl Peer {
    /// 是否处于握手阶段（上下行速度均 ≤ 0）。对应 Java `Peer.isHandshaking()`。
    pub fn is_handshaking(&self) -> bool {
        self.upload_speed <= 0 && self.download_speed <= 0
    }
}

/// libtorrent peer flag 位集。对应 Java `bittorrent/peer/PeerFlag.java`。
///
/// 骨架阶段只解析模块实际用到的 `interesting`(d/D) 与 `remote_interested`(u/U)；
/// M1 补全全部 21 个 peer 位 + 6 个 source 位与 `to_string()` 往返。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerFlag {
    pub raw: String,
    pub interesting: bool,
    pub remote_interested: bool,
}

impl PeerFlag {
    /// 解析 libtorrent flag 串，如 `"d UD I E P"`。
    pub fn parse(raw: &str) -> Self {
        let mut f = PeerFlag {
            raw: raw.to_string(),
            ..Default::default()
        };
        for ch in raw.chars() {
            match ch {
                'd' | 'D' => f.interesting = true,
                'u' | 'U' => f.remote_interested = true,
                _ => {}
            }
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn handshaking_detection() {
        let p = Peer {
            address: PeerAddress::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6881),
            peer_id: None,
            client_name: None,
            download_speed: 0,
            upload_speed: 0,
            downloaded: 0,
            uploaded: 0,
            progress: 0.0,
            flags: None,
        };
        assert!(p.is_handshaking());
    }

    #[test]
    fn peer_flag_parses_interest_bits() {
        let f = PeerFlag::parse("d UD I E P");
        assert!(f.interesting);
        assert!(f.remote_interested);
    }

    #[test]
    fn v6_raw_ip_is_bracketed() {
        let a = PeerAddress::new("::1".parse().unwrap(), 6881);
        assert_eq!(a.raw_ip, "[::1]:6881");
    }
}
