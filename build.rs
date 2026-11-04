fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    embed_icon();
    // The self-contained MinGW toolchain has no libshlwapi.a, which a dependency
    // (webbrowser) links against. GNU ld can link straight against a DLL, so give it
    // a private copy of the system's shlwapi.dll to resolve the import from.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        return;
    }
    let out = std::env::var("OUT_DIR").unwrap();
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let src = std::path::Path::new(&root).join("System32").join("shlwapi.dll");
    let dst = std::path::Path::new(&out).join("shlwapi.dll");
    if std::fs::copy(&src, &dst).is_ok() {
        println!("cargo:rustc-link-search=native={out}");
    }
}

/// Icon and version details of the .exe. Without a resource compiler the program still
/// builds, just without them.
#[cfg(windows)]
fn embed_icon() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico").set("ProductName", "Daleris").set("FileDescription", "Daleris");
    if let Err(e) = res.compile() {
        println!("cargo:warning=building without the embedded icon: {e}");
    }
}

#[cfg(not(windows))]
fn embed_icon() {}
