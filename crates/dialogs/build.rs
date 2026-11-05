fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // COM looks these up by their plain names, which 32-bit stdcall would otherwise decorate with @size.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target == "i686-pc-windows-gnu" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,--kill-at");
    }
}
