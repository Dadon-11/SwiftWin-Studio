//! Just enough DER encoding to build a PKCS#10 certificate signing request.

pub fn len(n: usize) -> Vec<u8> {
    if n < 0x80 {
        vec![n as u8]
    } else {
        let b: Vec<u8> = n.to_be_bytes().iter().copied().skip_while(|&x| x == 0).collect();
        let mut v = vec![0x80 | b.len() as u8];
        v.extend(b);
        v
    }
}

pub fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    v.extend(len(content.len()));
    v.extend_from_slice(content);
    v
}

pub fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

pub fn set(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x31, &parts.concat())
}

pub fn int_small(v: u8) -> Vec<u8> {
    tlv(0x02, &[v])
}

pub fn oid(encoded: &[u8]) -> Vec<u8> {
    tlv(0x06, encoded)
}

pub fn utf8(s: &str) -> Vec<u8> {
    tlv(0x0C, s.as_bytes())
}

pub fn printable(s: &str) -> Vec<u8> {
    tlv(0x13, s.as_bytes())
}

pub fn null() -> Vec<u8> {
    vec![0x05, 0x00]
}

pub fn bit_string(bytes: &[u8]) -> Vec<u8> {
    let mut c = vec![0u8];
    c.extend_from_slice(bytes);
    tlv(0x03, &c)
}

/// Context-specific constructed tag `[n]`.
pub fn ctx(n: u8, content: &[u8]) -> Vec<u8> {
    tlv(0xA0 | n, content)
}

pub const OID_CN: &[u8] = &[0x55, 0x04, 0x03];
pub const OID_C: &[u8] = &[0x55, 0x04, 0x06];
pub const OID_SHA256_RSA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B];

pub fn pem(label: &str, der: &[u8]) -> String {
    let b64 = swiftwin_core::base64_bytes::encode(der);
    let mut s = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        s.push_str(std::str::from_utf8(chunk).unwrap());
        s.push('\n');
    }
    s.push_str(&format!("-----END {label}-----\n"));
    s
}

pub fn pem_decode(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    swiftwin_core::base64_bytes::decode(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lengths() {
        assert_eq!(len(5), vec![5]);
        assert_eq!(len(200), vec![0x81, 200]);
        assert_eq!(len(300), vec![0x82, 1, 44]);
        let p = pem("TEST", &[1, 2, 3]);
        assert_eq!(pem_decode(&p).unwrap(), vec![1, 2, 3]);
    }
}
