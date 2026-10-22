// Windows: common-controls v6 manifest for native theming and DPI awareness.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/rsqldev.ico");
    if std::env::var("TARGET").unwrap_or_default().contains("windows") {
        use embed_manifest::manifest::{ActiveCodePage, DpiAwareness};
        let m = embed_manifest::new_manifest("rsqldev").active_code_page(ActiveCodePage::Utf8).dpi_awareness(DpiAwareness::PerMonitorV2);
        if let Err(e) = embed_manifest::embed_manifest(m) {
            println!("cargo:warning=manifest not embedded: {e}");
        }
        // Explorer / taskbar icon.
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/rsqldev.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
