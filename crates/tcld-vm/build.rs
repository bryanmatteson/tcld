fn main() {
    println!("cargo:rerun-if-changed=src/engine/execution.c");
    println!("cargo:rerun-if-changed=src/engine/coroutines.c");
    println!("cargo:rerun-if-env-changed=TCL9_SOURCE");
    if std::env::var_os("CARGO_FEATURE_DEBUG").is_some()
        && std::env::var_os("CARGO_FEATURE_HOST").is_some()
    {
        let include = std::env::var("DEP_TCL_INCLUDE").expect("Tcl headers");
        let mut build = cc::Build::new();
        build
            .file("src/engine/execution.c")
            .file("src/engine/coroutines.c")
            .include(&include);
        if let Ok(source) = std::env::var("TCL9_SOURCE") {
            build
                .include(format!("{source}/generic"))
                .include(format!("{source}/unix"));
        }
        build.compile("tcld_execution");
    }

    println!("cargo:rerun-if-changed=src/engine/fiber.c");
    if std::env::var_os("CARGO_FEATURE_DEBUG").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("emscripten")
    {
        cc::Build::new()
            .file("src/engine/fiber.c")
            .warnings(false)
            .compile("tcld_fiber");
    }
}
