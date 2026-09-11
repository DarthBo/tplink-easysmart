//! Wire format: a fixed 32-byte header followed by big-endian TLVs.

use super::crypto::rc4;

pub const HEADER_LEN: usize = 32;
pub const SWITCH_PORT: u16 = 29808;
pub const HOST_PORT: u16 = 29809;

/// Header `opcode` values. Requests are even-to-odd, replies are request + 1
/// for discovery/get (both answer with 2) and 4 for set.
pub mod op {
    pub const DISCOVER: u8 = 0;
    pub const GET: u8 = 1;
    pub const READ_REPLY: u8 = 2;
    pub const SET: u8 = 3;
    pub const SET_REPLY: u8 = 4;
}

/// TLV types used by the discovery and system-info pages. A few are never
/// read back but are kept so this stays a complete map of the wire format.
#[allow(dead_code)]
pub mod tlv {
    pub const MODEL: u16 = 1;
    pub const DESCRIPTION: u16 = 2;
    pub const MAC: u16 = 3;
    pub const IP: u16 = 4;
    pub const NETMASK: u16 = 5;
    pub const GATEWAY: u16 = 6;
    pub const FIRMWARE: u16 = 7;
    pub const HARDWARE: u16 = 8;
    pub const DHCP: u16 = 9;
    pub const PORT_COUNT: u16 = 10;
    pub const AUTOSAVE: u16 = 13;
    pub const IS_FACTORY: u16 = 14;
    pub const USERNAME: u16 = 512;
    pub const PASSWORD: u16 = 514;
    pub const SAVE_CONFIG: u16 = 2304;
    /// Terminates the TLV list: type 0xFFFF with zero length.
    pub const END: u16 = 0xFFFF;
}

/// Error codes the switch reports in the header.
pub fn describe_error(code: i32) -> &'static str {
    match code {
        0 => "ok",
        -2 => "save to flash failed",
        6 => "invalid IP address",
        7 => "wrong username or password",
        8 => "access denied",
        9 => "invalid subnet mask",
        10 => "invalid gateway",
        _ => "unknown error",
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub version: u8,
    pub opcode: u8,
    /// The switch being addressed; all zeroes broadcasts to every switch.
    pub switch_mac: [u8; 6],
    pub host_mac: [u8; 6],
    pub sequence: u16,
    pub error: i32,
    pub length: u16,
    pub fragment: u16,
    pub token: u16,
}

impl Header {
    pub fn new(opcode: u8, sequence: u16, switch_mac: [u8; 6], host_mac: [u8; 6]) -> Self {
        Header {
            version: 1,
            opcode,
            switch_mac,
            host_mac,
            sequence,
            error: 0,
            length: 0,
            fragment: 0,
            token: 0,
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.push(self.version);
        out.push(self.opcode);
        out.extend_from_slice(&self.switch_mac);
        out.extend_from_slice(&self.host_mac);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.error.to_be_bytes());
        out.extend_from_slice(&self.length.to_be_bytes());
        out.extend_from_slice(&self.fragment.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes()); // flag, unused
        out.extend_from_slice(&self.token.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // checksum, always zero
    }

    fn parse(buf: &[u8]) -> Option<Header> {
        if buf.len() < HEADER_LEN {
            return None;
        }
        let be16 = |o: usize| u16::from_be_bytes([buf[o], buf[o + 1]]);
        Some(Header {
            version: buf[0],
            opcode: buf[1],
            switch_mac: buf[2..8].try_into().ok()?,
            host_mac: buf[8..14].try_into().ok()?,
            sequence: be16(14),
            error: i32::from_be_bytes(buf[16..20].try_into().ok()?),
            length: be16(20),
            fragment: be16(22),
            token: be16(26),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Tlv {
    pub kind: u16,
    pub value: Vec<u8>,
}

impl Tlv {
    pub fn new(kind: u16, value: impl Into<Vec<u8>>) -> Self {
        Tlv { kind, value: value.into() }
    }

    pub fn empty(kind: u16) -> Self {
        Tlv { kind, value: Vec::new() }
    }

    /// Trailing NULs are common in string fields; strip them.
    pub fn as_string(&self) -> String {
        String::from_utf8_lossy(&self.value)
            .trim_end_matches('\0')
            .to_string()
    }

    pub fn as_byte(&self) -> Option<u8> {
        self.value.first().copied()
    }

    pub fn as_ipv4(&self) -> Option<std::net::Ipv4Addr> {
        let octets: [u8; 4] = self.value.get(..4)?.try_into().ok()?;
        Some(octets.into())
    }

    pub fn as_mac(&self) -> Option<[u8; 6]> {
        self.value.get(..6)?.try_into().ok()
    }
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub header: Header,
    pub tlvs: Vec<Tlv>,
}

impl Packet {
    /// Serialise and encrypt, ready to put on the wire.
    pub fn encode(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        for tlv in &self.tlvs {
            payload.extend_from_slice(&tlv.kind.to_be_bytes());
            payload.extend_from_slice(&(tlv.value.len() as u16).to_be_bytes());
            payload.extend_from_slice(&tlv.value);
        }
        payload.extend_from_slice(&tlv::END.to_be_bytes());
        payload.extend_from_slice(&0u16.to_be_bytes());

        let mut header = self.header.clone();
        header.length = (payload.len() + HEADER_LEN) as u16;

        let mut out = Vec::with_capacity(payload.len() + HEADER_LEN);
        header.write(&mut out);
        out.extend_from_slice(&payload);
        rc4(&mut out);
        out
    }

    /// Decrypt and parse. Returns `None` for anything that is not a plausible
    /// reply, which is how the vendor utility screens the socket too.
    pub fn decode(raw: &[u8]) -> Option<Packet> {
        let mut buf = raw.to_vec();
        rc4(&mut buf);
        let header = Header::parse(&buf)?;
        if header.version != 1 || header.opcode > op::SET_REPLY {
            return None;
        }

        let end = (header.length as usize).min(buf.len());
        let mut tlvs = Vec::new();
        let mut n = HEADER_LEN;
        while n + 4 <= end {
            let kind = u16::from_be_bytes([buf[n], buf[n + 1]]);
            let len = u16::from_be_bytes([buf[n + 2], buf[n + 3]]) as usize;
            n += 4;
            if kind == tlv::END && len == 0 {
                break;
            }
            if n + len > buf.len() {
                break;
            }
            tlvs.push(Tlv::new(kind, &buf[n..n + len]));
            n += len;
        }
        Some(Packet { header, tlvs })
    }

    pub fn find(&self, kind: u16) -> Option<&Tlv> {
        self.tlvs.iter().find(|t| t.kind == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_round_trips_through_encryption() {
        let mac = [0xc0, 0x06, 0xc3, 0x1a, 0xca, 0xa8];
        let pkt = Packet {
            header: Header::new(op::GET, 0x2001, mac, [1, 2, 3, 4, 5, 6]),
            tlvs: vec![Tlv::empty(tlv::DESCRIPTION)],
        };
        let wire = pkt.encode();
        let back = Packet::decode(&wire).expect("decodes");
        assert_eq!(back.header.opcode, op::GET);
        assert_eq!(back.header.sequence, 0x2001);
        assert_eq!(back.header.switch_mac, mac);
        assert_eq!(back.tlvs.len(), 1);
        assert_eq!(back.tlvs[0].kind, tlv::DESCRIPTION);
    }

    #[test]
    fn header_length_counts_header_and_payload() {
        let pkt = Packet {
            header: Header::new(op::DISCOVER, 1, [0; 6], [0; 6]),
            tlvs: vec![],
        };
        // Just the 4-byte end marker beyond the header.
        assert_eq!(Packet::decode(&pkt.encode()).unwrap().header.length as usize, HEADER_LEN + 4);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(Packet::decode(&[0u8; 8]).is_none());
    }
}
