use std::net::Ipv4Addr;
use std::sync::{Arc, RwLock};

use if_addrs::{IfAddr, Interface};

pub const DISCOVERY_MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 40, 45);

const NON_LAN_TOKENS: &[&str] = &[
    "tun", "utun", "tap", "wg", "ppp", "pptp", "l2tp", "ipsec", "xfrm", "rmnet", "ccmni",
    "pdp_ip", "wwan", "clat", "dummy", "ifb", "sit", "gre", "vti", "ip6tnl", "ovpn", "warp",
    "lo",
];

const NON_LAN_HINTS: &[&str] = &[
    "wireguard",
    "tailscale",
    "zerotier",
    "hamachi",
    "clash",
    "sing-box",
    "singbox",
    "nekobox",
    "v2ray",
    "xray",
    "amnezia",
    "openvpn",
    "proton",
    "mullvad",
    "nordlynx",
    "radmin",
    "virtualbox",
    "vmware",
    "hyper-v",
    "vethernet",
];

#[derive(Debug, Clone)]
pub struct LanInterface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub broadcast: Ipv4Addr,
}

impl LanInterface {
    pub fn contains_ip(&self, target: Ipv4Addr) -> bool {
        let mask = u32::from(self.netmask);
        if mask == 0 {
            return false;
        }
        (u32::from(self.ip) & mask) == (u32::from(target) & mask)
    }
}

pub type LanInterfaceHook = Arc<dyn Fn() -> Vec<LanInterface> + Send + Sync>;
pub type SocketBindHook = Arc<dyn Fn(i32) + Send + Sync>;

static LAN_INTERFACE_HOOK: RwLock<Option<LanInterfaceHook>> = RwLock::new(None);
static SOCKET_BIND_HOOK: RwLock<Option<SocketBindHook>> = RwLock::new(None);

pub fn set_lan_interface_hook(hook: LanInterfaceHook) {
    *LAN_INTERFACE_HOOK
        .write()
        .expect("lan_interface_hook lock poisoned") = Some(hook);
}

pub fn set_socket_bind_hook(hook: SocketBindHook) {
    *SOCKET_BIND_HOOK
        .write()
        .expect("socket_bind_hook lock poisoned") = Some(hook);
}

pub fn bind_socket_to_lan_network(fd: i32) {
    let hook = SOCKET_BIND_HOOK
        .read()
        .expect("socket_bind_hook lock poisoned")
        .clone();
    if let Some(hook) = hook {
        hook(fd);
    }
}

pub fn is_non_lan_interface_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    if NON_LAN_HINTS.iter().any(|hint| lower.contains(hint)) {
        return true;
    }
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|tok| !tok.is_empty())
        .any(|tok| {
            let stripped = tok.trim_end_matches(|c: char| c.is_ascii_digit());
            NON_LAN_TOKENS.contains(&tok) || NON_LAN_TOKENS.contains(&stripped)
        })
}

pub fn enumerate_lan_interfaces() -> Vec<LanInterface> {
    let mut result = Vec::new();

    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        result.extend(ifaces.into_iter().filter_map(extract_lan_ipv4));
    }

    let hook = LAN_INTERFACE_HOOK
        .read()
        .expect("lan_interface_hook lock poisoned")
        .clone();
    if let Some(hook) = hook {
        for iface in hook() {
            if !is_non_lan_interface_name(&iface.name)
                && !iface.ip.is_loopback()
                && !iface.ip.is_link_local()
                && !iface.ip.is_unspecified()
                && !result.iter().any(|existing| existing.ip == iface.ip)
            {
                result.push(iface);
            }
        }
    }

    result
}

pub fn is_ip_in_local_lan(ip: Ipv4Addr) -> bool {
    enumerate_lan_interfaces()
        .iter()
        .any(|iface| iface.contains_ip(ip))
}

fn extract_lan_ipv4(iface: Interface) -> Option<LanInterface> {
    if iface.is_loopback() || is_non_lan_interface_name(&iface.name) {
        return None;
    }
    let IfAddr::V4(v4) = iface.addr else {
        return None;
    };
    if v4.ip.is_loopback() || v4.ip.is_link_local() || v4.ip.is_unspecified() {
        return None;
    }
    let broadcast = v4.broadcast.unwrap_or_else(|| {
        let ip_u32 = u32::from(v4.ip);
        let mask_u32 = u32::from(v4.netmask);
        Ipv4Addr::from(ip_u32 | !mask_u32)
    });
    Some(LanInterface {
        name: iface.name,
        ip: v4.ip,
        netmask: v4.netmask,
        broadcast,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_vpn_and_tunnel_interfaces() {
        assert!(is_non_lan_interface_name("tun0"));
        assert!(is_non_lan_interface_name("wg-quick"));
        assert!(is_non_lan_interface_name("Tailscale"));
        assert!(is_non_lan_interface_name("AmneziaWG"));
        assert!(is_non_lan_interface_name("vEthernet (WSL)"));
        assert!(!is_non_lan_interface_name("wlan0"));
        assert!(!is_non_lan_interface_name("Ethernet"));
        assert!(!is_non_lan_interface_name("en0"));
    }
}
