//! 队列配置与旧版配置的目录隔离。

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

static MIGRATION: Mutex<()> = Mutex::new(());
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// 准备新版配置目录；旧文件只复制一次且不覆盖新版已存在的文件。
pub fn prepare_profile_directory(base: &Path) -> std::io::Result<PathBuf> {
    let _migration = MIGRATION.lock().unwrap_or_else(|e| e.into_inner());
    let destination = base.join("profiles-sequences");
    fs::create_dir_all(&destination)?;
    let marker = destination.join(".legacy-imported");
    if marker.exists() {
        return Ok(destination);
    }
    let source = base.join("profiles");
    if source.exists() {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || !entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("qzh"))
            {
                continue;
            }
            let path = destination.join(entry.file_name());
            if path.is_file() {
                continue;
            }
            // 正式文件只在复制完成后出现，崩溃残留的临时文件不会被当作已迁移配置。
            let (temporary, mut output) = loop {
                let temporary = destination.join(format!(
                    ".migration-{}-{}.tmp",
                    std::process::id(),
                    NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
                ));
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)
                {
                    Ok(output) => break (temporary, output),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
            };
            let copy = (|| {
                let mut input = fs::File::open(entry.path())?;
                std::io::copy(&mut input, &mut output)?;
                output.sync_all()
            })();
            drop(output);
            if let Err(error) = copy.and_then(|()| fs::rename(&temporary, &path)) {
                let _ = fs::remove_file(temporary);
                return Err(error);
            }
        }
    }
    fs::write(marker, b"ok")?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        std::env::temp_dir().join(format!(
            "flair-directory-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn incomplete_migration_preserves_existing_sequences_and_ignores_other_files() {
        let base = fixture();
        let legacy = base.join("profiles");
        let destination = base.join("profiles-sequences");
        fs::create_dir_all(legacy.join("nested.qzh")).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(legacy.join("existing.qzh"), b"old").unwrap();
        fs::write(legacy.join("another.QZH"), b"another").unwrap();
        fs::write(legacy.join("notes.txt"), b"ignored").unwrap();
        fs::write(destination.join("existing.qzh"), b"new").unwrap();
        fs::write(destination.join(".migration-stale.tmp"), b"partial").unwrap();
        prepare_profile_directory(&base).unwrap();
        assert_eq!(fs::read(destination.join("existing.qzh")).unwrap(), b"new");
        assert_eq!(
            fs::read(destination.join("another.QZH")).unwrap(),
            b"another"
        );
        assert!(!destination.join("notes.txt").exists());
        assert!(!destination.join("nested.qzh").exists());
        assert_eq!(fs::read(legacy.join("existing.qzh")).unwrap(), b"old");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn empty_install_and_conflicting_legacy_directory_are_handled_without_overwriting() {
        let base = fixture();
        let destination = prepare_profile_directory(&base).unwrap();
        assert!(destination.join(".legacy-imported").exists());
        fs::remove_dir_all(&base).unwrap();
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("profiles"), b"conflict").unwrap();
        assert!(prepare_profile_directory(&base).is_err());
        assert_eq!(fs::read(base.join("profiles")).unwrap(), b"conflict");
        assert!(!base.join("profiles-sequences/.legacy-imported").exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn legacy_writes_and_repeated_startup_do_not_replace_sequences() {
        let base = std::env::temp_dir().join(format!(
            "flair-directory-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let legacy = base.join("profiles");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("defaults.qzh"), b"old-default").unwrap();
        let directory = prepare_profile_directory(&base).unwrap();
        assert_eq!(
            fs::read(directory.join("defaults.qzh")).unwrap(),
            b"old-default"
        );
        fs::write(directory.join("defaults.qzh"), b"new-sequence").unwrap();
        fs::write(legacy.join("defaults.qzh"), b"old-startup-fallback").unwrap();
        prepare_profile_directory(&base).unwrap();
        assert_eq!(
            fs::read(directory.join("defaults.qzh")).unwrap(),
            b"new-sequence"
        );
        fs::remove_file(directory.join("defaults.qzh")).unwrap();
        prepare_profile_directory(&base).unwrap();
        assert!(!directory.join("defaults.qzh").exists());
        fs::remove_dir_all(base).unwrap();
    }
}
