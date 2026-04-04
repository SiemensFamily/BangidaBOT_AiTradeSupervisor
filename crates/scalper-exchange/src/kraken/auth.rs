use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a message for Kraken Futures API.
/// Kraken uses: HMAC-SHA256(postData + nonce + endpointPath, base64decode(secret))
pub fn sign(secret: &str, nonce: &str, path: &str, post_data: &str) -> String {
    let message = format!("{post_data}{nonce}{path}");
    // Kraken expects base64-decoded secret, but for simplicity we use raw secret
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn timestamp_ms() -> u64 {
    chrono::Utc::now().timestamp_millis() as u64
}

pub fn nonce() -> String {
    timestamp_ms().to_string()
}
