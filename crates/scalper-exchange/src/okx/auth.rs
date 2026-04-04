use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a request for OKX API.
/// OKX signature = Base64(HMAC_SHA256(timestamp + method + requestPath + body))
pub fn sign(secret: &str, timestamp: &str, method: &str, path: &str, body: &str) -> String {
    let message = format!("{timestamp}{method}{path}{body}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key");
    mac.update(message.as_bytes());
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// ISO 8601 timestamp for OKX requests.
pub fn timestamp_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn timestamp_ms() -> u64 {
    chrono::Utc::now().timestamp_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_produces_base64() {
        let sig = sign("secret", "2024-01-01T00:00:00.000Z", "GET", "/api/v5/account/balance", "");
        // Base64 encoded HMAC-SHA256 is 44 chars
        assert_eq!(sig.len(), 44);
    }
}
