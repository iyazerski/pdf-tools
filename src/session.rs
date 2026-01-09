use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use time::{Duration, OffsetDateTime};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub(crate) struct SessionSigner {
    key: Vec<u8>,
    ttl: Duration,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SessionPayload {
    pub(crate) u: String,
    pub(crate) exp_unix: i64,
}

impl SessionSigner {
    pub(crate) fn new(key: Vec<u8>, ttl: Duration) -> Self {
        Self { key, ttl }
    }

    pub(crate) fn issue(&self, username: &str, now: OffsetDateTime) -> String {
        let payload = SessionPayload {
            u: username.to_string(),
            exp_unix: (now + self.ttl).unix_timestamp(),
        };

        let payload_json =
            serde_json::to_vec(&payload).expect("session payload must serialize to JSON");
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);

        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC key must be non-empty");
        mac.update(payload_b64.as_bytes());
        let sig = mac.finalize().into_bytes();
        let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);

        format!("v1.{payload_b64}.{sig_b64}")
    }

    pub(crate) fn verify(&self, token: &str, now: OffsetDateTime) -> Option<SessionPayload> {
        let mut parts = token.split('.');
        let version = parts.next()?;
        let payload_b64 = parts.next()?;
        let sig_b64 = parts.next()?;
        if version != "v1" || parts.next().is_some() {
            return None;
        }

        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sig_b64.as_bytes())
            .ok()?;

        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(payload_b64.as_bytes());
        mac.verify_slice(&sig).ok()?;

        let payload_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64.as_bytes())
            .ok()?;
        let payload: SessionPayload = serde_json::from_slice(&payload_json).ok()?;
        if payload.exp_unix <= now.unix_timestamp() {
            return None;
        }
        Some(payload)
    }
}
