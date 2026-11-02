use super::*;

#[test]
fn sanitize_replaces_windows_reserved_chars() {
    assert_eq!(
        sanitize_filename(r#"a<b>c:d"e/f\g|h?i*j"#),
        "a_b_c_d_e_f_g_h_i_j"
    );
}

#[test]
fn sanitize_replaces_ascii_control_chars() {
    // 包含 NUL, \n, \t 等控制字符必须被替换
    assert_eq!(sanitize_filename("a\nb\tc\x00d"), "a_b_c_d");
}

#[test]
fn sanitize_preserves_chinese_and_normal_chars() {
    // 防止越来越严的过滤误伤合法文件名
    assert_eq!(sanitize_filename("默认配置-v2"), "默认配置-v2");
}

#[test]
fn sanitize_blocks_path_traversal_via_separators() {
    // ../../etc/passwd 类形态：单字符替换，每个分隔符变成一个 _
    assert_eq!(sanitize_filename("../../etc/passwd"), ".._.._etc_passwd");
    assert_eq!(
        sanitize_filename(r"..\..\windows\system32"),
        ".._.._windows_system32"
    );
}

#[test]
fn sanitize_keeps_dots_and_dashes() {
    // . 和 - 是合法文件名字符
    assert_eq!(sanitize_filename("my.profile-1"), "my.profile-1");
}

#[test]
fn sanitize_handles_empty_string() {
    assert_eq!(sanitize_filename(""), "");
}

#[test]
fn profile_path_for_name_appends_qzh_extension() {
    let dir = std::path::Path::new("/tmp/profiles");
    let p = profile_path_for_name(dir, "刺客");
    assert_eq!(p.parent().unwrap(), dir);
    assert_eq!(p.file_name().unwrap().to_string_lossy(), "刺客.qzh");
}

#[test]
fn profile_path_for_name_sanitizes_unsafe_chars() {
    // 名字含路径分隔符时不能逃出 dir
    let dir = std::path::Path::new("/tmp/profiles");
    let p = profile_path_for_name(dir, "../evil");
    assert_eq!(p.parent().unwrap(), dir);
    assert_eq!(p.file_name().unwrap().to_string_lossy(), ".._evil.qzh");
}

#[test]
fn pick_unique_name_returns_base_when_free() {
    let tmp = tempfile::tempdir().unwrap();
    let (name, path) = pick_unique_name(tmp.path(), "我的配置");
    assert_eq!(name, "我的配置");
    assert_eq!(path, tmp.path().join("我的配置.qzh"));
}

#[test]
fn pick_unique_name_appends_index_when_taken() {
    let tmp = tempfile::tempdir().unwrap();
    // 占位
    std::fs::write(tmp.path().join("我的配置.qzh"), b"x").unwrap();
    std::fs::write(tmp.path().join("我的配置 2.qzh"), b"x").unwrap();
    let (name, path) = pick_unique_name(tmp.path(), "我的配置");
    assert_eq!(name, "我的配置 3");
    assert_eq!(path, tmp.path().join("我的配置 3.qzh"));
}

#[test]
fn compute_aad_layout_is_magic_version_flags() {
    let aad = compute_aad();
    assert_eq!(aad.len(), 7);
    assert_eq!(&aad[..MAGIC.len()], MAGIC);
    assert_eq!(aad[MAGIC.len()], VERSION);
    // flags = 0u16 little-endian
    assert_eq!(&aad[MAGIC.len() + 1..], &[0u8, 0u8]);
}

#[test]
fn make_id_is_unique_across_calls() {
    let a = make_id();
    let b = make_id();
    assert_ne!(a, b);
    // 形如 "<16hex>-<n>"
    assert!(a.contains('-'));
}

#[test]
fn profile_summary_counts_rules_and_hotkeys() {
    let profile = Profile {
        schema_version: CURRENT_SCHEMA_VERSION,
        meta: ProfileMeta {
            name: "测试配置".to_string(),
            created_at: 1,
            updated_at: 2,
            app_version: "test".to_string(),
            signature: String::new(),
        },
        rules: vec![
            BurstRule {
                id: "hold".to_string(),
                enabled: true,
                trigger_key: KeyId::Keyboard(0x51),
                steps: vec![qzh_profile::profile::OutputStep {
                    key: KeyId::Keyboard(0x51),
                    interval_ms: 10,
                }],
                mode: BurstMode::Hold,
                stop_key: None,
                group: None,
            },
            BurstRule {
                id: "toggle".to_string(),
                enabled: false,
                trigger_key: KeyId::Keyboard(0x46),
                steps: vec![qzh_profile::profile::OutputStep {
                    key: KeyId::Keyboard(0x47),
                    interval_ms: 20,
                }],
                mode: BurstMode::Toggle,
                stop_key: Some(KeyId::Keyboard(0x48)),
                group: None,
            },
        ],
        hotkeys: Hotkeys {
            global_toggle: Some(KeyId::Keyboard(0x70)),
            global_stop: Some(KeyId::Keyboard(0x71)),
            panel_toggle: Some(KeyId::Mouse(qzh_profile::key_id::MouseButton::X1)),
        },
        advanced: Advanced::default(),
    };

    let summary = profile_summary(&profile);

    assert_eq!(summary.rules_total, 2);
    assert_eq!(summary.rules_enabled, 1);
    assert_eq!(summary.hold_count, 1);
    assert_eq!(summary.toggle_count, 1);
    assert_eq!(summary.group_count, 0);
    assert_eq!(summary.global_toggle, Some(KeyId::Keyboard(0x70)));
    assert_eq!(summary.global_stop, Some(KeyId::Keyboard(0x71)));
    assert_eq!(
        summary.panel_toggle,
        Some(KeyId::Mouse(qzh_profile::key_id::MouseButton::X1))
    );
}

#[test]
fn ensure_qzh_extension_appends_when_missing() {
    assert_eq!(
        ensure_qzh_extension(PathBuf::from("/tmp/我的配置")),
        PathBuf::from("/tmp/我的配置.qzh")
    );
}

#[test]
fn ensure_qzh_extension_keeps_existing_qzh() {
    // 大小写不敏感：.qzh / .QZH 都视为已有扩展名
    assert_eq!(
        ensure_qzh_extension(PathBuf::from("/tmp/a.qzh")),
        PathBuf::from("/tmp/a.qzh")
    );
    assert_eq!(
        ensure_qzh_extension(PathBuf::from("/tmp/a.QZH")),
        PathBuf::from("/tmp/a.QZH")
    );
}

#[test]
fn ensure_qzh_extension_appends_instead_of_replacing_other_extension() {
    // 追加而非替换，避免「我的.配置」被改写成「我的.qzh」
    assert_eq!(
        ensure_qzh_extension(PathBuf::from("/tmp/我的.配置")),
        PathBuf::from("/tmp/我的.配置.qzh")
    );
}

#[test]
fn import_base_name_prefers_meta_name() {
    assert_eq!(pick_import_base_name("刺客", "文件名"), "刺客");
}

#[test]
fn import_base_name_falls_back_to_stem_when_meta_empty_or_default() {
    assert_eq!(pick_import_base_name("", "分享配置"), "分享配置");
    assert_eq!(
        pick_import_base_name(DEFAULT_PROFILE_NAME, "分享配置"),
        "分享配置"
    );
}

#[test]
fn import_base_name_falls_back_to_placeholder() {
    // meta 与 stem 都不可用（空 / 默认名）时兜底「导入配置」
    assert_eq!(pick_import_base_name("", "  "), "导入配置");
    assert_eq!(
        pick_import_base_name(DEFAULT_PROFILE_NAME, DEFAULT_PROFILE_NAME),
        "导入配置"
    );
}

fn meta(created_at: u64, signature: &str) -> ProfileMeta {
    ProfileMeta {
        name: "p".into(),
        created_at,
        updated_at: 0,
        app_version: String::new(),
        signature: signature.into(),
    }
}

#[test]
fn carry_over_keeps_existing_signature_when_frontend_omits_it() {
    let mut incoming = meta(0, "");
    carry_over_meta(&mut incoming, Some(meta(100, "小花")), 500);
    assert_eq!(incoming.created_at, 100);
    assert_eq!(incoming.signature, "小花");
}

#[test]
fn carry_over_ignores_signature_sent_by_frontend() {
    // 署名只走 set_profile_signature：自动保存带来的值一律以文件为准
    let mut incoming = meta(0, "伪造");
    carry_over_meta(&mut incoming, Some(meta(100, "")), 500);
    assert_eq!(incoming.signature, "");
}

#[test]
fn carry_over_new_file_uses_now_and_no_signature() {
    let mut incoming = meta(0, "伪造");
    carry_over_meta(&mut incoming, None, 500);
    assert_eq!(incoming.created_at, 500);
    assert_eq!(incoming.signature, "");
}

#[test]
fn carry_over_keeps_explicit_created_at() {
    let mut incoming = meta(42, "");
    carry_over_meta(&mut incoming, Some(meta(100, "")), 500);
    assert_eq!(incoming.created_at, 42);
}
