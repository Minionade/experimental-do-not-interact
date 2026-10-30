fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // SDL2's static Windows backend calls registry APIs. sdl2-sys 0.37
    // omits this import library from its emitted static link list.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_BUNDLED_SDL").is_some()
    {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
