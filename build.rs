// Tanam ikon dan info versi ke amora.exe (terlihat di Explorer, taskbar, Settings → Apps).
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/amora.ico");
        res.set("ProductName", "Amora");
        res.set("FileDescription", "Amora");
        res.compile().expect("gagal menanam ikon (butuh rc.exe dari Windows SDK)");
    }
    println!("cargo:rerun-if-changed=assets/amora.ico");
}
