use std::path::Path;

use qzh_profile::{Profile, DEFAULT_INTERVAL_MS};

use super::super::profile::{pick_import_base_name, read_imported_qzh};
use super::ImportPreview;

pub(super) fn matches(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("qzh"))
}

pub(super) fn read(path: &Path) -> Result<Profile, String> {
    read_imported_qzh(path)
}

pub(super) fn preview(path: &Path) -> Result<ImportPreview, String> {
    let profile = read(path)?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    Ok(ImportPreview {
        source_app: "气质花".into(),
        suggested_name: pick_import_base_name(&profile.meta.name, &stem),
        rule_count: profile.rules.len(),
        skipped_count: 0,
        interval_ms: profile
            .rules
            .iter()
            .flat_map(|r| r.steps.iter().map(|step| step.interval_ms))
            .min()
            .unwrap_or(DEFAULT_INTERVAL_MS),
        interval_max_ms: profile
            .rules
            .iter()
            .flat_map(|r| r.steps.iter().map(|step| step.interval_ms))
            .max()
            .unwrap_or(DEFAULT_INTERVAL_MS),
        warnings: Vec::new(),
        global_toggle: profile.hotkeys.global_toggle,
        global_stop: profile.hotkeys.global_stop,
        panel_toggle: profile.hotkeys.panel_toggle,
    })
}
