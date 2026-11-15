//! Minimal lockdownd client (port 62078): framing + unauthenticated requests.
//!
//! Messages are a 4-byte big-endian length followed by an XML plist.
//! Without a TLS session only a few keys are readable (enough for device
//! detection). Everything needing a session goes through pymobiledevice3.

use std::io::{Read, Write};

use plist::{Dictionary, Value};
use swiftwin_core::{Result, SwiftWinError};

use super::usbmux::MuxStream;

pub const LOCKDOWN_PORT: u16 = 62078;

pub fn send(w: &mut impl Write, d: Dictionary) -> Result<()> {
    let mut body = Vec::new();
    Value::Dictionary(d).to_writer_xml(&mut body).map_err(|e| SwiftWinError::protocol(e.to_string()))?;
    w.write_all(&(body.len() as u32).to_be_bytes())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

pub fn recv(r: &mut impl Read) -> Result<Dictionary> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > 8 * 1024 * 1024 {
        return Err(SwiftWinError::protocol("lockdown message too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let v: Value = plist::from_bytes(&body).map_err(|e| SwiftWinError::protocol(format!("lockdown plist: {e}")))?;
    v.into_dictionary().ok_or_else(|| SwiftWinError::protocol("lockdown reply is not a dictionary"))
}

pub struct Lockdown {
    s: MuxStream,
}

impl Lockdown {
    pub fn new(s: MuxStream) -> Self {
        Lockdown { s }
    }

    fn call(&mut self, request: &str, extra: Dictionary) -> Result<Dictionary> {
        let mut d = Dictionary::new();
        d.insert("Label".into(), "swiftwin".into());
        d.insert("Request".into(), request.into());
        for (k, v) in extra {
            d.insert(k, v);
        }
        send(&mut self.s, d)?;
        let r = recv(&mut self.s)?;
        if let Some(e) = r.get("Error").and_then(|v| v.as_string()) {
            return Err(SwiftWinError::protocol(format!("lockdown {request}: {e}")));
        }
        Ok(r)
    }

    /// Should be "com.apple.mobile.lockdown".
    pub fn query_type(&mut self) -> Result<String> {
        let r = self.call("QueryType", Dictionary::new())?;
        Ok(r.get("Type").and_then(|v| v.as_string()).unwrap_or_default().to_string())
    }

    pub fn get_value(&mut self, key: &str) -> Result<Value> {
        let mut e = Dictionary::new();
        e.insert("Key".into(), key.into());
        let r = self.call("GetValue", e)?;
        r.get("Value").cloned().ok_or_else(|| SwiftWinError::NotFound(format!("lockdown value {key}")))
    }

    pub fn get_string(&mut self, key: &str) -> Option<String> {
        self.get_value(key).ok().and_then(|v| v.as_string().map(str::to_string))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::net::TcpStream;

    /// Answer like an unpaired lockdownd: a few public keys, nothing else.
    pub fn serve(mut s: TcpStream, udid: &str) {
        while let Ok(req) = recv(&mut s) {
            let mut r = Dictionary::new();
            match req.get("Request").and_then(|v| v.as_string()) {
                Some("QueryType") => {
                    r.insert("Type".into(), "com.apple.mobile.lockdown".into());
                }
                Some("GetValue") => {
                    let key = req.get("Key").and_then(|v| v.as_string()).unwrap_or("");
                    let v: Option<Value> = match key {
                        "ProductVersion" => Some("17.5.1".into()),
                        "ProductType" => Some("iPhone15,2".into()),
                        "UniqueDeviceID" => Some(udid.into()),
                        "DeviceName" => Some("Test iPhone".into()),
                        _ => None,
                    };
                    match v {
                        Some(v) => {
                            r.insert("Key".into(), key.into());
                            r.insert("Value".into(), v);
                        }
                        None => {
                            r.insert("Error".into(), "MissingValue".into());
                        }
                    }
                }
                _ => {
                    r.insert("Error".into(), "InvalidRequest".into());
                }
            }
            if send(&mut s, r).is_err() {
                return;
            }
        }
    }
}
