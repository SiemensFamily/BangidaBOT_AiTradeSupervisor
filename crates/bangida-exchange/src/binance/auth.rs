use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Credentials for the Binance Futures API.
#[derive(Debug, Clone)]
pub struct BinanceAuth {
    pub api_key: String,
    secret_key: String,
}

impl BinanceAuth {
    pub fn new(api_key: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            secret_key: secret_key.into(),
        }
    }

    /// Sign a query string with HMAC-SHA256, appending `timestamp` and `recvWindow`
    /// parameters before computing the signature.
    ///
    /// Returns the full query string including the `&signature=...` suffix.
    pub fn sign_query(&self, query: &str, recv_window: u64) -> String {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let full_query = if query.is_empty() {
            format!("timestamp={timestamp}&recvWindow={recv_window}")
        } else {
            format!("{query}&timestamp={timestamp}&recvWindow={recv_window}")
        };

        let signature = self.hmac_sign(&full_query);
        format!("{full_query}&signature={signature}")
    }

    /// Compute the HMAC-SHA256 hex digest for an arbitrary message.
    pub fn hmac_sign(&self, message: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Return the API key (used as `X-MBX-APIKEY` header).
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_sign() {
        let auth = BinanceAuth::new("key", "secret");
        let sig = auth.hmac_sign("hello");
        // deterministic HMAC output
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 64); // SHA256 = 32 bytes = 64 hex chars
    }
}
