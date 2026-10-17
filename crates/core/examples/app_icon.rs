//! Writes mccboard's app icon (logo L4 on its graphite tile) as a 1024 px PNG: the one source
//! image of `src-tauri/icons`. From the repository root:
//!
//! ```text
//! cargo run -p mccboard-core --example app_icon -- src-tauri/app-icon.png
//! cd ui && npx tauri icon ../src-tauri/app-icon.png -o ../src-tauri/icons
//! ```
//!
//! then keep only the files `src-tauri/tauri.conf.json` lists, and `icon.png`.

use mccboard_core::{LogoStyle, mccboard_logo};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "app-icon.png".to_owned());
    let image = mccboard_logo(1024, LogoStyle::App);
    std::fs::write(&path, image.to_png()).expect("cannot write the icon");
    println!("{path}");
}
