//! Settings › General › "Open Projects In": Tabs (the default) or Separate
//! Windows. The shell holds the mode (`pm_shell::tabs_mode`); this keeps it
//! following the setting.
use gpui::App;
use pm_kernel::KernelHandle;

/// The setting as it stands: Tabs unless Separate Windows was chosen.
pub(crate) fn setting(kernel: &KernelHandle) -> bool {
    kernel
        .state()
        .settings
        .get("openProjectsIn")
        .and_then(pm_model::Value::as_str)
        != Some("windows")
}

/// The setting changed: the shell follows it (the next project opened opens
/// by it; `pm_shell::apply_tabs_mode` says what happens to windows already
/// open) and every window redraws its bar.
pub(crate) fn follow(kernel: &KernelHandle, cx: &mut App) {
    let tabs = setting(kernel);
    if tabs == pm_shell::tabs_mode() {
        return;
    }
    cx.defer(move |cx| {
        pm_shell::apply_tabs_mode(tabs);
        for editor in crate::entry::editors(cx) {
            editor.read(cx).host.borrow().sync_tabs();
        }
    });
}
