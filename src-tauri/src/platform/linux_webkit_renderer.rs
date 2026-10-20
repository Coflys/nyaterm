const DISABLE_DMABUF_RENDERER: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
const FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";

pub fn prepare_webkit_renderer() {
    if std::env::var_os(DISABLE_DMABUF_RENDERER).is_some() || std::env::var_os(FORCE_SHM).is_some()
    {
        return;
    }

    // Keep WebKit's DMA-BUF renderer while using shared-memory buffers for compatibility
    // with graphics drivers and transparent windows. Respect any explicit user setting.
    // SAFETY: `run()` calls this synchronously before Tauri/GTK initialization and before
    // NyaTerm starts worker threads, so this does not race with other environment access.
    unsafe {
        std::env::set_var(FORCE_SHM, "1");
    }
}
