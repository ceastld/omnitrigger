//! 保存热键或轮盘前按原字节留档。调用方持有对应文件的配置锁。
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const RECENT: usize = 20;
const DAYS: u64 = 30;

fn edit_history_root(config: &Path) -> PathBuf {
    config.with_file_name("backups").join("edits")
}

pub(crate) fn edit_history_folder_to_open(config: &Path) -> Result<String, String> {
    let folder = std::path::absolute(edit_history_root(config)).map_err(|e| e.to_string())?;
    crate::script_runtime::check_local_path(&folder)?;
    match folder.symlink_metadata() {
        Ok(meta) if meta.is_dir() && crate::local_files::is_plain(&meta) => {
            let target = folder.to_str().ok_or("备份目录路径无法显示。")?;
            Ok(target.strip_prefix(r"\\?\").unwrap_or(target).to_owned())
        }
        Ok(_) => Err("备份路径不是普通本机目录。".into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err("本档还没有编辑备份。下次保存对已有热键或轮盘的修改后，可以在这里查看。".into())
        }
        Err(e) => Err(format!("无法打开备份目录：{e}")),
    }
}

struct Revision {
    number: u64,
    path: PathBuf,
    modified: SystemTime,
    size: u64,
}

fn utc_day(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400
}

fn filename(number: u64) -> String {
    format!("edit-{number:016}.json")
}

pub(crate) fn edit_revision_number(name: &std::ffi::OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let number = name
        .strip_prefix("edit-")?
        .strip_suffix(".json")?
        .parse()
        .ok()?;
    (number > 0 && filename(number) == name).then_some(number)
}

fn revisions(directory: &Path) -> io::Result<(u64, Vec<Revision>)> {
    let mut records = Vec::new();
    let mut highest = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(number) = edit_revision_number(&name) else {
            continue;
        };
        highest = highest.max(number);
        let path = entry.path();
        let meta = path.symlink_metadata()?;
        if meta.is_file() && crate::local_files::is_plain(&meta) {
            records.push(Revision {
                number,
                path,
                modified: meta.modified()?,
                size: meta.len(),
            });
        }
    }
    records.sort_unstable_by_key(|r| r.number);
    Ok((highest, records))
}

impl Revision {
    fn matches(&self, original: &[u8]) -> bool {
        if self.size != original.len() as u64 {
            return false;
        }
        let mut bytes = Vec::new();
        fs::File::open(&self.path)
            .and_then(|f| f.take(self.size.saturating_add(1)).read_to_end(&mut bytes))
            .is_ok()
            && bytes == original
    }
}

fn trim(records: &[Revision], today: u64) {
    let mut kept_days = BTreeSet::new();
    for (index, record) in records.iter().rev().enumerate() {
        let day = utc_day(record.modified);
        let daily =
            (today.saturating_sub(DAYS - 1)..=today).contains(&day) && kept_days.insert(day);
        if index >= RECENT
            && !daily
            && record
                .path
                .symlink_metadata()
                .is_ok_and(|meta| meta.is_file() && crate::local_files::is_plain(&meta))
        {
            // 旧备份若被打开，下次编辑时再清理；当前配置已经保存成功。
            let _ = fs::remove_file(&record.path);
        }
    }
}

pub(crate) fn save_editor_file(
    path: &Path,
    original: Option<&str>,
    contents: &[u8],
) -> Result<(), String> {
    let Some(original) = original.map(str::as_bytes) else {
        return super::atomic_write(path, contents);
    };
    if original == contents {
        return Ok(());
    }
    // 历史只随普通本机配置档维护，显式指定的其他配置路径仍沿用原保存方式。
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    let parent = absolute.parent().ok_or("配置路径不可用")?;
    if crate::script_runtime::check_local_path(parent).is_err() {
        return super::atomic_write(path, contents);
    }
    let directory = edit_history_root(path).join(path.file_name().ok_or("配置路径不可用")?);
    let backup_error = |error: io::Error| {
        format!(
            "保存前备份失败（{}），原配置已保留：{error}",
            directory.display()
        )
    };
    crate::script_runtime::check_local_path(&directory)
        .map_err(|e| format!("无法写入配置备份：{e}"))?;
    fs::create_dir_all(&directory).map_err(backup_error)?;
    let (highest, mut records) = revisions(&directory).map_err(backup_error)?;
    let now = SystemTime::now();
    // 上次已备份但目标文件被占用时，重试沿用那份原文。
    let already_saved = records.last().is_some_and(|r| r.matches(original));
    if !already_saved {
        let number = highest.checked_add(1).ok_or("配置备份编号已用尽")?;
        let next = directory.join(filename(number));
        super::atomic_create_new(&next, original)
            .map_err(|e| format!("无法保存配置备份 {}：{e}", next.display()))?;
        records.push(Revision {
            number,
            path: next,
            modified: now,
            size: original.len() as u64,
        });
    }
    super::atomic_write(path, contents)?;
    trim(&records, utc_day(now));
    Ok(())
}

pub(crate) fn edit_versions(config: &Path, name: &str) -> Result<Vec<(u64, SystemTime)>, String> {
    let directory = edit_history_root(config).join(name);
    crate::script_runtime::check_local_path(&directory)?;
    match revisions(&directory) {
        Ok((_, records)) => Ok(records
            .into_iter()
            .rev()
            .map(|r| (r.number, r.modified))
            .collect()),
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                && !directory.try_exists().map_err(|e| e.to_string())? =>
        {
            Ok(Vec::new())
        }
        Err(e) => Err(format!("无法读取 {name} 的备份列表：{e}")),
    }
}

pub(crate) fn edit_backup_file(config: &Path, name: &str, number: u64) -> Result<PathBuf, String> {
    let file = edit_history_root(config).join(name).join(filename(number));
    crate::script_runtime::check_local_path(&file)?;
    let meta = file
        .symlink_metadata()
        .map_err(|e| format!("无法读取 {name} 的第 {number} 份备份：{e}"))?;
    if !meta.is_file() || !crate::local_files::is_plain(&meta) {
        return Err(format!("{name} 的第 {number} 份备份不是普通本机文件。"));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wheels::tests::Scratch;
    use std::time::Duration;

    #[test]
    fn recent_edits_and_one_per_day_survive_thirty_day_retention() {
        let scratch = Scratch::new();
        let directory = edit_history_root(&scratch.bindings()).join("bindings.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("manual.json"), b"manual").unwrap();
        let today: u64 = 20_000;
        for number in 1..=57 {
            let age = 35u64.saturating_sub(number);
            let time = UNIX_EPOCH + Duration::from_secs((today - age) * 86_400);
            fs::File::create(directory.join(filename(number)))
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        let (last, records) = revisions(&directory).unwrap();
        assert_eq!(last, 57);
        trim(&records, today);
        for number in 1..=57 {
            assert_eq!(
                directory.join(filename(number)).exists(),
                (6..35).contains(&number) || number >= 38,
                "{number}"
            );
        }
        assert_eq!(fs::read(directory.join("manual.json")).unwrap(), b"manual");
    }
}
