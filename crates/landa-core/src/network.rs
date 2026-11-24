use std::net::Ipv4Addr;
use std::sync::{Arc, RwLock};

use if_addrs::{IfAddr, Interface};
use serde::{Deserialize, Serialize};

pub const DISCOVERY_MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 40, 45);

const NON_LAN_TOKENS: &[&str] = &[
    "tun", "utun", "tap", "wg", "ppp", "pptp", "l2tp", "ipsec", "xfrm", "rmnet", "ccmni", "pdp_ip",
    "wwan", "clat", "dummy", "ifb", "sit", "gre", "vti", "ip6tnl", "ovpn", "warp", "lo",
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

    pub fn subnet_cidr(&self) -> String {
        let mask = u32::from(self.netmask);
        let net = Ipv4Addr::from(u32::from(self.ip) & mask);
        let prefix = if mask.leading_ones() + mask.trailing_zeros() == 32 {
            mask.leading_ones()
        } else {
            mask.count_ones()
        };
        format!("{net}/{prefix}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LanInterfaceStatus {
    pub name: String,
    pub ip: String,
    pub netmask: String,
    pub broadcast: String,
    pub subnet_cidr: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExcludedSubnet {
    network: u32,
    mask: u32,
    cidr: String,
}

pub type LanInterfaceHook = Arc<dyn Fn() -> Vec<LanInterface> + Send + Sync>;
pub type SocketBindHook = Arc<dyn Fn(i32) + Send + Sync>;

static LAN_INTERFACE_HOOK: RwLock<Option<LanInterfaceHook>> = RwLock::new(None);
static SOCKET_BIND_HOOK: RwLock<Option<SocketBindHook>> = RwLock::new(None);
static EXCLUDED_SUBNETS: RwLock<Vec<ExcludedSubnet>> = RwLock::new(Vec::new());

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

pub fn parse_ipv4_cidr(raw: &str) -> Option<(Ipv4Addr, u32, String)> {
    let trimmed = raw.trim();
    let (ip_part, prefix_part) = trimmed.split_once('/')?;
    let ip: Ipv4Addr = ip_part.trim().parse().ok()?;
    let prefix: u32 = prefix_part.trim().parse().ok()?;
    if prefix > 32 {
        return None;
    }
    let mask = if prefix == 0 {
        0u32
    } else {
        !0u32 << (32 - prefix)
    };
    let network_u32 = u32::from(ip) & mask;
    let network_ip = Ipv4Addr::from(network_u32);
    Some((network_ip, mask, format!("{network_ip}/{prefix}")))
}

pub fn normalize_excluded_subnets(cidrs: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in cidrs {
        if let Some((_, _, normalized)) = parse_ipv4_cidr(raw) {
            if !out.contains(&normalized) {
                out.push(normalized);
            }
        }
    }
    out
}

pub fn set_excluded_subnets(cidrs: &[String]) {
    let mut parsed = Vec::new();
    for raw in cidrs {
        if let Some((net, mask, normalized)) = parse_ipv4_cidr(raw) {
            if !parsed.iter().any(|e: &ExcludedSubnet| e.cidr == normalized) {
                parsed.push(ExcludedSubnet {
                    network: u32::from(net),
                    mask,
                    cidr: normalized,
                });
            }
        }
    }
    *EXCLUDED_SUBNETS
        .write()
        .expect("excluded_subnets lock poisoned") = parsed;
}

pub fn get_excluded_subnets() -> Vec<String> {
    EXCLUDED_SUBNETS
        .read()
        .expect("excluded_subnets lock poisoned")
        .iter()
        .map(|e| e.cidr.clone())
        .collect()
}

pub fn is_ip_in_excluded_subnet(ip: Ipv4Addr) -> bool {
    let ip_u32 = u32::from(ip);
    EXCLUDED_SUBNETS
        .read()
        .expect("excluded_subnets lock poisoned")
        .iter()
        .any(|sub| (ip_u32 & sub.mask) == sub.network)
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

pub fn enumerate_all_lan_interfaces() -> Vec<LanInterface> {
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

pub fn enumerate_lan_interfaces() -> Vec<LanInterface> {
    enumerate_all_lan_interfaces()
        .into_iter()
        .filter(|iface| !is_ip_in_excluded_subnet(iface.ip))
        .collect()
}

pub fn list_lan_interface_statuses() -> Vec<LanInterfaceStatus> {
    enumerate_all_lan_interfaces()
        .into_iter()
        .map(|iface| {
            let subnet_cidr = iface.subnet_cidr();
            let enabled = !is_ip_in_excluded_subnet(iface.ip);
            LanInterfaceStatus {
                name: iface.name,
                ip: iface.ip.to_string(),
                netmask: iface.netmask.to_string(),
                broadcast: iface.broadcast.to_string(),
                subnet_cidr,
                enabled,
            }
        })
        .collect()
}

pub fn is_ip_in_local_lan(ip: Ipv4Addr) -> bool {
    if is_ip_in_excluded_subnet(ip) {
        return false;
    }
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
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn parses_and_normalizes_ipv4_cidrs() {
        assert_eq!(
            parse_ipv4_cidr("192.168.1.42/24"),
            Some((
                Ipv4Addr::new(192, 168, 1, 0),
                0xFFFF_FF00,
                "192.168.1.0/24".to_string()
            ))
        );
        assert_eq!(
            normalize_excluded_subnets(&[
                "192.168.56.15/24".into(),
                "192.168.56.1/24".into(),
                "invalid".into(),
                "10.0.0.5/33".into(),
                "172.16.5.9/16".into(),
            ]),
            vec!["192.168.56.0/24".to_string(), "172.16.0.0/16".to_string()]
        );
    }

    #[test]
    fn excluded_subnets_filter_active_interfaces_but_keep_status_list() {
        let _guard = TEST_LOCK.lock().unwrap();
        let prev_hook = LAN_INTERFACE_HOOK.read().unwrap().clone();
        let prev_excluded = get_excluded_subnets();

        set_lan_interface_hook(Arc::new(|| {
            vec![
                LanInterface {
                    name: "test-lan-a".into(),
                    ip: Ipv4Addr::new(198, 51, 100, 10),
                    netmask: Ipv4Addr::new(255, 255, 255, 0),
                    broadcast: Ipv4Addr::new(198, 51, 100, 255),
                },
                LanInterface {
                    name: "test-lan-b".into(),
                    ip: Ipv4Addr::new(203, 0, 113, 20),
                    netmask: Ipv4Addr::new(255, 255, 255, 0),
                    broadcast: Ipv4Addr::new(203, 0, 113, 255),
                },
            ]
        }));

        set_excluded_subnets(&["198.51.100.0/24".into()]);

        let active = enumerate_lan_interfaces();
        assert!(!active
            .iter()
            .any(|i| i.ip == Ipv4Addr::new(198, 51, 100, 10)));
        assert!(active
            .iter()
            .any(|i| i.ip == Ipv4Addr::new(203, 0, 113, 20)));
        assert!(!is_ip_in_local_lan(Ipv4Addr::new(198, 51, 100, 55)));
        assert!(is_ip_in_local_lan(Ipv4Addr::new(203, 0, 113, 55)));

        let statuses = list_lan_interface_statuses();
        let a_status = statuses.iter().find(|s| s.name == "test-lan-a").unwrap();
        assert_eq!(a_status.subnet_cidr, "198.51.100.0/24");
        assert!(!a_status.enabled);

        let b_status = statuses.iter().find(|s| s.name == "test-lan-b").unwrap();
        assert_eq!(b_status.subnet_cidr, "203.0.113.0/24");
        assert!(b_status.enabled);

        // Re-enabling restores interface to active list
        set_excluded_subnets(&[]);
        let restored = enumerate_lan_interfaces();
        assert!(restored
            .iter()
            .any(|i| i.ip == Ipv4Addr::new(198, 51, 100, 10)));

        *LAN_INTERFACE_HOOK.write().unwrap() = prev_hook;
        set_excluded_subnets(&prev_excluded);
    }

    #[test]
    fn overlapping_excluded_subnet_rejects_target_ip_in_is_ip_in_local_lan() {
        let _guard = TEST_LOCK.lock().unwrap();
        let prev_hook = LAN_INTERFACE_HOOK.read().unwrap().clone();
        let prev_excluded = get_excluded_subnets();

        set_lan_interface_hook(Arc::new(|| {
            vec![LanInterface {
                name: "test-lan-wide".into(),
                ip: Ipv4Addr::new(192, 168, 2, 10),
                netmask: Ipv4Addr::new(255, 255, 0, 0),
                broadcast: Ipv4Addr::new(192, 168, 255, 255),
            }]
        }));

        set_excluded_subnets(&["192.168.1.0/24".into()]);

        assert!(!is_ip_in_local_lan(Ipv4Addr::new(192, 168, 1, 55)));
        assert!(is_ip_in_local_lan(Ipv4Addr::new(192, 168, 2, 55)));

        *LAN_INTERFACE_HOOK.write().unwrap() = prev_hook;
        set_excluded_subnets(&prev_excluded);
    }
}
