pub fn setup(app: &tauri::AppHandle) {
    #[cfg(windows)]
    {
        use tauri::Manager;
        for window in app.webview_windows().values() {
            match window.scale_factor() {
                Ok(scale) => apply(window, scale),
                Err(error) => tracing::warn!("读取窗口 DPI 失败（{}）：{error}", window.label()),
            }
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(windows)]
pub fn apply(window: &tauri::WebviewWindow, scale_factor: f64) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller3, COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS,
    };
    use windows::core::Interface;

    let label = window.label().to_owned();
    let callback_label = label.clone();
    if let Err(error) = window.with_webview(move |webview| {
        let result = (|| -> windows::core::Result<()> {
            let controller: ICoreWebView2Controller3 = webview.controller().cast()?;
            // SAFETY: with_webview 在控制器所属的主线程执行，接口由运行中的 WebView2 提供。
            unsafe {
                // WebView2 默认把文字大小也计入 DPI；固定画布只跟随显示器 DPI，
                // 应用自身的放大仍由前端 ZoomFactor 控制。
                controller.SetShouldDetectMonitorScaleChanges(false)?;
                controller.SetBoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS)?;
                controller.SetRasterizationScale(scale_factor)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!("设置 WebView 显示缩放失败（{callback_label}）：{error}");
        }
    }) {
        tracing::warn!("派发 WebView 显示缩放失败（{label}）：{error}");
    }
}
