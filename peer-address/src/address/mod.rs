use std::net::{IpAddr, Ipv4Addr};

use nimiq_hash::{Blake2bHash, Blake2bHasher, Hasher};
use nimiq_keys::PublicKey;

pub use self::net_address::*;
pub use self::peer_address::*;
pub use self::peer_uri::PeerUri;
pub use self::seed_list::SeedList;

pub mod net_address;
pub mod peer_address;
pub mod peer_uri;
pub mod seed_list;

create_typed_array!(PeerId, u8, 16);
add_hex_io_fns_typed_arr!(PeerId, PeerId::SIZE);

impl From<Blake2bHash> for PeerId {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        PeerId::from(&hash_arr[0..PeerId::len()])
    }
}

impl<'a> From<&'a PublicKey> for PeerId {
    fn from(public_key: &'a PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        PeerId::from(hash)
    }
}

fn is_ip_globally_reachable_legacy(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => is_ipv4_globally_reachable_legacy(*ipv4),
        IpAddr::V6(ipv6) => {
            let octets = ipv6.octets();
            // check for local ip ::1
            if octets[0..octets.len() - 1] == [0; 15] && octets[15] == 1 {
                return false;
            }
            // Private subnet is fc00::/7.
            // So, we only check the first 7 bits of the address to be equal fc00.
            if (octets[0] & 0xfe) == 0xfc {
                return false;
            }
            // Link-local addresses are fe80::/10.
            if octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80 {
                return false;
            }
            // IPv4-mapped or compatible
            if let Some(ipv4) = ipv6.to_ipv4() {
                return is_ipv4_globally_reachable_legacy(ipv4);
            }
            true
        }
    }
}

fn is_ipv4_globally_reachable_legacy(ipv4: Ipv4Addr) -> bool {
    let octets = ipv4.octets();
    if let [127, 0, 0, 1] = octets {
        return false;
    }
    if ipv4.is_private() || ipv4.is_link_local() {
        return false;
    }
    // 100.64.0.0/10
    if octets[0] == 100 && (octets[1] >= 64 && octets[1] <= 127) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::is_ip_globally_reachable_legacy;
    use std::{net::IpAddr, str::FromStr};

    #[test]
    fn is_ip_globally_reachable_legacy_falsifys() {
        let bad_ips = vec![
            // Local IPs
            "127.0.0.1",
            // Private IPs
            "192.168.2.1",
            "172.16.0.0",
            "172.31.0.0",
            "172.31.255.255",
            "100.64.0.0",
            "169.254.0.0",
            "fd12:3456:789a:1::1",
            "fe80:3456:789a:1::1",
            "fd00:3456:789a:1::1",
            // Private IPv4-mapped IPv6
            "::ffff:127.0.0.1",
        ];
        for bad_ip in bad_ips {
            assert!(!is_ip_globally_reachable_legacy(&IpAddr::from_str(bad_ip).unwrap()));
        }
    }

    #[test]
    fn is_ip_globally_reachable_legacy_verifies() {
        let good_ips = vec![
            // Non-Private IPs
            "100.168.2.1",
            "172.32.0.0",
            "172.15.255.255",
            "fbff:3456:789a:1::1",
            "fe00:3456:789a:1::1",
            "ff02:3456:789a:1::1",
            "::3456:789a:1:1",
            // IPv4-mapped IPv6
            "::ffff:100.168.2.1",
        ];
        for good_ip in good_ips {
            assert!(is_ip_globally_reachable_legacy(&IpAddr::from_str(good_ip).unwrap()));
        }
    }
}
