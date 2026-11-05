use super::core::{id, Environment, QueueFile, QUEUE_BYTES};
use std::path::{Path, PathBuf};

pub fn read_queue(path: Option<&Path>) -> QueueFile {
    path.and_then(|path| {
        (std::fs::metadata(path).ok()?.len() <= QUEUE_BYTES as u64).then_some(())?;
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    })
    .unwrap_or_default()
}

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 同目录替换，不会因崩溃留下半份队列；Windows 的 rename 不覆盖目标，使用 MoveFileExW。
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, bytes)?;
    replace(&temp, path)
}

#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: 两个路径均为 NUL 结尾宽串，缓冲区在调用期间有效。
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

pub fn environment(path: Option<&Path>) -> Environment {
    resolve_environment(machine_id(), path)
}

fn resolve_environment(
    machine: Option<(String, &'static str)>,
    path: Option<&Path>,
) -> Environment {
    if let Some((id, source)) = machine.filter(|(value, _)| {
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    }) {
        return Environment {
            id,
            source: source.into(),
            version: "1".into(),
            scope: "persistent".into(),
        };
    }
    if let Some(existing) = path
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|data| serde_json::from_slice::<Environment>(&data).ok())
        .filter(|env| {
            env.source == "random-uuid"
                && env.version == "1"
                && env.scope == "persistent"
                && uuid::Uuid::parse_str(&env.id).is_ok()
        })
    {
        return existing;
    }
    let mut environment = Environment {
        id: id(),
        source: "random-uuid".into(),
        version: "1".into(),
        scope: "persistent".into(),
    };
    if path.is_none_or(|path| write_json(path, &environment).is_err()) {
        environment.scope = "session".into();
    }
    environment
}

fn machine_id() -> Option<(String, &'static str)> {
    #[cfg(windows)]
    {
        use win_sysinfo::registry::{read_reg_sz_at, RegRoot};
        read_reg_sz_at(
            RegRoot::HklmWow64_64,
            r"SOFTWARE\Microsoft\Cryptography",
            "MachineGuid",
        )
        .map(|value| (value.trim().to_ascii_lowercase(), "windows-machine-guid"))
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/sbin/ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
            .ok()?;
        let output = String::from_utf8(output.stdout).ok()?;
        let line = output
            .lines()
            .find(|line| line.contains("\"IOPlatformUUID\""))?;
        let value = line.split('"').nth(3)?.trim().to_ascii_lowercase();
        Some((value, "macos-platform-uuid"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::fs::read_to_string("/etc/machine-id")
            .ok()
            .map(|value| (value.trim().to_ascii_lowercase(), "linux-machine-id"))
    }
}

pub struct Paths {
    pub queue: PathBuf,
    pub environment: PathBuf,
}

impl Paths {
    pub fn new(base: PathBuf) -> Self {
        let base = base.join(if cfg!(debug_assertions) {
            "insights-development"
        } else {
            "insights"
        });
        Self {
            queue: base.join("queue.json"),
            environment: base.join("environment.json"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_machine_id_uses_persistent_random_or_honest_session_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("environment.json");
        let first = resolve_environment(None, Some(&path));
        let second = resolve_environment(None, Some(&path));
        assert_eq!(first, second);
        assert_eq!(first.source, "random-uuid");
        assert_eq!(first.scope, "persistent");
        let transient = resolve_environment(None, None);
        assert_eq!(transient.scope, "session");
        let system = resolve_environment(
            Some(("system-id".into(), "windows-machine-guid")),
            Some(&path),
        );
        assert_eq!(system.source, "windows-machine-guid");
        assert_eq!(system.id, "system-id");
    }

    #[test]
    fn atomic_queue_replacement_reloads_and_empty_queue_stays_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/queue.json");
        write_json(&path, &QueueFile::default()).unwrap();
        write_json(&path, &QueueFile::default()).unwrap();
        assert!(read_queue(Some(&path)).items.is_empty());
        assert!(!path.with_extension("tmp").exists());
        std::fs::write(&path, b"broken").unwrap();
        assert!(read_queue(Some(&path)).items.is_empty());
        std::fs::write(&path, vec![b' '; QUEUE_BYTES + 1]).unwrap();
        assert!(read_queue(Some(&path)).items.is_empty());
    }
}
