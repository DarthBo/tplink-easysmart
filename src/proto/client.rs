//! Request/response handling over the discovery socket.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use super::iface::Interface;
use super::packet::{op, tlv, Header, Packet, Tlv, HOST_PORT, SWITCH_PORT};
use crate::model::SwitchInfo;

/// Everything goes out as a *limited* broadcast (255.255.255.255), even
/// requests aimed at a single switch. That is deliberate, and copied from the
/// vendor utility: a switch sitting on a foreign subnet -- the factory default
/// 192.168.0.1, say -- drops a subnet-directed broadcast like 10.0.255.255
/// because it is not its own broadcast address, but accepts the limited one.
/// Replies are matched on the MAC in the header rather than on the source IP.
pub struct Client {
    socket: UdpSocket,
    iface: Interface,
    sequence: u16,
    token: u16,
}

impl Client {
    pub fn bind(iface: Interface) -> io::Result<Client> {
        // Bind the wildcard address, not the interface address. Switches answer
        // to 255.255.255.255 and Linux will not deliver those to a socket bound
        // to a specific unicast address -- the bug that makes the vendor utility
        // find nothing when run under a Linux JRE.
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, HOST_PORT))?;
        socket.set_broadcast(true)?;
        // Short enough that a window with no reply still ends promptly.
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        // A wildcard bind would otherwise send the limited broadcast out
        // whichever interface the routing table prefers, ignoring --interface.
        bind_to_device(&socket, &iface.name)?;
        Ok(Client {
            socket,
            iface,
            sequence: rand_seed(),
            token: 0,
        })
    }

    pub fn interface(&self) -> &Interface {
        &self.iface
    }

    fn next_sequence(&mut self) -> u16 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    fn send(&self, packet: &Packet) -> io::Result<()> {
        let target = SocketAddr::from(SocketAddrV4::new(Ipv4Addr::BROADCAST, SWITCH_PORT));
        self.socket.send_to(&packet.encode(), target)?;
        Ok(())
    }

    /// Collect replies carrying `sequence`. Stops early once `done` accepts a
    /// packet, and otherwise waits out `window`.
    ///
    /// Discovery has to wait the full window because any number of switches may
    /// answer, but a request aimed at one switch expects exactly one reply --
    /// draining the rest of the window there just makes the UI feel dead.
    fn collect_until(
        &mut self,
        sequence: u16,
        window: Duration,
        done: impl Fn(&Packet) -> bool,
    ) -> Vec<Packet> {
        let deadline = Instant::now() + window;
        let mut buf = [0u8; 8192];
        let mut out = Vec::new();
        while Instant::now() < deadline {
            let n = match self.socket.recv_from(&mut buf) {
                Ok((n, _)) => n,
                Err(ref e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue
                }
                Err(_) => break,
            };
            let Some(pkt) = Packet::decode(&buf[..n]) else { continue };
            if pkt.header.sequence != sequence {
                continue;
            }
            if pkt.header.token != 0 {
                self.token = pkt.header.token;
            }
            let finished = done(&pkt);
            out.push(pkt);
            if finished {
                break;
            }
        }
        out
    }

    /// Collect every reply carrying `sequence` for the whole window.
    fn collect(&mut self, sequence: u16, window: Duration) -> Vec<Packet> {
        self.collect_until(sequence, window, |_| false)
    }

    fn request(&mut self, opcode: u8, switch_mac: [u8; 6], tlvs: Vec<Tlv>) -> Packet {
        let seq = self.next_sequence();
        let mut header = Header::new(opcode, seq, switch_mac, self.iface.mac);
        header.token = self.token;
        Packet { header, tlvs }
    }

    /// Broadcast a discovery request and gather every switch that answers.
    pub fn discover(&mut self, window: Duration) -> io::Result<Vec<SwitchInfo>> {
        let packet = self.request(op::DISCOVER, [0; 6], vec![]);
        let seq = packet.header.sequence;
        self.send(&packet)?;

        let mut found: Vec<SwitchInfo> = Vec::new();
        for reply in self.collect(seq, window) {
            if reply.header.opcode != op::READ_REPLY {
                continue;
            }
            let info = SwitchInfo::from_packet(&reply);
            if !found.iter().any(|s| s.mac == info.mac) {
                found.push(info);
            }
        }
        found.sort_by_key(|s| (s.ip, s.mac));
        Ok(found)
    }

    /// Re-read one switch's system info. No credentials needed.
    pub fn fetch(&mut self, mac: [u8; 6], window: Duration) -> io::Result<Option<SwitchInfo>> {
        let packet = self.request(op::GET, mac, vec![Tlv::empty(tlv::DESCRIPTION)]);
        let seq = packet.header.sequence;
        self.send(&packet)?;

        let is_reply = move |p: &Packet| p.header.opcode == op::READ_REPLY && p.header.switch_mac == mac;
        Ok(self
            .collect_until(seq, window, is_reply)
            .into_iter()
            .find(is_reply)
            .map(|p| SwitchInfo::from_packet(&p)))
    }

    /// Apply description and IP settings. Credentials are mandatory for any
    /// write; the switch answers with error 7 if they are wrong.
    pub fn apply_settings(
        &mut self,
        mac: [u8; 6],
        creds: &Credentials,
        settings: &IpSettings,
        window: Duration,
    ) -> io::Result<Result<(), String>> {
        // Order matters: the utility puts the credentials first.
        let mut tlvs = vec![
            Tlv::string(tlv::USERNAME, &creds.username),
            Tlv::string(tlv::PASSWORD, &creds.password),
            Tlv::string(tlv::DESCRIPTION, &settings.description),
            Tlv::new(tlv::DHCP, vec![settings.dhcp as u8]),
        ];
        // With DHCP on, the utility sends zeroes rather than the stale values.
        let (ip, mask, gw) = if settings.dhcp {
            (Ipv4Addr::UNSPECIFIED, Ipv4Addr::UNSPECIFIED, Ipv4Addr::UNSPECIFIED)
        } else {
            (settings.ip, settings.netmask, settings.gateway)
        };
        tlvs.push(Tlv::new(tlv::IP, ip.octets().to_vec()));
        tlvs.push(Tlv::new(tlv::NETMASK, mask.octets().to_vec()));
        tlvs.push(Tlv::new(tlv::GATEWAY, gw.octets().to_vec()));

        self.write_request(mac, tlvs, window)
    }

    /// Ask the switch to persist its running config to flash.
    pub fn save_config(
        &mut self,
        mac: [u8; 6],
        creds: &Credentials,
        window: Duration,
    ) -> io::Result<Result<(), String>> {
        let tlvs = vec![
            Tlv::string(tlv::USERNAME, &creds.username),
            Tlv::string(tlv::PASSWORD, &creds.password),
            Tlv::empty(tlv::SAVE_CONFIG),
        ];
        self.write_request(mac, tlvs, window)
    }

    /// Writes are gated on a single-use token. The switch issues one in reply
    /// to a TLV 2305 read, and silently drops any set carrying a zero or stale
    /// token -- no error, no response at all -- so one must be fetched
    /// immediately before every write.
    fn refresh_token(&mut self, mac: [u8; 6], window: Duration) -> io::Result<bool> {
        let packet = self.request(op::GET, mac, vec![Tlv::empty(tlv::TOKEN)]);
        let seq = packet.header.sequence;
        self.send(&packet)?;

        let issued = |p: &Packet| {
            p.header.opcode == op::READ_REPLY && p.header.switch_mac == mac && p.header.token != 0
        };
        // collect_until records the token as it goes.
        Ok(self.collect_until(seq, window, issued).iter().any(issued))
    }

    fn write_request(
        &mut self,
        mac: [u8; 6],
        tlvs: Vec<Tlv>,
        window: Duration,
    ) -> io::Result<Result<(), String>> {
        if !self.refresh_token(mac, window)? {
            return Ok(Err("switch did not issue a write token".to_string()));
        }
        let packet = self.request(op::SET, mac, tlvs);
        let seq = packet.header.sequence;
        self.send(&packet)?;

        let is_reply = move |p: &Packet| p.header.opcode == op::SET_REPLY && p.header.switch_mac == mac;
        let reply = self
            .collect_until(seq, window, is_reply)
            .into_iter()
            .find(is_reply);

        Ok(match reply {
            None => Err("no response from switch".to_string()),
            Some(p) if p.header.error == 0 => Ok(()),
            Some(p) => Err(format!(
                "{} (code {})",
                super::packet::describe_error(p.header.error),
                p.header.error
            )),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct IpSettings {
    pub description: String,
    pub dhcp: bool,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub gateway: Ipv4Addr,
}

/// Pin the socket to one interface with SO_BINDTODEVICE, which -- unlike a
/// bind to the interface address -- constrains the egress device while leaving
/// the socket able to receive broadcasts. Needs no privileges.
fn bind_to_device(socket: &UdpSocket, name: &str) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let cname = std::ffi::CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad interface name"))?;
    // Length includes the NUL, matching what the kernel expects.
    let len = (cname.as_bytes().len() + 1) as libc::socklen_t;
    let rc = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            cname.as_ptr() as *const libc::c_void,
            len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The sequence number only has to be unpredictable enough that stale replies
/// from a previous run are not mistaken for ours.
fn rand_seed() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u16)
        .unwrap_or(0x1234)
}
