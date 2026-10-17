fn main() {
    // Tauri's default Windows manifest, plus the supported Windows versions: without them a
    // layered window can't be a child window, and the taskbar widget is one (#45).
    let windows = tauri_build::WindowsAttributes::new()
        .app_manifest(include_str!("windows-app-manifest.xml"));
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("tauri-build failed: {error:#}");
    }
}
