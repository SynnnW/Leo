use std::path::PathBuf;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/amora.ico");
        res.set("ProductName", "Amora");
        res.set("FileDescription", "Installer Amora");
        res.compile().expect("gagal menanam ikon (butuh rc.exe dari Windows SDK)");
    }
    // Installer membawa amora.exe di dalamnya. Lokasinya bisa diatur lewat AMORA_EXE
    // (dipakai tools/build-installer.ps1); bawaannya target/release/amora.exe.
    let src = std::env::var("AMORA_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("../target/release/amora.exe"));
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("amora.exe");
    std::fs::copy(&src, &out)
        .unwrap_or_else(|e| panic!("amora.exe tidak ditemukan di {}: {e}. Jalankan tools/build-installer.ps1", src.display()));
    println!("cargo:rerun-if-env-changed=AMORA_EXE");
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed=../assets/amora.ico");
}
