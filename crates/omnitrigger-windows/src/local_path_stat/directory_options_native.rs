// 本文件由directory.rs的既有native模块include；沿用同一目录校验与句柄所有权。
use windows::Win32::{
    Foundation::{FILETIME, SYSTEMTIME},
    Storage::FileSystem::{FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW},
    System::Time::{
        FileTimeToSystemTime, GetDynamicTimeZoneInformation, SystemTimeToTzSpecificLocalTimeEx,
        DYNAMIC_TIME_ZONE_INFORMATION, TIME_ZONE_ID_INVALID,
    },
};

struct FindHandle(HANDLE);
impl Drop for FindHandle {
    fn drop(&mut self) {
        let _ = unsafe { FindClose(self.0) };
    }
}

fn file_time_ticks(file_time: FILETIME) -> u64 {
    (u64::from(file_time.dwHighDateTime) << 32) | u64::from(file_time.dwLowDateTime)
}

struct LocalClock {
    zone: DYNAMIC_TIME_ZONE_INFORMATION,
    key: String,
}
impl LocalClock {
    fn capture() -> Result<Self, LocalStatError> {
        let mut zone = DYNAMIC_TIME_ZONE_INFORMATION::default();
        if unsafe { GetDynamicTimeZoneInformation(&mut zone) } == TIME_ZONE_ID_INVALID {
            return Err(directory_fault("fs_error"));
        }
        let length = zone
            .TimeZoneKeyName
            .iter()
            .position(|u| *u == 0)
            .unwrap_or(zone.TimeZoneKeyName.len());
        let key = String::from_utf16(&zone.TimeZoneKeyName[..length])
            .map_err(|_| directory_fault("fs_error"))?;
        Ok(Self { zone, key })
    }
    fn local_ticks(&self, raw: u64) -> Result<u64, LocalStatError> {
        // DateTime.FromFileTimeUtc有效上界；不以截断/饱和造一个可排序的假时间。
        if raw > 2_650_467_743_999_999_999 {
            return Err(directory_fault("fs_error"));
        }
        let file = FILETIME {
            dwLowDateTime: raw as u32,
            dwHighDateTime: (raw >> 32) as u32,
        };
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        unsafe { FileTimeToSystemTime(&file, &mut utc) }.map_err(code)?;
        unsafe { SystemTimeToTzSpecificLocalTimeEx(Some(&self.zone), &utc, &mut local) }
            .map_err(code)?;
        super::options::datetime_ticks(
            [
                local.wYear,
                local.wMonth,
                local.wDay,
                local.wHour,
                local.wMinute,
                local.wSecond,
                local.wMilliseconds,
            ],
            raw % 10_000,
        )
    }
    fn metadata(
        &self,
        data: &WIN32_FIND_DATAW,
        ordinal: usize,
    ) -> Result<LocalDirectoryMetadata, LocalStatError> {
        let creation = file_time_ticks(data.ftCreationTime);
        let access = file_time_ticks(data.ftLastAccessTime);
        let write = file_time_ticks(data.ftLastWriteTime);
        Ok(LocalDirectoryMetadata {
            ordinal,
            attributes: data.dwFileAttributes,
            creation_utc_100ns: creation,
            access_utc_100ns: access,
            write_utc_100ns: write,
            creation_local_ticks: self.local_ticks(creation)?,
            access_local_ticks: self.local_ticks(access)?,
            write_local_ticks: self.local_ticks(write)?,
            time_zone_key: self.key.clone(),
        })
    }
}

pub(super) fn read_options(
    path: &str,
    options: &LocalDirectoryOptions,
    include_file_sizes: bool,
    check: &impl Fn() -> Result<(), LocalStatError>,
) -> Result<Vec<LocalDirectoryEntry>, LocalStatError> {
    require_directory(stat_local_path(path, check)?)?;
    let handle = system_step(check, || open(path))??;
    let original = system_step(check, || info(&handle))??;
    verify_opened_path(handle.0, path, check)?;
    require_directory(stat_local_path(path, check)?)?;
    let clock = if options.metadata {
        Some(system_step(check, LocalClock::capture)??)
    } else {
        None
    };
    let pattern = options
        .search_pattern
        .as_deref()
        .map(super::options::normalize_pattern)
        .transpose()?
        .unwrap_or_else(|| "*".into());
    let normalized = path.replace('/', "\\");
    let search = format!("\\\\?\\{}\\{}", normalized.trim_end_matches('\\'), pattern);
    if search.encode_utf16().count() > super::super::MAX_PATH_UNITS {
        return Err(directory_fault("invalid_input"));
    }
    let wide: Vec<u16> = search.encode_utf16().chain(Some(0)).collect();
    let mut data = WIN32_FIND_DATAW::default();
    let first = system_step(check, || {
        unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut data) }.map(FindHandle)
    });
    let mut collector = Collector::new(path, options.max_entries);
    let mut sort_keys = Vec::new();
    // 成功句柄在system_step的闭包内建立RAII，迟到取消时也会释放。
    let find = match first? {
        Ok(handle) => Some(handle),
        Err(error) if matches!(error.code().0 as u32 & 0xffff, 2 | 18) => None,
        Err(error) => return Err(code(error)),
    };
    if let Some(find) = find {
        loop {
            checkpoint(check)?;
            let end = data
                .cFileName
                .iter()
                .position(|u| *u == 0)
                .ok_or_else(|| directory_fault("fs_error"))?;
            let name = String::from_utf16(&data.cFileName[..end])
                .map_err(|_| directory_fault("fs_error"))?;
            let before = collector.entries.len();
            let directory = data.dwFileAttributes & 0x10 != 0;
            let included = match options.entry_type {
                DirectoryEntryType::All => true,
                DirectoryEntryType::File => !directory,
                DirectoryEntryType::Directory => directory,
            };
            if included {
                collector.add(name, data.dwFileAttributes)?;
            }
            if collector.entries.len() > before {
                let length =
                    (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow);
                match options.order {
                    DirectoryOrder::ModifiedDescending | DirectoryOrder::ModifiedAscending => {
                        let modified = file_time_ticks(data.ftLastWriteTime);
                        sort_keys.push((modified != 0).then_some(modified));
                    }
                    DirectoryOrder::CreatedDescending | DirectoryOrder::CreatedAscending => {
                        let created = file_time_ticks(data.ftCreationTime);
                        sort_keys.push((created != 0).then_some(created));
                    }
                    DirectoryOrder::SizeDescending | DirectoryOrder::SizeAscending => {
                        sort_keys.push((!directory).then_some(length));
                    }
                    DirectoryOrder::Name | DirectoryOrder::Native => {}
                }
                if include_file_sizes
                    || matches!(
                        options.order,
                        DirectoryOrder::SizeDescending | DirectoryOrder::SizeAscending
                    )
                {
                    let entry = collector.entries.last_mut().expect("entry added");
                    entry.size_bytes = (entry.kind == "file").then_some(length);
                }
                if let Some(clock) = &clock {
                    let metadata = system_step(check, || clock.metadata(&data, before))??;
                    collector.json_bound += 512 + json_string_bytes(&metadata.time_zone_key);
                    if collector.json_bound > MAX_DIRECTORY_BYTES {
                        return Err(directory_fault("directory_too_large"));
                    }
                    collector.entries.last_mut().expect("entry added").metadata = Some(metadata);
                }
            }
            match system_step(check, || unsafe { FindNextFileW(find.0, &mut data) })? {
                Ok(()) => {}
                Err(error) if error.code().0 as u32 & 0xffff == 18 => break,
                Err(error) => return Err(code(error)),
            }
        }
    }
    verify_opened_path(handle.0, path, check)?;
    let current = system_step(check, || info(&handle))??;
    let reopened = system_step(check, || open(path))??;
    let current_path = system_step(check, || info(&reopened))??;
    if identity(&original) != identity(&current)
        || identity(&original) != identity(&current_path)
        || original.ftLastWriteTime != current.ftLastWriteTime
    {
        return Err(directory_fault("directory_changed"));
    }
    require_directory(stat_local_path(path, check)?)?;
    if let Some(clock) = clock {
        if system_step(check, LocalClock::capture)??.zone != clock.zone {
            return Err(directory_fault("directory_changed"));
        }
    }
    checkpoint(check)?;
    Ok(match options.order {
        DirectoryOrder::Name => collector.finish(),
        DirectoryOrder::Native => collector.entries,
        DirectoryOrder::ModifiedDescending
        | DirectoryOrder::ModifiedAscending
        | DirectoryOrder::CreatedDescending
        | DirectoryOrder::CreatedAscending
        | DirectoryOrder::SizeDescending
        | DirectoryOrder::SizeAscending => {
            let ascending = matches!(
                options.order,
                DirectoryOrder::ModifiedAscending
                    | DirectoryOrder::CreatedAscending
                    | DirectoryOrder::SizeAscending
            );
            let mut entries: Vec<_> = sort_keys.into_iter().zip(collector.entries).collect();
            entries.sort_by(|(left_key, left), (right_key, right)| {
                let by_value = if ascending {
                    (left_key.is_none(), left_key).cmp(&(right_key.is_none(), right_key))
                } else {
                    right_key.cmp(left_key)
                };
                by_value.then_with(|| left.name.cmp(&right.name))
            });
            entries.into_iter().map(|(_, entry)| entry).collect()
        }
    })
}
