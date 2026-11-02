#[cfg(windows)]
mod build_support;

fn main() {
    // 必须先把 interception.dll 同步到 resources/，再交给 tauri_build：
    // tauri_build::build() 会校验 bundle.resources 里所有路径存在，否则中止。
    #[cfg(windows)]
    copy_interception_dll();

    tauri_build::build();
}

/// `interception-sys` 把 `interception.dll` 复制到自己的 OUT_DIR 后供链接，但运行时
/// 仅 `cargo run` 会自动把 OUT_DIR 注入 dll 搜索路径。直接执行 `target/<profile>/flair-bloom.exe`
/// （例如 ShellExecuteEx 提权重启或安装后启动）会因找不到 interception.dll 进程加载失败。
///
/// 复制目标有两处：
/// 1. `target/<profile>/interception.dll`：与 dev/release 的 EXE 同级，覆盖 `cargo run`、
///    `pnpm dev`、直接运行 release exe 等所有本地启动方式。
/// 2. `apps/main/src-tauri/resources/interception.dll`：让 `bundle.resources` 能把 DLL 打入
///    NSIS/MSI 安装包；NSIS hook 在 PostInstall 时再把它从 `$INSTDIR\resources\` 移动到
///    EXE 同级，保证安装后的运行环境也能解析。
#[cfg(windows)]
fn copy_interception_dll() {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let Ok(out_dir) = env::var("OUT_DIR") else {
        return;
    };
    let out = PathBuf::from(&out_dir);
    let Some(target_profile_dir) = out.ancestors().nth(3).map(|p| p.to_path_buf()) else {
        return;
    };
    let build_dir = target_profile_dir.join("build");

    let Ok(entries) = fs::read_dir(&build_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.starts_with("interception-sys-") {
            continue;
        }
        let dll = entry.path().join("out").join("interception.dll");
        if !dll.exists() {
            continue;
        }

        let resources_dir = env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .map(|p| p.join("resources"));

        let mut dests: Vec<PathBuf> = vec![target_profile_dir.join("interception.dll")];
        if let Ok(res_dir) = resources_dir {
            dests.push(res_dir.join("interception.dll"));
        }

        for dest in &dests {
            if let Err(e) = build_support::copy_if_changed(&dll, dest) {
                println!(
                    "cargo:warning=复制 interception.dll 到 {} 失败: {}",
                    dest.display(),
                    e
                );
            }
        }

        println!("cargo:rerun-if-changed={}", dll.display());
        return;
    }
}
