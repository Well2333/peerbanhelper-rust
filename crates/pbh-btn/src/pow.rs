//! Proof-of-Work captcha 求解。对应上游 `util/pow/PoWClient.java`。
//!
//! 服务端给挑战字节 + 难度位数;客户端找一个 nonce 使 `SHA256(challenge || nonce)` 前导有
//! `difficulty_bits` 个 0 比特。结果（nonce 字节的 base64）放 `X-BTN-PowSolution` 头,
//! 挑战 id 放 `X-BTN-PowID`。

use sha2::{Digest, Sha256};

/// PoW 挑战（从 `GET {pow_endpoint}?type=<ability>` 解析）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PowChallenge {
    pub id: String,
    /// base64 编码的挑战字节。
    pub challenge_base64: String,
    pub difficulty_bits: u32,
}

/// 求解：返回满足前导零比特的 8 字节 nonce（big-endian）。失败（超迭代上限）返回 None。
pub fn solve(challenge: &[u8], difficulty_bits: u32) -> Option<[u8; 8]> {
    // 难度 0 即任意 nonce 都满足。
    const MAX_ITERS: u64 = 1 << 32;
    let mut nonce: u64 = 0;
    while nonce < MAX_ITERS {
        let mut h = Sha256::new();
        h.update(challenge);
        h.update(nonce.to_be_bytes());
        if has_leading_zero_bits(&h.finalize(), difficulty_bits) {
            return Some(nonce.to_be_bytes());
        }
        nonce += 1;
    }
    None
}

/// hash 前导是否有 `bits` 个 0 比特。
pub fn has_leading_zero_bits(hash: &[u8], bits: u32) -> bool {
    let full = (bits / 8) as usize;
    let rem = bits % 8;
    if full > hash.len() {
        return false;
    }
    if hash[..full].iter().any(|&b| b != 0) {
        return false;
    }
    if rem > 0 {
        if full >= hash.len() {
            return false;
        }
        let mask = 0xFFu8 << (8 - rem);
        if hash[full] & mask != 0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_zero_bits_check() {
        assert!(has_leading_zero_bits(&[0x00, 0x0f], 8));
        assert!(has_leading_zero_bits(&[0x00, 0x0f], 12));
        assert!(!has_leading_zero_bits(&[0x00, 0x1f], 12)); // 0x1f 高位非 0
        assert!(has_leading_zero_bits(&[0xff], 0)); // 0 难度恒真
    }

    #[test]
    fn solves_low_difficulty() {
        let challenge = b"btn-pow-test-challenge";
        let bits = 12;
        let nonce = solve(challenge, bits).expect("应能求解");
        // 验证结果确实满足。
        let mut h = Sha256::new();
        h.update(challenge);
        h.update(nonce);
        assert!(has_leading_zero_bits(&h.finalize(), bits));
    }
}
