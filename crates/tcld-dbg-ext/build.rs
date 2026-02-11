fn main() {
    let loadable_extension = std::env::var_os("CARGO_FEATURE_LOADABLE_EXTENSION").is_some();
    let mut build = cc::Build::new();
    build.file("src/opcode_runtime_shim.c");
    if let Ok(include) = std::env::var("DEP_TCL_INCLUDE") {
        build.include(include);
    }
    build.compile("tcld_opcode_runtime_shim");

    if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=dl");
    }

    // The crate's test binaries link libtcl directly; rustc-link-arg does
    // not propagate from tcl-sys's build script, so re-emit the rpath here
    // (Linux resolves the bare soname through rpath alone).
    if loadable_extension {
        if cfg!(target_os = "macos") {
            println!("cargo:rustc-cdylib-link-arg=-Wl,-undefined,dynamic_lookup");
        } else if cfg!(target_os = "linux") {
            println!("cargo:rustc-cdylib-link-arg=-Wl,--allow-shlib-undefined");
        }
    } else if let Ok(lib_dir) = std::env::var("DEP_TCL_LIB_DIR")
        && cfg!(any(target_os = "macos", target_os = "linux"))
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
    }

    println!("cargo:rerun-if-changed=src/opcode_runtime_shim.c");
}
