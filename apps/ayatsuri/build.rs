// Windows only: embed the app icon and version info into the .exe, so
// Explorer, the taskbar and the installer show it. Other targets skip this.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/app-icon/ayatsuri2d.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/ayatsuri2d.ico");
    res.set("ProductName", "Ayatsuri2D");
    res.set("FileDescription", "Ayatsuri2D");
    res.set("CompanyName", "Yoru_KenomoVT");
    res.set("LegalCopyright", "Yoru_KenomoVT, GPL-3.0");
    if let Err(e) = res.compile() {
        // no resource compiler: still build, just without the icon
        println!("cargo:warning=could not embed the Windows icon: {e}");
    }
}
