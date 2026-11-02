//! 气质花（FlairBloom）业务 Profile schema：连发规则、KeyId、宏序列与 schema 迁移。
//!
//! 提供高层 I/O helper [`load_from_path`] / [`save_to_path`]，封装
//! read+decrypt+migrate+validate / serialize+encrypt+atomic-rename 全流程。
//!
//! 文件容器格式（[`qzh_format::header::FileHeader`] / AES-GCM）由 `qzh-format` crate 负责。

pub mod key_id;
pub mod key_policy;
pub mod profile;
pub mod schema_migrate;
pub mod storage;

pub use key_id::{KeyId, MouseButton};
pub use key_policy::{
    accepts, find_rule_violations, sanitize_profile, slot_policy, InjectCaps, KeyRejection,
    KeySlot, SanitizeReport, SlotPolicy,
};
pub use profile::{
    sanitize_signature, Advanced, BurstMode, BurstRule, Hotkeys, OutputStep, Profile, ProfileError,
    ProfileMeta, CURRENT_SCHEMA_VERSION, DEFAULT_INTERVAL_MS, MAX_INTERVAL_MS, MAX_OUTPUT_STEPS,
    MAX_RULES, MAX_SIGNATURE_CHARS, MIN_EFFECTIVE_INTERVAL_MS, MIN_INTERVAL_MS,
};
pub use schema_migrate::migrate_profile;

use std::path::Path;

impl From<qzh_format::FormatError> for ProfileError {
    fn from(e: qzh_format::FormatError) -> Self {
        match e {
            qzh_format::FormatError::Io(e) => Self::Io(e),
            qzh_format::FormatError::InvalidHeader => Self::InvalidFormat,
            qzh_format::FormatError::Crypto(e) => Self::Crypto(e),
            qzh_format::FormatError::Json(e) => Self::Json(e),
        }
    }
}

/// 读取、解密、迁移、校验一个 `.qzh` 文件，返回 [`Profile`]。
pub fn load_from_path(path: &Path) -> Result<Profile, ProfileError> {
    let (value, schema_version) = qzh_format::read_encrypted(path)?;
    let value = if schema_version < CURRENT_SCHEMA_VERSION {
        migrate_profile(value, schema_version)?
    } else if schema_version > CURRENT_SCHEMA_VERSION {
        return Err(ProfileError::TooNew(schema_version));
    } else {
        value
    };
    let mut profile: Profile = serde_json::from_value(value)?;
    // 加载即适配有效操作下限：旧配置 <10ms 的间隔静默钳到 10ms（管线可持续注入上限），
    // 避免超发导致停止后下游输入队列排空滞后（「收不住」）。见 [`MIN_EFFECTIVE_INTERVAL_MS`]。
    profile.clamp_intervals();
    profile.validate()?;
    Ok(profile)
}

/// 校验、序列化、加密、原子写入 [`Profile`] 到 `.qzh` 文件。
pub fn save_to_path(path: &Path, profile: &Profile) -> Result<(), ProfileError> {
    profile.validate()?;
    qzh_format::write_encrypted(path, profile)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Advanced, Hotkeys, ProfileMeta};

    fn minimal_profile() -> Profile {
        Profile {
            schema_version: CURRENT_SCHEMA_VERSION,
            meta: ProfileMeta {
                name: "test".into(),
                created_at: 0,
                updated_at: 0,
                app_version: "0".into(),
                signature: String::new(),
            },
            rules: vec![],
            hotkeys: Hotkeys::default(),
            advanced: Advanced::default(),
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join("qzh_profile_test_roundtrip.qzh");
        let profile = minimal_profile();
        save_to_path(&path, &profile).unwrap();
        let loaded = load_from_path(&path).unwrap();
        assert_eq!(loaded.meta.name, "test");
        assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_clamps_sub_effective_interval_up_to_floor() {
        use crate::key_id::KeyId;
        use crate::profile::{BurstMode, BurstRule};
        let dir = std::env::temp_dir();
        let path = dir.join("qzh_profile_test_clamp.qzh");
        let mut profile = minimal_profile();
        // 旧配置存了一个低于有效下限的间隔（结构合法 ≥1，保存允许）。
        profile.rules.push(BurstRule {
            id: "r".into(),
            enabled: true,
            trigger_key: KeyId::Keyboard(0x51),
            steps: vec![crate::profile::OutputStep {
                key: KeyId::Keyboard(0x45),
                interval_ms: 5,
            }],
            mode: BurstMode::Hold,
            stop_key: None,
            group: None,
        });
        save_to_path(&path, &profile).unwrap();

        // 加载时被钳到有效下限。
        let loaded = load_from_path(&path).unwrap();
        assert_eq!(
            loaded.rules[0].steps[0].interval_ms,
            MIN_EFFECTIVE_INTERVAL_MS
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_from_nonexistent_file_returns_io_error() {
        let path = std::path::PathBuf::from("/tmp/qzh_profile_nonexistent.qzh");
        let _ = std::fs::remove_file(&path);
        assert!(matches!(load_from_path(&path), Err(ProfileError::Io(_))));
    }

    #[test]
    fn save_rejects_invalid_profile() {
        use crate::key_id::KeyId;
        use crate::profile::{BurstMode, BurstRule};
        let dir = std::env::temp_dir();
        let path = dir.join("qzh_profile_test_invalid.qzh");
        let mut profile = minimal_profile();
        // 间隔超范围
        profile.rules.push(BurstRule {
            id: "r".into(),
            enabled: true,
            trigger_key: KeyId::Keyboard(0x51),
            steps: vec![crate::profile::OutputStep {
                key: KeyId::Keyboard(0x51),
                interval_ms: 0,
            }],
            mode: BurstMode::Hold,
            stop_key: None,
            group: None,
        });
        assert!(matches!(
            save_to_path(&path, &profile),
            Err(ProfileError::InvalidInterval(0))
        ));
        let _ = std::fs::remove_file(&path);
    }
}
