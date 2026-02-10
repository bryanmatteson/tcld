fn main() {
    if let Ok(lib_dir) = std::env::var("DEP_TCL_LIB_DIR")
        && cfg!(any(target_os = "macos", target_os = "linux"))
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
    }
}
