//! Local interface discovery: we need the NIC's MAC for the packet header and
//! its broadcast address to reach switches whose IP is on a foreign subnet.

use std::ffi::CStr;
use std::net::Ipv4Addr;

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub mac: [u8; 6],
    pub addr: Ipv4Addr,
    pub netmask: Ipv4Addr,
}

impl Interface {
    /// True if `ip` sits on this interface's subnet.
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = u32::from(self.netmask);
        u32::from(ip) & mask == u32::from(self.addr) & mask
    }
}

fn sockaddr_in_addr(sa: *const libc::sockaddr) -> Option<Ipv4Addr> {
    if sa.is_null() {
        return None;
    }
    unsafe {
        if (*sa).sa_family as i32 != libc::AF_INET {
            return None;
        }
        let sin = sa as *const libc::sockaddr_in;
        Some(Ipv4Addr::from(u32::from_be((*sin).sin_addr.s_addr)))
    }
}

/// MAC addresses come from sysfs; simpler than a second getifaddrs pass over
/// AF_PACKET entries and it fails gracefully for virtual interfaces.
fn mac_of(name: &str) -> Option<[u8; 6]> {
    let text = std::fs::read_to_string(format!("/sys/class/net/{name}/address")).ok()?;
    let mut mac = [0u8; 6];
    let mut parts = text.trim().split(':');
    for slot in mac.iter_mut() {
        *slot = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    if parts.next().is_some() || mac == [0; 6] {
        return None;
    }
    Some(mac)
}

/// Every up, non-loopback interface with an IPv4 address and a real MAC.
pub fn list() -> Vec<Interface> {
    let mut out = Vec::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return out;
    }

    let mut cur = head;
    while !cur.is_null() {
        let entry = unsafe { &*cur };
        cur = entry.ifa_next;

        let flags = entry.ifa_flags as i32;
        if flags & libc::IFF_UP == 0 || flags & libc::IFF_LOOPBACK != 0 {
            continue;
        }
        let Some(addr) = sockaddr_in_addr(entry.ifa_addr) else { continue };
        let name = unsafe { CStr::from_ptr(entry.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let Some(mac) = mac_of(&name) else { continue };

        let netmask = sockaddr_in_addr(entry.ifa_netmask).unwrap_or(Ipv4Addr::BROADCAST);

        out.push(Interface { name, mac, addr, netmask });
    }

    unsafe { libc::freeifaddrs(head) };
    out
}

/// The interface carrying the default route, which is where switches normally
/// live. Falls back to the first candidate.
pub fn default_interface() -> Option<Interface> {
    let all = list();
    let routed = std::fs::read_to_string("/proc/net/route")
        .ok()
        .and_then(|text| {
            text.lines()
                .skip(1)
                .find(|line| {
                    let mut f = line.split_whitespace();
                    // destination 00000000 marks the default route
                    matches!((f.next(), f.next()), (Some(_), Some("00000000")))
                })
                .and_then(|line| line.split_whitespace().next().map(str::to_string))
        });

    if let Some(name) = routed {
        if let Some(found) = all.iter().find(|i| i.name == name) {
            return Some(found.clone());
        }
    }
    all.into_iter().next()
}

pub fn by_name(name: &str) -> Option<Interface> {
    list().into_iter().find(|i| i.name == name)
}

pub fn format_mac(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
