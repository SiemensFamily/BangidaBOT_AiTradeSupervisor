use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a query string using HMAC-SHA256 for Binance API authentication.
pub fn sign(secret: &str, message: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Generate the current timestamp in milliseconds for Binance requests.
pub fn timestamp_ms() -> u64 {
    chrono::Utc::now().timestamp_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_produces_hex_string() {
        let sig = sign("my_secret", "symbol=BTCUSDT&timestamp=1234567890");
        assert_eq!(sig.len(), 64); // SHA256 hex is 64 chars
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sign_is_deterministic() {
        let a = sign("key", "data");
        let b = sign("key", "data");
        assert_eq!(a, b);
    }
}
