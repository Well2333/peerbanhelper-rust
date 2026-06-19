//! 种子标识隐私哈希。对应上游 `InfoHashUtil.getHashedIdentifier`。
//!
//! BTN 从不上报原始 infohash:`SHA256(lower(hash) + crc32_salt)`。
//! salt = Guava `Hashing.crc32().hashString(lower).toString()`,即 CRC-32(IEEE) 的
//! **小端字节** 十六进制串（Guava HashCode 字节序）。

use sha2::{Digest, Sha256};

/// CRC-32（IEEE 802.3）。
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// 计算种子隐私标识。
pub fn hashed_identifier(info_hash: &str) -> String {
    let lower = info_hash.to_lowercase();
    let crc = crc32(lower.as_bytes());
    // Guava HashCode.toString(): 字节小端 hex。
    let salt: String = crc
        .to_le_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut h = Sha256::new();
    h.update(lower.as_bytes());
    h.update(salt.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vector() {
        // CRC-32("123456789") = 0xCBF43926。
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn hashed_identifier_stable_and_lowercased() {
        let a = hashed_identifier("ABCDEF0123456789ABCDEF0123456789ABCDEF01");
        let b = hashed_identifier("abcdef0123456789abcdef0123456789abcdef01");
        // 大小写无关（先 lowercase）。
        assert_eq!(a, b);
        // SHA-256 hex = 64 字符。
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
