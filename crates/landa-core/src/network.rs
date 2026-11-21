use std::net::Ipv4Addr;

use if_addrs::{IfAddr, Interface};

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
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };

    ifaces
        .into_iter()
        .filter_map(extract_lan_ipv4)
        .collect()
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
