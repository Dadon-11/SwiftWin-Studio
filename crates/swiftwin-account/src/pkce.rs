//! PKCE (RFC 7636): a random verifier and its S256 challenge.

use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// 64 characters from the unreserved set (RFC 7636 allows 43..=128), from the OS RNG.
pub fn verifier() -> String {
    let mut rng = rand::rngs::OsRng;
    (0..64).map(|_| UNRESERVED[rng.gen_range(0..UNRESERVED.len())] as char).collect()
}

/// `BASE64URL(SHA256(verifier))` without padding.
pub fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_appendix_b() {
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let v = verifier();
        assert_eq!(v.len(), 64);
        assert_ne!(v, verifier());
    }
}
