fn main() {
    if let Ok(lib_dir) = std::env::var("DEP_TCL_LIB_DIR")
        && matches!(
            std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
            Ok("macos" | "linux")
        )
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
    }
}
