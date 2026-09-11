//! The switch record shown in the discovery table.

use std::net::Ipv4Addr;

use crate::proto::iface::{format_mac, Interface};
use crate::proto::packet::{tlv, Packet};

#[derive(Debug, Clone)]
pub struct SwitchInfo {
    pub model: String,
    pub description: String,
    pub mac: [u8; 6],
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub firmware: String,
    pub hardware: String,
    pub dhcp: bool,
    pub port_count: Option<u8>,
}

impl SwitchInfo {
    pub fn from_packet(packet: &Packet) -> SwitchInfo {
        let string = |k| packet.find(k).map(|t| t.as_string()).unwrap_or_default();
        let ip = |k| {
            packet
                .find(k)
                .and_then(|t| t.as_ipv4())
                .unwrap_or(Ipv4Addr::UNSPECIFIED)
        };
        SwitchInfo {
            model: string(tlv::MODEL),
            description: string(tlv::DESCRIPTION),
            // Fall back to the header MAC; a system-info reply always has both.
            mac: packet
                .find(tlv::MAC)
                .and_then(|t| t.as_mac())
                .unwrap_or(packet.header.switch_mac),
            ip: ip(tlv::IP),
            netmask: ip(tlv::NETMASK),
            gateway: ip(tlv::GATEWAY),
            firmware: string(tlv::FIRMWARE),
            hardware: string(tlv::HARDWARE),
            dhcp: packet
                .find(tlv::DHCP)
                .and_then(|t| t.as_byte())
                .is_some_and(|b| b == 1),
            port_count: packet.find(tlv::PORT_COUNT).and_then(|t| t.as_byte()),
        }
    }

    pub fn mac_string(&self) -> String {
        format_mac(&self.mac)
    }

    /// A switch still on its factory address cannot be reached over IP from a
    /// different subnet, so the web UI is not an option for it.
    pub fn reachable_from(&self, iface: &Interface) -> bool {
        !self.ip.is_unspecified() && iface.contains(self.ip)
    }

    pub fn web_url(&self) -> String {
        format!("http://{}", self.ip)
    }
}
