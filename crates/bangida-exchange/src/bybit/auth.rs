use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Credentials and HMAC signing for the Bybit V5 API.
///
/// Bybit's signature scheme differs from Binance: the pre-sign string is
/// `timestamp + api_key + recv_window + queryString` (for GET) or
/// `timestamp + api_key + recv_window + body` (for POST).
#[derive(Debug, Clone)]
pub struct BybitAuth {
    api_key: String,
    secret_key: String,
}

impl BybitAuth {
    pub fn new(api_key: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            secret_key: secret_key.into(),
        }
    }

    /// Return the API key (used as `X-BAPI-API-KEY` header).
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Return the secret key (needed for WebSocket auth).
    pub fn secret_key(&self) -> &str {
        &self.secret_key
    }

    /// Compute the HMAC-SHA256 hex digest of an arbitrary message.
    pub fn hmac_sign(&self, message: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Build the full set of authentication headers for a Bybit V5 REST request.
    ///
    /// Returns `(timestamp_str, sign, recv_window_str)` which the caller must
    /// attach as `X-BAPI-TIMESTAMP`, `X-BAPI-SIGN`, `X-BAPI-RECV-WINDOW`.
    ///
    /// * `payload` - the query string (for GET) or the JSON body (for POST).
    /// * `recv_window` - recommended 5000.
    pub fn sign_request(&self, payload: &str, recv_window: u64) -> (String, String, String) {
        let timestamp = chrono::Utc::now().timestamp_millis().to_string();
        let recv_window_str = recv_window.to_string();
        let pre_sign = format!("{}{}{}{}", timestamp, self.api_key, recv_window_str, payload);
        let signature = self.hmac_sign(&pre_sign);
        (timestamp, signature, recv_window_str)
    }

    /// Produce the HMAC signature for Bybit WebSocket private authentication.
    ///
    /// The message to sign is `"GET/realtime" + expires` where `expires` is a
    /// millisecond timestamp slightly in the future.
    pub fn ws_auth_signature(&self, expires: u64) -> String {
        let message = format!("GET/realtime{expires}");
        self.hmac_sign(&message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_sign() {
        let auth = BybitAuth::new("key", "secret");
        let sig = auth.hmac_sign("hello");
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn test_sign_request_returns_three_parts() {
        let auth = BybitAuth::new("mykey", "mysecret");
        let (ts, sign, rw) = auth.sign_request("symbol=BTCUSDT", 5000);
        assert!(!ts.is_empty());
        assert_eq!(sign.len(), 64);
        assert_eq!(rw, "5000");
    }
}
