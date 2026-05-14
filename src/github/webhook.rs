//! Plan 10e — GitHub webhook signature validation.
//!
//! GitHub signs webhook deliveries with HMAC-SHA256 of the raw body using
//! a per-webhook secret. The signature arrives in the `X-Hub-Signature-256`
//! header as `sha256=<hex>`. We compute the expected HMAC and constant-time
//! compare via `subtle::ConstantTimeEq`.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("missing or malformed signature header")]
    MalformedSignature,

    #[error("signature mismatch")]
    Mismatch,
}

/// Verify `body`'s HMAC-SHA256 signature against the `sha256=<hex>` value
/// in `signature_header` using `secret` as the HMAC key.
pub fn verify_signature(body: &[u8], signature_header: &str, secret: &str) -> Result<(), WebhookError> {
    let Some(hex_part) = signature_header.strip_prefix("sha256=") else {
        return Err(WebhookError::MalformedSignature);
    };
    let provided = hex::decode(hex_part).map_err(|_| WebhookError::MalformedSignature)?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| WebhookError::MalformedSignature)?;
    mac.update(body);
    let expected = mac.finalize().into_bytes();

    if expected.ct_eq(&provided).into() {
        Ok(())
    } else {
        Err(WebhookError::Mismatch)
    }
}
