fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // GPUI's nested element layout overflows the default 1 MiB Windows stack.
        println!("cargo:rustc-link-arg=/STACK:8000000");
    }
}
