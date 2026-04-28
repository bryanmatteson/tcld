fn main() {
    let target = std::env::var("TARGET").expect("Cargo TARGET");
    println!("cargo:rustc-env=TCLD_TARGET_TRIPLE={target}");
}
