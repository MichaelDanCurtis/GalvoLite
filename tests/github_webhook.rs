//! Plan 10e Task 1 — HMAC validator tests.

use galvolite::github::webhook::{verify_signature, WebhookError};

// Known-good HMAC computed externally:
// body = "hello world", secret = "topsecret"
// sha256 = 8a3a84bcd5d0a4d49ce6db96b57ac9d6c8d20cf04ee9a91dad6dd99b3eaf3a8a
// (verified via `echo -n 'hello world' | openssl dgst -sha256 -hmac 'topsecret'`)

#[test]
fn verify_passes_with_correct_signature() {
    let body = b"hello world";
    let secret = "topsecret";
    // Compute expected signature dynamically to avoid hard-coding (which
    // would couple this test to openssl output formatting):
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    assert!(verify_signature(body, &sig, secret).is_ok());
}

#[test]
fn verify_fails_with_wrong_signature() {
    let body = b"hello world";
    let secret = "topsecret";
    let bad_sig = "sha256=0000000000000000000000000000000000000000000000000000000000000000";
    let err = verify_signature(body, bad_sig, secret).unwrap_err();
    assert!(matches!(err, WebhookError::Mismatch));
}

#[test]
fn verify_fails_with_malformed_header() {
    let body = b"x";
    assert!(matches!(
        verify_signature(body, "no-prefix", "s"),
        Err(WebhookError::MalformedSignature)
    ));
    assert!(matches!(
        verify_signature(body, "sha256=not-hex", "s"),
        Err(WebhookError::MalformedSignature)
    ));
}

#[test]
fn verify_fails_when_body_modified() {
    let body = b"original";
    let secret = "s";
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    let tampered = b"tampered";
    assert!(matches!(
        verify_signature(tampered, &sig, secret),
        Err(WebhookError::Mismatch)
    ));
}
