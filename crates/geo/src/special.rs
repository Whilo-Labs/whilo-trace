//! Addresses set aside for special purposes (the IANA IPv4 and IPv6 special-purpose address
//! registries): private networks, carrier-grade NAT, documentation, benchmarks and the rest.
//! No database has a place for them, so they are never looked up; the home network's own
//! classes are drawn at home as "your network".

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// What a special-purpose address is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Special {
    /// 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16 (RFC 1918) and unique local fc00::/7 (RFC 4193).
    Private,
    /// 100.64.0.0/10, the provider's carrier-grade NAT (RFC 6598).
    Cgnat,
    /// 127.0.0.0/8 and ::1.
    Loopback,
    /// 169.254.0.0/16 and fe80::/10.
    LinkLocal,
    /// 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 (RFC 5737), 2001:db8::/32 (RFC 3849) and
    /// 3fff::/20 (RFC 9637).
    Documentation,
    /// 198.18.0.0/15 (RFC 2544) and 2001:2::/48 (RFC 5180).
    Benchmarking,
    /// 224.0.0.0/4 and ff00::/8.
    Multicast,
    /// The rest that is set aside: 0.0.0.0/8, 192.0.0.0/24, 192.88.99.0/24, 240.0.0.0/4, and
    /// IPv6 outside global unicast 2000::/3 or inside 2001::/23 and 2002::/16.
    Reserved,
    /// 0.0.0.0 and ::.
    Unspecified,
    /// 255.255.255.255.
    Broadcast,
}

impl Special {
    /// The name in the interface contract.
    pub fn name(self) -> &'static str {
        match self {
            Special::Private => "private",
            Special::Cgnat => "cgnat",
            Special::Loopback => "loopback",
            Special::LinkLocal => "link-local",
            Special::Documentation => "documentation",
            Special::Benchmarking => "benchmarking",
            Special::Multicast => "multicast",
            Special::Reserved => "reserved",
            Special::Unspecified => "unspecified",
            Special::Broadcast => "broadcast",
        }
    }

    /// The user's own network and the provider's NAT in front of it: drawn at home as "your
    /// network".
    pub fn is_own(self) -> bool {
        matches!(
            self,
            Special::Private | Special::Cgnat | Special::Loopback | Special::LinkLocal
        )
    }
}

/// The special-purpose class of `ip`, or `None` for an ordinary public unicast address that a
/// database may know. An IPv4-mapped IPv6 address follows the IPv4 rules.
pub fn classify(ip: IpAddr) -> Option<Special> {
    match ip.to_canonical() {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(ip: Ipv4Addr) -> Option<Special> {
    let class = match ip.octets() {
        [0, 0, 0, 0] => Special::Unspecified,
        [255, 255, 255, 255] => Special::Broadcast,
        // "This network" (RFC 791) around 0.0.0.0.
        [0, ..] => Special::Reserved,
        [10, ..] | [172, 16..=31, ..] | [192, 168, ..] => Special::Private,
        [100, 64..=127, ..] => Special::Cgnat,
        [127, ..] => Special::Loopback,
        [169, 254, ..] => Special::LinkLocal,
        [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _] => Special::Documentation,
        [198, 18..=19, ..] => Special::Benchmarking,
        // IETF protocol assignments (RFC 6890) and the retired 6to4 relay anycast (RFC 7526).
        [192, 0, 0, _] | [192, 88, 99, _] => Special::Reserved,
        [224..=239, ..] => Special::Multicast,
        // Former class E (RFC 1112).
        [240..=255, ..] => Special::Reserved,
        _ => return None,
    };
    Some(class)
}

fn classify_v6(ip: Ipv6Addr) -> Option<Special> {
    let class = match ip.segments() {
        [0, 0, 0, 0, 0, 0, 0, 0] => Special::Unspecified,
        [0, 0, 0, 0, 0, 0, 0, 1] => Special::Loopback,
        [0xfc00..=0xfdff, ..] => Special::Private,
        [0xfe80..=0xfebf, ..] => Special::LinkLocal,
        [0xff00..=0xffff, ..] => Special::Multicast,
        [0x2001, 0x0db8, ..] | [0x3fff, 0..=0x0fff, ..] => Special::Documentation,
        [0x2001, 0x0002, 0, ..] => Special::Benchmarking,
        // IETF protocol assignments (Teredo, ORCHID, AMT and others) and 6to4.
        [0x2001, 0..=0x01ff, ..] | [0x2002, ..] => Special::Reserved,
        // Global unicast.
        [0x2000..=0x3fff, ..] => return None,
        // Everything else is not global unicast: NAT64 64:ff9b::/96 and 64:ff9b:1::/48,
        // discard-only 100::/64, SRv6 5f00::/16, IPv4-compatible ::a.b.c.d, site-local fec0::/10.
        _ => Special::Reserved,
    };
    Some(class)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(cases: &[(&str, Option<Special>)]) {
        for &(addr, want) in cases {
            let ip: IpAddr = addr.parse().unwrap();
            assert_eq!(classify(ip), want, "{addr}");
        }
    }

    #[test]
    fn ipv4_ranges_and_their_boundaries() {
        use Special::*;
        check(&[
            ("0.0.0.0", Some(Unspecified)),
            ("0.0.0.1", Some(Reserved)),
            ("0.255.255.255", Some(Reserved)),
            ("1.0.0.0", None),
            ("9.255.255.255", None),
            ("10.0.0.0", Some(Private)),
            ("10.255.255.255", Some(Private)),
            ("11.0.0.0", None),
            ("100.63.255.255", None),
            ("100.64.0.0", Some(Cgnat)),
            ("100.127.255.255", Some(Cgnat)),
            ("100.128.0.0", None),
            ("126.255.255.255", None),
            ("127.0.0.0", Some(Loopback)),
            ("127.255.255.255", Some(Loopback)),
            ("128.0.0.0", None),
            ("169.253.255.255", None),
            ("169.254.0.0", Some(LinkLocal)),
            ("169.254.255.255", Some(LinkLocal)),
            ("169.255.0.0", None),
            ("172.15.255.255", None),
            ("172.16.0.0", Some(Private)),
            ("172.31.255.255", Some(Private)),
            ("172.32.0.0", None),
            ("191.255.255.255", None),
            ("192.0.0.0", Some(Reserved)),
            ("192.0.0.255", Some(Reserved)),
            ("192.0.1.0", None),
            ("192.0.1.255", None),
            ("192.0.2.0", Some(Documentation)),
            ("192.0.2.255", Some(Documentation)),
            ("192.0.3.0", None),
            ("192.88.98.255", None),
            ("192.88.99.0", Some(Reserved)),
            ("192.88.99.255", Some(Reserved)),
            ("192.88.100.0", None),
            ("192.167.255.255", None),
            ("192.168.0.0", Some(Private)),
            ("192.168.255.255", Some(Private)),
            ("192.169.0.0", None),
            ("198.17.255.255", None),
            ("198.18.0.0", Some(Benchmarking)),
            ("198.19.255.255", Some(Benchmarking)),
            ("198.20.0.0", None),
            ("198.51.99.255", None),
            ("198.51.100.0", Some(Documentation)),
            ("198.51.100.255", Some(Documentation)),
            ("198.51.101.0", None),
            ("203.0.112.255", None),
            ("203.0.113.0", Some(Documentation)),
            ("203.0.113.255", Some(Documentation)),
            ("203.0.114.0", None),
            ("223.255.255.255", None),
            ("224.0.0.0", Some(Multicast)),
            ("239.255.255.255", Some(Multicast)),
            ("240.0.0.0", Some(Reserved)),
            ("255.255.255.254", Some(Reserved)),
            ("255.255.255.255", Some(Broadcast)),
            ("8.8.8.8", None),
            ("1.1.1.1", None),
            ("77.88.8.8", None),
        ]);
    }

    #[test]
    fn ipv6_ranges_and_their_boundaries() {
        use Special::*;
        check(&[
            ("::", Some(Unspecified)),
            ("::1", Some(Loopback)),
            ("::2", Some(Reserved)),
            // IPv4-mapped addresses follow the IPv4 rules.
            ("::ffff:8.8.8.8", None),
            ("::ffff:192.168.1.1", Some(Private)),
            ("::ffff:100.64.0.1", Some(Cgnat)),
            ("::ffff:198.51.100.7", Some(Documentation)),
            ("64:ff9b::808:808", Some(Reserved)),
            ("64:ff9b:1::1", Some(Reserved)),
            ("100::1", Some(Reserved)),
            ("5f00::1", Some(Reserved)),
            ("1fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some(Reserved)),
            ("2000::", None),
            ("2001::", Some(Reserved)),
            ("2001:1ff:ffff:ffff::", Some(Reserved)),
            ("2001:200::", None),
            ("2001:1::", Some(Reserved)),
            ("2001:2::", Some(Benchmarking)),
            ("2001:2:0:ffff:ffff:ffff:ffff:ffff", Some(Benchmarking)),
            ("2001:2:1::", Some(Reserved)),
            ("2001:db7:ffff:ffff::", None),
            ("2001:db8::", Some(Documentation)),
            (
                "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff",
                Some(Documentation),
            ),
            ("2001:db9::", None),
            ("2002::", Some(Reserved)),
            ("2002:ffff:ffff::", Some(Reserved)),
            ("2003::", None),
            ("3ffe:ffff::", None),
            ("3fff::", Some(Documentation)),
            ("3fff:fff:ffff:ffff::", Some(Documentation)),
            ("3fff:1000::", None),
            ("3fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", None),
            ("4000::", Some(Reserved)),
            ("fbff:ffff::", Some(Reserved)),
            ("fc00::", Some(Private)),
            ("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some(Private)),
            ("fe00::", Some(Reserved)),
            ("fe7f:ffff::", Some(Reserved)),
            ("fe80::", Some(LinkLocal)),
            ("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some(LinkLocal)),
            ("fec0::", Some(Reserved)),
            ("feff:ffff::", Some(Reserved)),
            ("ff00::", Some(Multicast)),
            ("ff02::1", Some(Multicast)),
            ("2a00:1450:4001:82a::200e", None),
            ("2606:4700:4700::1111", None),
        ]);
    }

    #[test]
    fn names_follow_the_contract_and_own_classes_stay_at_home() {
        use Special::*;
        let all = [
            (Private, "private", true),
            (Cgnat, "cgnat", true),
            (Loopback, "loopback", true),
            (LinkLocal, "link-local", true),
            (Documentation, "documentation", false),
            (Benchmarking, "benchmarking", false),
            (Multicast, "multicast", false),
            (Reserved, "reserved", false),
            (Unspecified, "unspecified", false),
            (Broadcast, "broadcast", false),
        ];
        for (class, name, own) in all {
            assert_eq!(class.name(), name);
            assert_eq!(class.is_own(), own, "{name}");
        }
    }
}
