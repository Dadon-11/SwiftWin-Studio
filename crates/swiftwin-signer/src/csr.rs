//! RSA-2048 key generation and PKCS#10 CSR creation (what Keychain Access
//! does when you "Request a Certificate From a Certificate Authority").

use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::signature::{SignatureEncoding, Signer};
use rsa::RsaPrivateKey;
use sha2::Sha256;
use swiftwin_core::{Result, SwiftWinError};

use crate::der;

pub struct KeyPair {
    pub key: RsaPrivateKey,
}

impl KeyPair {
    pub fn generate() -> Result<KeyPair> {
        let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).map_err(|e| SwiftWinError::other(format!("RSA: {e}")))?;
        Ok(KeyPair { key })
    }

    pub fn from_pkcs8_pem(pem: &str) -> Result<KeyPair> {
        let key = RsaPrivateKey::from_pkcs8_pem(pem).map_err(|e| SwiftWinError::invalid(format!("private key: {e}")))?;
        Ok(KeyPair { key })
    }

    pub fn to_pkcs8_pem(&self) -> Result<String> {
        Ok(self.key.to_pkcs8_pem(LineEnding::LF).map_err(|e| SwiftWinError::other(e.to_string()))?.to_string())
    }

    pub fn public_key_der(&self) -> Result<Vec<u8>> {
        Ok(self.key.to_public_key().to_public_key_der().map_err(|e| SwiftWinError::other(e.to_string()))?.into_vec())
    }

    /// PEM-encoded PKCS#10 CSR with subject `CN=<common_name>, C=<country>`.
    pub fn csr_pem(&self, common_name: &str, country: &str) -> Result<String> {
        let name = der::seq(&[
            der::set(&[der::seq(&[der::oid(der::OID_C), der::printable(country)])]),
            der::set(&[der::seq(&[der::oid(der::OID_CN), der::utf8(common_name)])]),
        ]);
        let info = der::seq(&[der::int_small(0), name, self.public_key_der()?, der::ctx(0, &[])]);
        let sig = SigningKey::<Sha256>::new(self.key.clone()).sign(&info).to_vec();
        let alg = der::seq(&[der::oid(der::OID_SHA256_RSA), der::null()]);
        let csr = der::seq(&[info, alg, der::bit_string(&sig)]);
        Ok(der::pem("CERTIFICATE REQUEST", &csr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::signature::Verifier;

    #[test]
    fn csr_is_self_consistent() {
        let kp = KeyPair::generate().unwrap();
        let pem = kp.csr_pem("SwiftWin Development", "US").unwrap();
        assert!(pem.starts_with("-----BEGIN CERTIFICATE REQUEST-----\n"));
        let der = der::pem_decode(&pem).unwrap();
        // Outer SEQUENCE { info, alg, sig }: re-split and verify the signature over `info`.
        assert_eq!(der[0], 0x30);
        let (hdr, _) = parse_len(&der[1..]);
        let body = &der[1 + hdr..];
        let (ih, il) = parse_len(&body[1..]);
        let info = &body[..1 + ih + il];
        let rest = &body[info.len()..];
        let (ah, al) = parse_len(&rest[1..]);
        let bits = &rest[1 + ah + al..];
        assert_eq!(bits[0], 0x03);
        let (bh, bl) = parse_len(&bits[1..]);
        let sig = &bits[1 + bh + 1..1 + bh + bl];
        let vk = VerifyingKey::<Sha256>::new(kp.key.to_public_key());
        vk.verify(info, &Signature::try_from(sig).unwrap()).unwrap();
        // Key round-trips through PEM.
        let again = KeyPair::from_pkcs8_pem(&kp.to_pkcs8_pem().unwrap()).unwrap();
        assert_eq!(again.public_key_der().unwrap(), kp.public_key_der().unwrap());
    }

    fn parse_len(b: &[u8]) -> (usize, usize) {
        if b[0] < 0x80 {
            (1, b[0] as usize)
        } else {
            let n = (b[0] & 0x7f) as usize;
            (1 + n, b[1..=n].iter().fold(0usize, |a, &x| (a << 8) | x as usize))
        }
    }
}
