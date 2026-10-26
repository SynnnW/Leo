use std::path::PathBuf;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/leo.ico");
        res.set("ProductName", "Leo");
        res.set("FileDescription", "Installer Leo");
        res.compile().expect("gagal menanam ikon (butuh rc.exe dari Windows SDK)");
    }
    // Installer membawa leo.exe di dalamnya. Lokasinya bisa diatur lewat LEO_EXE
    // (dipakai tools/build-installer.ps1); bawaannya target/release/leo.exe.
    let src = std::env::var("LEO_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("../target/release/leo.exe"));
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("leo.exe");
    std::fs::copy(&src, &out)
        .unwrap_or_else(|e| panic!("leo.exe tidak ditemukan di {}: {e}. Jalankan tools/build-installer.ps1", src.display()));
    println!("cargo:rerun-if-env-changed=LEO_EXE");
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed=../assets/leo.ico");
}
