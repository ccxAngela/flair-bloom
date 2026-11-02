use std::{fs, io, path::Path};

pub fn copy_if_changed(source: &Path, destination: &Path) -> io::Result<()> {
    let contents = fs::read(source)?;
    // 无变化时不能触碰目标：resources 下的写入会触发 Tauri 重启，EXE 同级 DLL 也可能正在使用。
    if fs::read(destination).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    fs::copy(source, destination)?;
    Ok(())
}
