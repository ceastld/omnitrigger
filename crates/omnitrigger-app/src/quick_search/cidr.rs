use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use num_bigint::BigUint;

use super::calculator::normalize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CidrError {
    Incomplete,
    Address,
    Prefix,
    DifferentFamilies,
    ReversedRange,
}

impl CidrError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 IP 地址和前缀长度，例如 192.0.2.137/26",
            Self::Address => "请输入有效的 IPv4 或 IPv6 地址",
            Self::Prefix => "请输入有效的前缀长度或连续的 IPv4 掩码",
            Self::DifferentFamilies => "地址和网段须使用相同的 IP 版本",
            Self::ReversedRange => "起始地址不得大于结束地址",
        }
    }
}

fn prefix_length(text: &str, width: u32) -> Result<u32, CidrError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(CidrError::Incomplete);
    }
    if width == 32 && text.contains('.') {
        let mask = text
            .parse::<Ipv4Addr>()
            .map(u32::from)
            .map_err(|_| CidrError::Prefix)?;
        return if mask.leading_ones() == mask.count_ones() {
            Ok(mask.leading_ones())
        } else {
            Err(CidrError::Prefix)
        };
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CidrError::Prefix);
    }
    text.parse()
        .ok()
        .filter(|&prefix| prefix <= width)
        .ok_or(CidrError::Prefix)
}

fn bits(address: IpAddr) -> u128 {
    match address {
        IpAddr::V4(address) => u128::from(u32::from(address)),
        IpAddr::V6(address) => u128::from(address),
    }
}

fn address_width(address: IpAddr) -> u32 {
    if address.is_ipv4() {
        32
    } else {
        128
    }
}

fn as_address(bits: u128, width: u32) -> IpAddr {
    if width == 32 {
        IpAddr::V4(Ipv4Addr::from(bits as u32))
    } else {
        IpAddr::V6(Ipv6Addr::from(bits))
    }
}

fn host_mask(host_bits: u32) -> u128 {
    u128::MAX.checked_shr(128 - host_bits).unwrap_or(0)
}

pub(super) fn calculate_cidr(query: &str) -> Result<String, CidrError> {
    let input = normalize(query).replace(char::is_whitespace, " ");
    if let Some((operation, range)) = input.split_once(' ') {
        match operation.to_ascii_lowercase().as_str() {
            "覆盖" | "cover" => return range_to_cidr(range, true),
            "分解" | "range-to-cidr" => return range_to_cidr(range, false),
            _ => {}
        }
    }
    let (network, member) = input
        .split_once("包含")
        .or_else(|| {
            let index = input.to_ascii_lowercase().find(" contains ")?;
            Some((&input[..index], &input[index + " contains ".len()..]))
        })
        .map_or((input.as_str(), None), |(network, member)| {
            (network, Some(member.trim()))
        });
    let (address, prefix) = network.split_once('/').ok_or(CidrError::Incomplete)?;
    let address = address.trim();
    if address.is_empty() {
        return Err(CidrError::Incomplete);
    }
    let address = address.parse::<IpAddr>().map_err(|_| CidrError::Address)?;
    let width = address_width(address);
    let prefix = prefix_length(prefix, width)?;
    let host_bits = width - prefix;
    let host_mask = host_mask(host_bits);
    let first = bits(address) & !host_mask;
    let last = first | host_mask;
    let network = format!("{}/{prefix}", as_address(first, width));
    if let Some(member) = member {
        if member.is_empty() {
            return Err(CidrError::Incomplete);
        }
        let member = member.parse::<IpAddr>().map_err(|_| CidrError::Address)?;
        if member.is_ipv4() != address.is_ipv4() {
            return Err(CidrError::DifferentFamilies);
        }
        let relation = if (first..=last).contains(&bits(member)) {
            "属于"
        } else {
            "不属于"
        };
        return Ok(format!("{member} {relation} {network}"));
    }
    let mask = if address.is_ipv4() {
        format!("掩码 {}\n", Ipv4Addr::from(!(host_mask as u32)))
    } else {
        String::new()
    };
    let count = BigUint::from(1u8) << host_bits;
    Ok(format!(
        "网段 {network}\n{mask}范围 {} 至 {}\n地址数 {count}",
        as_address(first, width),
        as_address(last, width)
    ))
}

fn range_to_cidr(input: &str, cover_only: bool) -> Result<String, CidrError> {
    let (first, last) = input.split_once(['至', ',']).ok_or(CidrError::Incomplete)?;
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() || last.is_empty() {
        return Err(CidrError::Incomplete);
    }
    let first = first.parse::<IpAddr>().map_err(|_| CidrError::Address)?;
    let last = last.parse::<IpAddr>().map_err(|_| CidrError::Address)?;
    let width = address_width(first);
    if address_width(last) != width {
        return Err(CidrError::DifferentFamilies);
    }
    let (mut cursor, last) = (bits(first), bits(last));
    if cursor > last {
        return Err(CidrError::ReversedRange);
    }

    if cover_only {
        let host_bits = 128 - (cursor ^ last).leading_zeros();
        let start = as_address(cursor & !host_mask(host_bits), width);
        return Ok(format!("{start}/{}", width - host_bits));
    }

    let mut networks = Vec::new();
    loop {
        // 选取起点对齐且完全位于剩余区间中的最大网段。
        let remaining_bits = (last - cursor).checked_add(1).map_or(128, u128::ilog2);
        let host_bits = cursor.trailing_zeros().min(remaining_bits).min(width);
        let start = as_address(cursor, width);
        networks.push(format!("{start}/{}", width - host_bits));
        let block_last = cursor | host_mask(host_bits);
        if block_last == last {
            return Ok(networks.join("\n"));
        }
        cursor = block_last + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_ranges_to_one_covering_network_or_exact_cidr_blocks() {
        for (input, expected) in [
            ("覆盖 192.0.2.1 至 192.0.2.6", "192.0.2.0/29"),
            ("分解 192.0.2.1，192.0.2.6", "192.0.2.1/32\n192.0.2.2/31\n192.0.2.4/31\n192.0.2.6/32"),
            ("cover 192.0.2.1, 192.0.2.1", "192.0.2.1/32"),
            ("分解 192.0.2.128 至 192.0.2.191", "192.0.2.128/26"),
            ("分解 0.0.0.0 至 255.255.255.255", "0.0.0.0/0"),
            ("分解 2001:db8::1 至 2001:db8::3", "2001:db8::1/128\n2001:db8::2/127"),
            ("覆盖 2001:db8::1 至 2001:db8::3", "2001:db8::/126"),
            ("分解 :: 至 ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", "::/0"),
            ("覆盖 :: 至 8000::", "::/0"),
            ("range-to-cidr ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe, ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe/127"),
        ] {
            assert_eq!(calculate_cidr(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, expected) in [
            ("分解 192.0.2.6 至 192.0.2.1", CidrError::ReversedRange),
            ("覆盖 192.0.2.1 至 ::1", CidrError::DifferentFamilies),
            ("分解 192.0.2.1 至", CidrError::Incomplete),
            ("分解 192.0.2.1 至 192.0.2.300", CidrError::Address),
        ] {
            assert_eq!(calculate_cidr(input), Err(expected), "{input}");
        }
    }

    #[test]
    fn computes_address_ranges_and_membership_for_both_ip_versions() {
        let ipv4 =
            "网段 192.0.2.128/26\n掩码 255.255.255.192\n范围 192.0.2.128 至 192.0.2.191\n地址数 64";
        for query in ["192.0.2.137/26", "192.0.2.137 / 255.255.255.192"] {
            assert_eq!(calculate_cidr(query).as_deref(), Ok(ipv4));
        }
        assert_eq!(
            calculate_cidr("2001:db8::1234/64").as_deref(),
            Ok("网段 2001:db8::/64\n范围 2001:db8:: 至 2001:db8::ffff:ffff:ffff:ffff\n地址数 18446744073709551616")
        );
        assert_eq!(
            calculate_cidr("192.0.2.1/0.0.0.0").as_deref(),
            Ok("网段 0.0.0.0/0\n掩码 0.0.0.0\n范围 0.0.0.0 至 255.255.255.255\n地址数 4294967296")
        );
        let single_ipv4 = calculate_cidr("192.0.2.1/255.255.255.255").unwrap();
        assert!(single_ipv4.ends_with("范围 192.0.2.1 至 192.0.2.1\n地址数 1"));
        let entire_ipv6_space = calculate_cidr("::1/0").unwrap();
        assert!(entire_ipv6_space.ends_with("地址数 340282366920938463463374607431768211456"));
        for (query, expected) in [
            (
                "192.0.2.137/26 包含 192.0.2.191",
                "192.0.2.191 属于 192.0.2.128/26",
            ),
            (
                "192.0.2.137/26 包含 192.0.2.192",
                "192.0.2.192 不属于 192.0.2.128/26",
            ),
            (
                "2001:db8::/64 CONTAINS 2001:db8::cafe",
                "2001:db8::cafe 属于 2001:db8::/64",
            ),
            ("::1/128 包含 ::", ":: 不属于 ::1/128"),
        ] {
            assert_eq!(calculate_cidr(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("192.0.2.1/", CidrError::Incomplete),
            ("192.0.2.1/33", CidrError::Prefix),
            ("::1/129", CidrError::Prefix),
            ("192.0.2.1/255.0.255.0", CidrError::Prefix),
            ("192.0.2.256/24", CidrError::Address),
            ("192.0.2.0/24 包含 ::1", CidrError::DifferentFamilies),
        ] {
            assert_eq!(calculate_cidr(query), Err(error), "{query}");
        }
    }
}
