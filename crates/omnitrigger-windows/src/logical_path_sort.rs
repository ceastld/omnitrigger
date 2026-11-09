//! 对已给定完整字符串排序；不读取路径、文件系统或 Explorer。
//!
//! 比较使用当前 Windows 的 StrCmpLogicalW。相等项的移动顺序移植自
//! dotnet/runtime v10.0.0 Array.Sort(Array, IComparer) 的 SorterObjectArray，
//! 不以 Rust 稳定排序或额外的字符串比较打破平局。
//!
//! 排序算法改写自 .NET Foundation and Contributors（MIT）：
//! https://github.com/dotnet/runtime/blob/v10.0.0/src/libraries/System.Private.CoreLib/src/System/Array.cs
//! 完整许可见 THIRD_PARTY_NOTICES.md。

use std::cmp::Ordering;
use windows::{core::PCWSTR, Win32::UI::Shell::StrCmpLogicalW};

/// 保留输入及重复项的完整内容，返回排序后的副本。
///
/// UTF-16 中的内嵌 NUL 和原 Win32 调用一样终止比较，但不会截断返回的字符串。
/// 同值项顺序绑定上述 .NET 10 算法；系统比较结果随 Windows 版本而定。
pub fn sort_logical_paths(paths: &[String]) -> Vec<String> {
    let wide: Vec<Vec<u16>> = paths
        .iter()
        .map(|path| path.encode_utf16().chain(Some(0)).collect())
        .collect();
    let mut order: Vec<usize> = (0..paths.len()).collect();
    sort_dotnet10_by(&mut order, &|left, right| {
        // 两个 UTF-16 缓冲区在整个排序期间存活，且均以 NUL 结束。
        unsafe { StrCmpLogicalW(PCWSTR(wide[left].as_ptr()), PCWSTR(wide[right].as_ptr())) }.cmp(&0)
    });
    order
        .into_iter()
        .map(|index| paths[index].clone())
        .collect()
}

fn sort_dotnet10_by(keys: &mut [usize], compare: &impl Fn(usize, usize) -> Ordering) {
    let length = keys.len();
    if length > 1 {
        let depth = 2 * (usize::BITS - length.leading_zeros()) as usize;
        intro_sort(keys, 0, length - 1, depth, compare);
    }
}

fn swap_if_greater(
    keys: &mut [usize],
    left: usize,
    right: usize,
    compare: &impl Fn(usize, usize) -> Ordering,
) {
    if left != right && compare(keys[left], keys[right]) == Ordering::Greater {
        keys.swap(left, right);
    }
}

fn intro_sort(
    keys: &mut [usize],
    lo: usize,
    mut hi: usize,
    mut depth: usize,
    compare: &impl Fn(usize, usize) -> Ordering,
) {
    while hi > lo {
        let size = hi - lo + 1;
        if size <= 16 {
            match size {
                2 => swap_if_greater(keys, lo, hi, compare),
                3 => {
                    swap_if_greater(keys, lo, hi - 1, compare);
                    swap_if_greater(keys, lo, hi, compare);
                    swap_if_greater(keys, hi - 1, hi, compare);
                }
                _ => insertion_sort(keys, lo, hi, compare),
            }
            return;
        }
        if depth == 0 {
            heap_sort(keys, lo, hi, compare);
            return;
        }
        depth -= 1;
        let pivot = partition(keys, lo, hi, compare);
        intro_sort(keys, pivot + 1, hi, depth, compare);
        hi = pivot - 1;
    }
}

fn partition(
    keys: &mut [usize],
    lo: usize,
    hi: usize,
    compare: &impl Fn(usize, usize) -> Ordering,
) -> usize {
    let mid = lo + (hi - lo) / 2;
    swap_if_greater(keys, lo, mid, compare);
    swap_if_greater(keys, lo, hi, compare);
    swap_if_greater(keys, mid, hi, compare);
    let pivot = keys[mid];
    keys.swap(mid, hi - 1);
    let mut left = lo;
    let mut right = hi - 1;
    while left < right {
        loop {
            left += 1;
            if compare(keys[left], pivot) != Ordering::Less {
                break;
            }
        }
        loop {
            right -= 1;
            if compare(pivot, keys[right]) != Ordering::Less {
                break;
            }
        }
        if left >= right {
            break;
        }
        keys.swap(left, right);
    }
    if left != hi - 1 {
        keys.swap(left, hi - 1);
    }
    left
}

fn heap_sort(
    keys: &mut [usize],
    lo: usize,
    hi: usize,
    compare: &impl Fn(usize, usize) -> Ordering,
) {
    let size = hi - lo + 1;
    for index in (1..=size / 2).rev() {
        down_heap(keys, index, size, lo, compare);
    }
    for index in (2..=size).rev() {
        keys.swap(lo, lo + index - 1);
        down_heap(keys, 1, index - 1, lo, compare);
    }
}

fn down_heap(
    keys: &mut [usize],
    mut index: usize,
    size: usize,
    lo: usize,
    compare: &impl Fn(usize, usize) -> Ordering,
) {
    let value = keys[lo + index - 1];
    while index <= size / 2 {
        let mut child = 2 * index;
        if child < size && compare(keys[lo + child - 1], keys[lo + child]) == Ordering::Less {
            child += 1;
        }
        if compare(value, keys[lo + child - 1]) != Ordering::Less {
            break;
        }
        keys[lo + index - 1] = keys[lo + child - 1];
        index = child;
    }
    keys[lo + index - 1] = value;
}

fn insertion_sort(
    keys: &mut [usize],
    lo: usize,
    hi: usize,
    compare: &impl Fn(usize, usize) -> Ordering,
) {
    for index in lo..hi {
        let value = keys[index + 1];
        let mut destination = index + 1;
        while destination > lo && compare(value, keys[destination - 1]) == Ordering::Less {
            keys[destination] = keys[destination - 1];
            destination -= 1;
        }
        keys[destination] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn logical_paths_compare_full_input_before_file_name() {
        let input = strings(&[r"S:\b2\file1.txt", r"S:\b1\file20.txt", r"S:\b1\file3.txt"]);
        let original = input.clone();
        assert_eq!(
            sort_logical_paths(&input),
            strings(&[r"S:\b1\file3.txt", r"S:\b1\file20.txt", r"S:\b2\file1.txt",])
        );
        assert_eq!(input, original);
    }

    #[test]
    fn empty_singleton_and_duplicate_contents_are_preserved() {
        assert!(sort_logical_paths(&[]).is_empty());
        let singleton = strings(&["  S:/不存在/🦀\0suffix  "]);
        assert_eq!(sort_logical_paths(&singleton), singleton);
        let mut input = strings(&["S:\\a\0tail2", "S:\\A\0tail1", "S:\\a\0tail2", "中文🦀"]);
        let mut sorted = sort_logical_paths(&input);
        // 这里只按字节比较多重集合，不用该次序作为逻辑排序结果。
        input.sort();
        sorted.sort();
        assert_eq!(sorted, input);
    }

    #[test]
    #[ignore = "需先生成同机 .NET Framework 4.7.2 / StrCmpLogicalW 合成字符串 oracle"]
    fn framework472_oracle_matches_exact_sequence() {
        let path =
            std::env::var_os("OMNITRIGGER_LOGICAL_SORT_ORACLE").expect("须指定本轮 oracle.tsv");
        let text = std::fs::read_to_string(path).unwrap();
        let mut lines = text.lines();
        let header: Vec<_> = lines.next().unwrap().split('\t').collect();
        assert_eq!(header.len(), 8);
        assert_eq!(header[0], "RUNTIME");
        assert!(header[1].starts_with("4."));
        assert_eq!(header[2], "mscorlib");
        assert_eq!(header[4], ".NETFramework,Version=v4.7.2");
        assert_eq!(header[5], ".NETFramework,Version=v4.7.2");
        assert!(matches!(header[6], "32" | "64"));
        let mut count = 0;
        for line in lines {
            let fields: Vec<_> = line.split('\t').collect();
            if fields[0] == "EQUAL" {
                assert_eq!(fields[2], "0", "{}", fields[1]);
                continue;
            }
            assert_eq!(fields[0], "CASE");
            let length: usize = fields[2].parse().unwrap();
            assert_eq!(fields.len(), 3 + 2 * length);
            let decode = |encoded: &&str| {
                assert_eq!(encoded.len() % 2, 0);
                let bytes: Vec<u8> = (0..encoded.len())
                    .step_by(2)
                    .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16).unwrap())
                    .collect();
                String::from_utf8(bytes).unwrap()
            };
            let input: Vec<_> = fields[3..3 + length].iter().map(decode).collect();
            let expected: Vec<_> = fields[3 + length..].iter().map(decode).collect();
            assert_eq!(sort_logical_paths(&input), expected, "{}", fields[1]);
            if fields[1] == "ties17" || fields[1] == "ties33" {
                assert_ne!(expected, input, "oracle 必须能识别稳定排序的差异");
            }
            count += 1;
        }
        assert_eq!(count, 6, "必须完整核对六组 oracle，不能用空文件通过");
    }
}
