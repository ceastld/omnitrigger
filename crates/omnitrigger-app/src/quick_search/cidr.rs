use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

use num_bigint::BigUint;

use super::{
    calculator::normalize, csv_tools::append_bounded_csv_row, operators::json_argument_and_tail,
};

const MAX_SUBNETS: u32 = 128;
const MAX_SET_NETWORKS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CidrError {
    Incomplete,
    Address,
    Prefix,
    DifferentFamilies,
    ReversedRange,
    ChildPrefix,
    OutsideNetwork,
    TooManySubnets,
    LookupArguments,
    ReportTooLarge,
    NetworkSetArguments,
    NetworkSetTooLarge,
}

impl CidrError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 IP 地址和前缀长度，例如 192.0.2.137/26",
            Self::Address => "请输入有效的 IPv4 或 IPv6 地址",
            Self::Prefix => "请输入有效的前缀长度或连续的 IPv4 掩码",
            Self::DifferentFamilies => "地址和网段须使用相同的 IP 版本",
            Self::ReversedRange => "起始地址不得大于结束地址",
            Self::ChildPrefix => "子网的前缀长度须大于或等于原网段",
            Self::OutsideNetwork => "要排除的网段须完全位于原网段内",
            Self::TooManySubnets => "每次最多列出 128 个子网，请减小前缀差",
            Self::LookupArguments => {
                "用 JSON 数组填写候选网段，再用逗号、空格或换行分隔要查找的 IP 地址"
            }
            Self::ReportTooLarge => "网段归属表超过 CSV 输出上限",
            Self::NetworkSetArguments => {
                "填写两组网段，例如 [[\"192.0.2.0/24\"],[\"192.0.2.128/25\"]]"
            }
            Self::NetworkSetTooLarge => "交集或差集最多输入和输出各 4096 个网段",
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

pub(super) fn cidr_csv_report(query: &str) -> Option<Result<String, CidrError>> {
    let (operation, data) = query.split_once(char::is_whitespace)?;
    match operation.to_ascii_lowercase().as_str() {
        "归属csv" | "longest-prefix-csv" => Some(longest_prefix_report(&normalize(data))),
        _ => None,
    }
}

fn longest_prefix_report(input: &str) -> Result<String, CidrError> {
    let (networks, addresses) =
        json_argument_and_tail::<Vec<String>>(input).ok_or(CidrError::LookupArguments)?;
    if networks.is_empty() {
        return Err(CidrError::LookupArguments);
    }
    let mut candidates = HashSet::new();
    for network in networks {
        let (address, prefix) = parse_network(&network)?;
        let width = address_width(address);
        let first = bits(address) & !host_mask(width - prefix);
        candidates.insert((width, prefix, first));
    }
    const HEADERS: &str = "地址,匹配网段";
    let mut output = String::from(HEADERS);
    for address in addresses
        .split(|character: char| character == ',' || character.is_whitespace())
        .filter(|address| !address.is_empty())
    {
        let address = address.parse::<IpAddr>().map_err(|_| CidrError::Address)?;
        let width = address_width(address);
        let address_bits = bits(address);
        let network = (0..=width)
            .rev()
            .find_map(|prefix| {
                let first = address_bits & !host_mask(width - prefix);
                candidates
                    .contains(&(width, prefix, first))
                    .then(|| format!("{}/{prefix}", as_address(first, width)))
            })
            .unwrap_or_default();
        let address = address.to_string();
        append_bounded_csv_row(
            &mut output,
            [address.as_str(), network.as_str()].into_iter(),
            CidrError::ReportTooLarge,
        )?;
    }
    if output.len() == HEADERS.len() {
        return Err(CidrError::LookupArguments);
    }
    Ok(output)
}

pub(super) fn calculate_cidr(query: &str) -> Result<String, CidrError> {
    if let Some(report) = cidr_csv_report(query) {
        return report;
    }
    let input = normalize(query).replace(char::is_whitespace, " ");
    if let Some((operation, range)) = input.split_once(' ') {
        match operation.to_ascii_lowercase().as_str() {
            "覆盖" | "cover" => return range_to_cidr(range, true),
            "分解" | "range-to-cidr" => return range_to_cidr(range, false),
            "子网" | "subnets" => return split_subnets(range),
            "排除" | "exclude" => return exclude_network(range),
            "合并" | "collapse" => return collapse_networks(range),
            "交集" | "intersect" => {
                return compare_network_sets(range, NetworkSetOperation::Intersect)
            }
            "差集" | "subtract" => {
                return compare_network_sets(range, NetworkSetOperation::Subtract)
            }
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
    let (address, prefix) = parse_network(network)?;
    let width = address_width(address);
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

fn parse_network(input: &str) -> Result<(IpAddr, u32), CidrError> {
    let (address, prefix) = input.split_once('/').ok_or(CidrError::Incomplete)?;
    let address = address.trim();
    if address.is_empty() {
        return Err(CidrError::Incomplete);
    }
    let address = address.parse::<IpAddr>().map_err(|_| CidrError::Address)?;
    let prefix = prefix_length(prefix, address_width(address))?;
    Ok((address, prefix))
}

fn split_subnets(input: &str) -> Result<String, CidrError> {
    let (network, target) = input
        .split_once(['到', '至'])
        .or_else(|| {
            let index = input.to_ascii_lowercase().find(" to ")?;
            Some((&input[..index], &input[index + " to ".len()..]))
        })
        .ok_or(CidrError::Incomplete)?;
    let (address, prefix) = parse_network(network)?;
    let width = address_width(address);
    let target = target.trim();
    let child = prefix_length(target.strip_prefix('/').unwrap_or(target), width)?;
    let extra = child.checked_sub(prefix).ok_or(CidrError::ChildPrefix)?;
    let count = 1u32
        .checked_shl(extra)
        .filter(|&count| count <= MAX_SUBNETS)
        .ok_or(CidrError::TooManySubnets)?;
    let first = bits(address) & !host_mask(width - prefix);
    let mut subnets = Vec::with_capacity(count as usize);
    for index in 0..count {
        // IPv6 /0 的同前缀查询只有首个网段，因此偏移为零。
        let offset = u128::from(index).checked_shl(width - child).unwrap_or(0);
        subnets.push(format!("{}/{child}", as_address(first + offset, width)));
    }
    Ok(subnets.join("\n"))
}

fn exclude_network(input: &str) -> Result<String, CidrError> {
    let (parent, excluded) = input.split_once(',').ok_or(CidrError::Incomplete)?;
    let (parent, parent_prefix) = parse_network(parent)?;
    let (excluded, excluded_prefix) = parse_network(excluded)?;
    let width = address_width(parent);
    if address_width(excluded) != width {
        return Err(CidrError::DifferentFamilies);
    }
    let parent_mask = !host_mask(width - parent_prefix);
    if excluded_prefix < parent_prefix || bits(parent) & parent_mask != bits(excluded) & parent_mask
    {
        return Err(CidrError::OutsideNetwork);
    }
    if excluded_prefix == parent_prefix {
        return Ok("无剩余网段".into());
    }

    // 沿排除网段的前缀逐位下降，每一层保留不含该网段的另一半。
    let mut networks = Vec::with_capacity((excluded_prefix - parent_prefix) as usize);
    for prefix in parent_prefix + 1..=excluded_prefix {
        let host_bits = width - prefix;
        let start = (bits(excluded) & !host_mask(host_bits)) ^ (1u128 << host_bits);
        networks.push((start, prefix));
    }
    networks.sort_unstable();
    Ok(networks
        .into_iter()
        .map(|(start, prefix)| format!("{}/{prefix}", as_address(start, width)))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn collapse_networks(input: &str) -> Result<String, CidrError> {
    let intervals = merge_network_intervals(
        input
            .split(|character: char| character == ',' || character.is_whitespace())
            .filter(|network| !network.is_empty()),
    )?;
    if intervals.is_empty() {
        return Err(CidrError::Incomplete);
    }
    let mut networks = Vec::new();
    for NetworkInterval { width, first, last } in intervals {
        append_network_range(&mut networks, width, first, last);
    }
    Ok(networks.join("\n"))
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NetworkInterval {
    width: u32,
    first: u128,
    last: u128,
}

fn merge_network_intervals<'a>(
    networks: impl Iterator<Item = &'a str>,
) -> Result<Vec<NetworkInterval>, CidrError> {
    let mut intervals = Vec::new();
    for network in networks {
        let (address, prefix) = parse_network(network)?;
        let width = address_width(address);
        let mask = host_mask(width - prefix);
        intervals.push(NetworkInterval {
            width,
            first: bits(address) & !mask,
            last: bits(address) | mask,
        });
    }
    intervals.sort_unstable();
    let mut merged: Vec<NetworkInterval> = Vec::new();
    for interval in intervals {
        if let Some(previous) = merged.last_mut().filter(|previous| {
            previous.width == interval.width && interval.first <= previous.last.saturating_add(1)
        }) {
            previous.last = previous.last.max(interval.last);
        } else {
            merged.push(interval);
        }
    }
    Ok(merged)
}

enum NetworkSetOperation {
    Intersect,
    Subtract,
}

fn compare_network_sets(input: &str, operation: NetworkSetOperation) -> Result<String, CidrError> {
    let [left, right]: [Vec<String>; 2] =
        serde_json::from_str(input).map_err(|_| CidrError::NetworkSetArguments)?;
    if left.len() + right.len() > MAX_SET_NETWORKS {
        return Err(CidrError::NetworkSetTooLarge);
    }
    let left = merge_network_intervals(left.iter().map(String::as_str))?;
    let right = merge_network_intervals(right.iter().map(String::as_str))?;
    let intervals = match operation {
        NetworkSetOperation::Intersect => intersect_network_intervals(&left, &right),
        NetworkSetOperation::Subtract => subtract_network_intervals(left, &right),
    };
    let mut networks = Vec::new();
    for NetworkInterval { width, first, last } in intervals {
        append_network_range(&mut networks, width, first, last);
        if networks.len() > MAX_SET_NETWORKS {
            return Err(CidrError::NetworkSetTooLarge);
        }
    }
    if networks.is_empty() {
        return Ok(match operation {
            NetworkSetOperation::Intersect => "无重叠网段",
            NetworkSetOperation::Subtract => "无剩余网段",
        }
        .to_owned());
    }
    Ok(networks.join("\n"))
}

fn intersect_network_intervals(
    left: &[NetworkInterval],
    right: &[NetworkInterval],
) -> Vec<NetworkInterval> {
    let (mut left_index, mut right_index) = (0, 0);
    let mut result = Vec::new();
    while let (Some(a), Some(b)) = (left.get(left_index), right.get(right_index)) {
        let (width, first) = (a.width, a.first).max((b.width, b.first));
        let (last_width, last) = (a.width, a.last).min((b.width, b.last));
        if width == last_width && first <= last {
            result.push(NetworkInterval { width, first, last });
        }
        if (a.width, a.last) <= (b.width, b.last) {
            left_index += 1;
        } else {
            right_index += 1;
        }
    }
    result
}

fn subtract_network_intervals(
    left: Vec<NetworkInterval>,
    right: &[NetworkInterval],
) -> Vec<NetworkInterval> {
    let mut right = right.iter().peekable();
    let mut result = Vec::new();
    for mut remaining in left {
        loop {
            let Some(excluded) = right.peek() else {
                result.push(remaining);
                break;
            };
            if (excluded.width, excluded.last) < (remaining.width, remaining.first) {
                right.next();
                continue;
            }
            if (excluded.width, excluded.first) > (remaining.width, remaining.last) {
                result.push(remaining);
                break;
            }
            if remaining.first < excluded.first {
                result.push(NetworkInterval {
                    last: excluded.first - 1,
                    ..remaining
                });
            }
            if excluded.last >= remaining.last {
                break;
            }
            remaining.first = excluded.last + 1;
            right.next();
        }
    }
    result
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
    let (cursor, last) = (bits(first), bits(last));
    if cursor > last {
        return Err(CidrError::ReversedRange);
    }

    if cover_only {
        let host_bits = 128 - (cursor ^ last).leading_zeros();
        let start = as_address(cursor & !host_mask(host_bits), width);
        return Ok(format!("{start}/{}", width - host_bits));
    }

    let mut networks = Vec::new();
    append_network_range(&mut networks, width, cursor, last);
    Ok(networks.join("\n"))
}

fn append_network_range(networks: &mut Vec<String>, width: u32, mut cursor: u128, last: u128) {
    loop {
        // 选取起点对齐且完全位于剩余区间中的最大网段。
        let remaining_bits = (last - cursor).checked_add(1).map_or(128, u128::ilog2);
        let host_bits = cursor.trailing_zeros().min(remaining_bits).min(width);
        let start = as_address(cursor, width);
        networks.push(format!("{start}/{}", width - host_bits));
        let block_last = cursor | host_mask(host_bits);
        if block_last == last {
            break;
        }
        cursor = block_last + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_overlapping_and_adjacent_networks_without_covering_gaps() {
        for (query, expected) in [
            (
                "合并 2001:db8:0:1::/64,192.0.2.192/26,192.0.2.128/25,192.0.2.9/25,2001:db8::/64",
                "192.0.2.0/24\n2001:db8::/63",
            ),
            (
                "collapse 192.0.2.0/26,192.0.2.128/26,192.0.2.192/27",
                "192.0.2.0/26\n192.0.2.128/26\n192.0.2.192/27",
            ),
            (
                "合并 ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128,::/0,0.0.0.0/0",
                "0.0.0.0/0\n::/0",
            ),
        ] {
            assert_eq!(calculate_cidr(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(calculate_cidr("合并 192.0.2.1/33"), Err(CidrError::Prefix));
        assert_eq!(calculate_cidr("合并 ,"), Err(CidrError::Incomplete));
    }

    #[test]
    fn compares_sets_of_networks_without_enumerating_addresses() {
        const LISTS: &str = r#"[["192.0.2.0/24","2001:db8::/63"],["192.0.2.0/26","192.0.2.128/25","2001:db8:0:1::/64"]]"#;
        for (query, expected) in [
            (
                format!("交集 {LISTS}"),
                "192.0.2.0/26\n192.0.2.128/25\n2001:db8:0:1::/64",
            ),
            (format!("差集 {LISTS}"), "192.0.2.64/26\n2001:db8::/64"),
            (
                r#"intersect [["0.0.0.0/0"],["::/0"]]"#.to_owned(),
                "无重叠网段",
            ),
            (
                r#"subtract [["192.0.2.0/26","192.0.2.128/26","192.0.3.0/24"],["192.0.2.0/24"]]"#.to_owned(),
                "192.0.3.0/24",
            ),
            (
                r#"差集 [["255.255.255.255/32","ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128"],["::/0"]]"#.to_owned(),
                "255.255.255.255/32",
            ),
            (r#"差集 [["::/0"],[]]"#.to_owned(), "::/0"),
            (r#"差集 [["::/0"],["::/0"]]"#.to_owned(), "无剩余网段"),
            (r#"交集 [[],["::/0"]]"#.to_owned(), "无重叠网段"),
        ] {
            assert_eq!(calculate_cidr(&query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_cidr("交集 []"),
            Err(CidrError::NetworkSetArguments)
        );
        assert_eq!(
            calculate_cidr(r#"差集 [["::/129"],[]]"#),
            Err(CidrError::Prefix)
        );
    }

    #[test]
    fn rejects_network_sets_with_too_many_inputs_or_output_prefixes() {
        let all = vec!["::/0".to_owned()];
        let massive = vec!["::1/128".to_owned(); MAX_SET_NETWORKS];
        let sparse: Vec<_> = (0..48).map(|prefix| format!("{prefix:x}::1/128")).collect();
        for excluded in [massive, sparse] {
            let query = serde_json::to_string(&[&all, &excluded]).unwrap();
            assert_eq!(
                calculate_cidr(&format!("差集 {query}")),
                Err(CidrError::NetworkSetTooLarge)
            );
        }
    }

    #[test]
    fn excludes_a_contained_subnet_without_enumerating_addresses() {
        for (query, expected) in [
            (
                "排除 192.0.2.0/28，192.0.2.1/32",
                "192.0.2.0/32\n192.0.2.2/31\n192.0.2.4/30\n192.0.2.8/29",
            ),
            ("exclude 2001:db8::3/126, 2001:db8::2/127", "2001:db8::/127"),
            (
                "排除 192.0.2.1/255.255.255.0, 192.0.2.128/25",
                "192.0.2.0/25",
            ),
            ("排除 ::1/0, ffff::/0", "无剩余网段"),
        ] {
            assert_eq!(calculate_cidr(query).as_deref(), Ok(expected), "{query}");
        }
        let largest = calculate_cidr("排除 ::/0, ::/128").unwrap();
        assert_eq!(largest.lines().count(), 128);
        assert_eq!(largest.lines().next(), Some("::1/128"));
        assert_eq!(largest.lines().last(), Some("8000::/1"));
        for (query, expected) in [
            ("排除 ::/0, 192.0.2.0/24", CidrError::DifferentFamilies),
            ("排除 192.0.2.0/24, 192.0.3.0/25", CidrError::OutsideNetwork),
            ("排除 192.0.2.0/25, 192.0.2.0/24", CidrError::OutsideNetwork),
            ("排除 192.0.2.0/24,", CidrError::Incomplete),
        ] {
            assert_eq!(calculate_cidr(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn splits_ipv4_and_ipv6_prefixes_into_ordered_subnets_with_bounded_output() {
        for (query, expected) in [
            (
                "子网 192.0.2.137/26 到 /28",
                "192.0.2.128/28\n192.0.2.144/28\n192.0.2.160/28\n192.0.2.176/28",
            ),
            ("subnets 2001:db8::3/126 TO 127", "2001:db8::/127\n2001:db8::2/127"),
            ("子网 ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/127 至 /128", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe/128\nffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128"),
            ("子网 ::/0 到 /0", "::/0"),
        ] {
            assert_eq!(calculate_cidr(query).as_deref(), Ok(expected));
        }
        for (query, expected) in [
            ("子网 192.0.2.137/26 到 /25", CidrError::ChildPrefix),
            ("子网 192.0.2.137/26 到 /33", CidrError::Prefix),
            ("子网 ::/0 到 /8", CidrError::TooManySubnets),
            ("子网 ::/0 到 /128", CidrError::TooManySubnets),
        ] {
            assert_eq!(calculate_cidr(query), Err(expected));
        }
    }

    #[test]
    fn finds_longest_matching_prefixes_with_mixed_address_families() {
        let query = "归属CSV [\"192.0.2.0/24\",\"192.0.2.150/25\",\"2001:db8::/32\",\"::1/128\"] 192.0.2.190,192.0.2.1\r\n192.0.3.1 2001:db8::7 ::1";
        let expected = concat!(
            "地址,匹配网段\r\n",
            "192.0.2.190,192.0.2.128/25\r\n",
            "192.0.2.1,192.0.2.0/24\r\n",
            "192.0.3.1,\"\"\r\n",
            "2001:db8::7,2001:db8::/32\r\n",
            "::1,::1/128",
        );
        assert_eq!(calculate_cidr(query).as_deref(), Ok(expected));
        assert_eq!(cidr_csv_report(query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            calculate_cidr(r#"longest-prefix-csv ["0.0.0.0/0"] 198.51.100.4"#).as_deref(),
            Ok("地址,匹配网段\r\n198.51.100.4,0.0.0.0/0")
        );
        assert_eq!(
            calculate_cidr("归属CSV [] 192.0.2.1"),
            Err(CidrError::LookupArguments)
        );
    }

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
