//! Native client for usbmuxd, the daemon that multiplexes TCP connections to
//! iPhones over USB.
//!
//! * Windows: Apple Mobile Device Service listens on `127.0.0.1:27015`
//!   (installed with the Apple Devices app or iTunes).
//! * macOS/Linux: unix socket `/var/run/usbmuxd`.
//! * Override with `USBMUXD_SOCKET_ADDRESS=host:port` (same variable libimobiledevice uses).
//!
//! Wire format (plist protocol, version 1): 16-byte little-endian header
//! `{length incl. header, version=1, message=8 (plist), tag}` followed by an XML plist.
//! References: libusbmuxd `src/libusbmuxd.c`, pymobiledevice3 `usbmux.py`.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

use plist::{Dictionary, Value};
use swiftwin_core::{Result, SwiftWinError};

pub const WINDOWS_ADDR: &str = "127.0.0.1:27015";
const PLIST_MESSAGE: u32 = 8;
const PROTOCOL_VERSION: u32 = 1;

/// Where usbmuxd lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxAddr {
    Tcp(String),
    #[cfg(unix)]
    Unix(std::path::PathBuf),
}

impl MuxAddr {
    pub fn platform_default() -> MuxAddr {
        if let Ok(a) = std::env::var("USBMUXD_SOCKET_ADDRESS") {
            if let Some(p) = a.strip_prefix("UNIX:") {
                #[cfg(unix)]
                return MuxAddr::Unix(p.into());
                #[cfg(not(unix))]
                let _ = p;
            } else {
                return MuxAddr::Tcp(a);
            }
        }
        #[cfg(unix)]
        {
            MuxAddr::Unix("/var/run/usbmuxd".into())
        }
        #[cfg(not(unix))]
        {
            MuxAddr::Tcp(WINDOWS_ADDR.into())
        }
    }
}

/// A byte stream to usbmuxd (and, after `Connect`, to a port on the phone).
pub enum MuxStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl MuxStream {
    pub fn try_clone(&self) -> std::io::Result<MuxStream> {
        Ok(match self {
            MuxStream::Tcp(s) => MuxStream::Tcp(s.try_clone()?),
            #[cfg(unix)]
            MuxStream::Unix(s) => MuxStream::Unix(s.try_clone()?),
        })
    }
    pub fn shutdown(&self) {
        match self {
            MuxStream::Tcp(s) => {
                let _ = s.shutdown(Shutdown::Both);
            }
            #[cfg(unix)]
            MuxStream::Unix(s) => {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
    }
    pub fn set_read_timeout(&self, d: Option<Duration>) -> std::io::Result<()> {
        match self {
            MuxStream::Tcp(s) => s.set_read_timeout(d),
            #[cfg(unix)]
            MuxStream::Unix(s) => s.set_read_timeout(d),
        }
    }
}

impl Read for MuxStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            MuxStream::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            MuxStream::Unix(s) => s.read(buf),
        }
    }
}

impl Write for MuxStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            MuxStream::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            MuxStream::Unix(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            MuxStream::Tcp(s) => s.flush(),
            #[cfg(unix)]
            MuxStream::Unix(s) => s.flush(),
        }
    }
}

/// A phone as reported by usbmuxd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuxDevice {
    pub device_id: u64,
    pub udid: String,
    /// "USB" or "Network".
    pub connection_type: String,
    pub product_id: Option<u64>,
}

/// Compare UDIDs ignoring dashes and case (usbmuxd may report them without the dash).
pub fn same_udid(a: &str, b: &str) -> bool {
    let n = |s: &str| s.replace('-', "").to_ascii_lowercase();
    n(a) == n(b)
}

/// Spellings to try when looking up a pairing record: `udid` itself, then the other form of a
/// 25-character UDID (`XXXXXXXX-XXXXXXXXXXXXXXXX`, iPhone XS and newer). Apple's Windows usbmuxd
/// matches `ReadPairRecord` IDs ignoring case but only with the dash (checked against the real
/// service), and keeps the records as `C:\ProgramData\Apple\Lockdown\<UDID>.plist`.
pub fn udid_variants(udid: &str) -> Vec<String> {
    let mut v = vec![udid.to_string()];
    let bare = udid.replace('-', "");
    if bare.len() == 24 && bare.chars().all(|c| c.is_ascii_hexdigit()) {
        let other = if udid.contains('-') { bare } else { format!("{}-{}", &bare[..8], &bare[8..]) };
        v.push(other);
    }
    v
}

#[derive(Debug, Clone)]
pub struct Usbmux {
    pub addr: MuxAddr,
}

impl Usbmux {
    pub fn new(addr: MuxAddr) -> Self {
        Usbmux { addr }
    }

    pub fn platform() -> Self {
        Usbmux::new(MuxAddr::platform_default())
    }

    fn open(&self) -> Result<MuxStream> {
        let s = match &self.addr {
            MuxAddr::Tcp(a) => MuxStream::Tcp(TcpStream::connect(a).map_err(|e| {
                SwiftWinError::tool(
                    "usbmuxd",
                    format!(
                        "cannot reach Apple Mobile Device Service at {a} ({e}). Install the Apple Devices app \
                         from the Microsoft Store (or iTunes) and make sure the service is running"
                    ),
                )
            })?),
            #[cfg(unix)]
            MuxAddr::Unix(p) => MuxStream::Unix(
                std::os::unix::net::UnixStream::connect(p)
                    .map_err(|e| SwiftWinError::tool("usbmuxd", format!("cannot open {}: {e}", p.display())))?,
            ),
        };
        s.set_read_timeout(Some(Duration::from_secs(10)))?;
        Ok(s)
    }

    fn request(&self, stream: &mut MuxStream, msg_type: &str, extra: Dictionary) -> Result<Dictionary> {
        let mut d = Dictionary::new();
        d.insert("MessageType".into(), msg_type.into());
        d.insert("ClientVersionString".into(), "swiftwin".into());
        d.insert("ProgName".into(), "swiftwin".into());
        d.insert("kLibUSBMuxVersion".into(), Value::Integer(3.into()));
        for (k, v) in extra {
            d.insert(k, v);
        }
        write_packet(stream, 1, &Value::Dictionary(d))?;
        let (_, v) = read_packet(stream)?;
        v.into_dictionary().ok_or_else(|| SwiftWinError::protocol("usbmuxd reply is not a dictionary"))
    }

    pub fn list_devices(&self) -> Result<Vec<MuxDevice>> {
        let mut s = self.open()?;
        let reply = self.request(&mut s, "ListDevices", Dictionary::new())?;
        let list = reply
            .get("DeviceList")
            .and_then(|v| v.as_array())
            .ok_or_else(|| SwiftWinError::protocol("ListDevices reply has no DeviceList"))?;
        Ok(list.iter().filter_map(parse_device).collect())
    }

    /// Pairing record for a device, or `None` if this computer is not trusted yet.
    pub fn read_pair_record(&self, udid: &str) -> Result<Option<Vec<u8>>> {
        let mut s = self.open()?;
        let mut extra = Dictionary::new();
        extra.insert("PairRecordID".into(), udid.into());
        let reply = self.request(&mut s, "ReadPairRecord", extra)?;
        if let Some(d) = reply.get("PairRecordData").and_then(|v| v.as_data()) {
            return Ok(Some(d.to_vec()));
        }
        Ok(None)
    }

    /// Open a raw TCP connection to `port` on the phone.
    pub fn connect(&self, device_id: u64, port: u16) -> Result<MuxStream> {
        let mut s = self.open()?;
        let mut extra = Dictionary::new();
        extra.insert("DeviceID".into(), Value::Integer(device_id.into()));
        // usbmuxd expects the port in network byte order inside a host integer.
        extra.insert("PortNumber".into(), Value::Integer(u64::from(port.to_be()).into()));
        let reply = self.request(&mut s, "Connect", extra)?;
        match reply.get("Number").and_then(|v| v.as_unsigned_integer()) {
            Some(0) => {
                s.set_read_timeout(None)?;
                Ok(s)
            }
            Some(3) => Err(SwiftWinError::protocol(format!("connection to device port {port} refused"))),
            Some(2) => Err(SwiftWinError::DeviceNotFound(format!("usbmux device id {device_id}"))),
            other => Err(SwiftWinError::protocol(format!("usbmuxd Connect failed with result {other:?}"))),
        }
    }
}

fn parse_device(v: &Value) -> Option<MuxDevice> {
    let d = v.as_dictionary()?;
    let props = d.get("Properties")?.as_dictionary()?;
    Some(MuxDevice {
        device_id: d.get("DeviceID").or_else(|| props.get("DeviceID"))?.as_unsigned_integer()?,
        udid: props.get("SerialNumber")?.as_string()?.to_string(),
        connection_type: props.get("ConnectionType").and_then(|v| v.as_string()).unwrap_or("USB").to_string(),
        product_id: props.get("ProductID").and_then(|v| v.as_unsigned_integer()),
    })
}

pub fn write_packet(w: &mut impl Write, tag: u32, v: &Value) -> Result<()> {
    let mut body = Vec::new();
    v.to_writer_xml(&mut body).map_err(|e| SwiftWinError::protocol(e.to_string()))?;
    let mut pkt = Vec::with_capacity(16 + body.len());
    pkt.extend_from_slice(&((16 + body.len()) as u32).to_le_bytes());
    pkt.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    pkt.extend_from_slice(&PLIST_MESSAGE.to_le_bytes());
    pkt.extend_from_slice(&tag.to_le_bytes());
    pkt.extend_from_slice(&body);
    w.write_all(&pkt)?;
    w.flush()?;
    Ok(())
}

pub fn read_packet(r: &mut impl Read) -> Result<(u32, Value)> {
    let mut hdr = [0u8; 16];
    r.read_exact(&mut hdr)?;
    let len = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
    let tag = u32::from_le_bytes(hdr[12..16].try_into().unwrap());
    if !(16..=16 * 1024 * 1024).contains(&len) {
        return Err(SwiftWinError::protocol(format!("bad usbmuxd packet length {len}")));
    }
    let mut body = vec![0u8; len - 16];
    r.read_exact(&mut body)?;
    let v = plist::from_bytes(&body).map_err(|e| SwiftWinError::protocol(format!("usbmuxd plist: {e}")))?;
    Ok((tag, v))
}

/// Forward local TCP connections on 127.0.0.1 to `device_port` on the phone.
/// Returns the local port. Runs on background threads for the life of the process.
pub fn spawn_forwarder(mux: Usbmux, device_id: u64, device_port: u16) -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let local = listener.local_addr()?.port();
    std::thread::Builder::new().name(format!("fwd-{device_port}")).spawn(move || {
        for client in listener.incoming().flatten() {
            let mux = mux.clone();
            std::thread::spawn(move || {
                let Ok(remote) = mux.connect(device_id, device_port) else {
                    let _ = client.shutdown(Shutdown::Both);
                    return;
                };
                pipe(client, remote);
            });
        }
    })?;
    Ok(local)
}

fn pipe(client: TcpStream, remote: MuxStream) {
    let (Ok(mut c2), Ok(mut r2)) = (client.try_clone(), remote.try_clone()) else { return };
    let (mut c1, mut r1) = (client, remote);
    let up = std::thread::spawn(move || {
        let _ = std::io::copy(&mut c1, &mut r1);
        r1.shutdown();
    });
    let _ = std::io::copy(&mut r2, &mut c2);
    let _ = c2.shutdown(Shutdown::Both);
    let _ = up.join();
}

#[cfg(test)]
pub(crate) mod fake {
    //! A fake usbmuxd + phone used by tests. It speaks the real wire format.
    use super::*;
    use std::net::TcpListener;

    pub struct FakeMux {
        pub addr: String,
    }

    /// Start a fake usbmuxd with one USB device. `paired` controls ReadPairRecord.
    /// After `Connect` to port 62078 it behaves like an unpaired lockdownd;
    /// any other port echoes bytes back.
    pub fn start(udid: &'static str, paired: bool) -> FakeMux {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                std::thread::spawn(move || serve(s, udid, paired));
            }
        });
        FakeMux { addr }
    }

    fn serve(mut s: TcpStream, udid: &str, paired: bool) {
        let Ok((tag, v)) = read_packet(&mut s) else { return };
        let d = v.into_dictionary().unwrap();
        let mut reply = Dictionary::new();
        match d.get("MessageType").and_then(|v| v.as_string()).unwrap() {
            "ListDevices" => {
                let mut props = Dictionary::new();
                props.insert("ConnectionType".into(), "USB".into());
                props.insert("DeviceID".into(), Value::Integer(7.into()));
                props.insert("SerialNumber".into(), udid.replace('-', "").into());
                props.insert("ProductID".into(), Value::Integer(4776.into()));
                let mut dev = Dictionary::new();
                dev.insert("DeviceID".into(), Value::Integer(7.into()));
                dev.insert("MessageType".into(), "Attached".into());
                dev.insert("Properties".into(), Value::Dictionary(props));
                reply.insert("DeviceList".into(), Value::Array(vec![Value::Dictionary(dev)]));
            }
            "ReadPairRecord" => {
                // Like Apple's Windows usbmuxd: the ID matches ignoring case, but only with the dash.
                let id = d.get("PairRecordID").and_then(|v| v.as_string()).unwrap_or("");
                if paired && id.eq_ignore_ascii_case(udid) {
                    reply.insert("PairRecordData".into(), Value::Data(b"<pair/>".to_vec()));
                } else {
                    reply.insert("MessageType".into(), "Result".into());
                    reply.insert("Number".into(), Value::Integer(2.into()));
                }
            }
            "Connect" => {
                let raw = d.get("PortNumber").unwrap().as_unsigned_integer().unwrap() as u16;
                let port = u16::from_be(raw);
                let ok = d.get("DeviceID").unwrap().as_unsigned_integer() == Some(7);
                reply.insert("MessageType".into(), "Result".into());
                reply.insert("Number".into(), Value::Integer(if ok { 0 } else { 2 }.into()));
                write_packet(&mut s, tag, &Value::Dictionary(reply)).unwrap();
                if !ok {
                    return;
                }
                if port == crate::real::lockdown::LOCKDOWN_PORT {
                    crate::real::lockdown::fake::serve(s, udid);
                } else {
                    let mut r = s.try_clone().unwrap();
                    let _ = std::io::copy(&mut r, &mut s);
                }
                return;
            }
            _ => {}
        }
        let _ = write_packet(&mut s, tag, &Value::Dictionary(reply));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udid_spellings() {
        assert_eq!(udid_variants("00008110-000A1B2C3D4E5F60"), ["00008110-000A1B2C3D4E5F60", "00008110000A1B2C3D4E5F60"]);
        assert_eq!(udid_variants("00008110000A1B2C3D4E5F60"), ["00008110000A1B2C3D4E5F60", "00008110-000A1B2C3D4E5F60"]);
        // Old 40-character UDIDs have no dash form.
        assert_eq!(udid_variants("abc").len(), 1);
        assert_eq!(udid_variants(&"f".repeat(40)).len(), 1);
    }

    #[test]
    fn port_is_network_order() {
        assert_eq!(62078u16.to_be(), if cfg!(target_endian = "little") { 0x7EF2 } else { 62078 });
    }

    #[test]
    fn list_pair_and_connect_against_fake_usbmuxd() {
        let f = fake::start("00008110-000A1B2C3D4E5F60", true);
        let mux = Usbmux::new(MuxAddr::Tcp(f.addr.clone()));
        let devs = mux.list_devices().unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].device_id, 7);
        assert!(same_udid(&devs[0].udid, "00008110-000A1B2C3D4E5F60"));
        assert!(mux.read_pair_record("00008110-000a1b2c3d4e5f60").unwrap().is_some());
        assert!(mux.read_pair_record(&devs[0].udid).unwrap().is_none(), "the fake lists the serial without the dash");
        let mut s = mux.connect(7, 8100).unwrap();
        s.write_all(b"ping").unwrap();
        let mut buf = [0u8; 4];
        s.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
        assert!(mux.connect(99, 8100).is_err());
    }

    #[test]
    fn unpaired_has_no_record() {
        let f = fake::start("abc", false);
        let mux = Usbmux::new(MuxAddr::Tcp(f.addr));
        assert_eq!(mux.read_pair_record("abc").unwrap(), None);
    }

    #[test]
    fn forwarder_pipes_bytes() {
        let f = fake::start("abc", true);
        let mux = Usbmux::new(MuxAddr::Tcp(f.addr));
        let local = spawn_forwarder(mux, 7, 8100).unwrap();
        let mut c = TcpStream::connect(("127.0.0.1", local)).unwrap();
        c.write_all(b"hello").unwrap();
        let mut buf = [0u8; 5];
        c.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
    }

    #[test]
    fn unreachable_service_gives_helpful_error() {
        let mux = Usbmux::new(MuxAddr::Tcp("127.0.0.1:1".into()));
        let e = mux.list_devices().unwrap_err().to_string();
        assert!(e.contains("Apple Devices app"), "{e}");
    }
}
