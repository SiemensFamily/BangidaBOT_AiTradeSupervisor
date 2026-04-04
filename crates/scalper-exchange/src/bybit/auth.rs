use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a message using HMAC-SHA256 for Bybit V5 API.
/// Bybit signature = HMAC_SHA256(timestamp + api_key + recv_window + payload)
pub fn sign(secret: &str, timestamp: u64, api_key: &str, recv_window: u64, payload: &str) -> String {
    let message = format!("{timestamp}{api_key}{recv_window}{payload}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn timestamp_ms() -> u64 {
    chrono::Utc::now().timestamp_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_produces_valid_hex() {
        let sig = sign("secret", 1234567890, "api_key", 5000, "{}");
        assert_eq!(sig.len(), 64);
    }
}
