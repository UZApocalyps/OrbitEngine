//! Embeds the Windows executable icon (`assets/brand/icon.ico`).

fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    {
        // Only when targeting Windows (the web build of the player skips it).
        if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("../../assets/brand/icon.ico");
            if let Err(e) = res.compile() {
                println!("cargo:warning=could not embed the exe icon: {e}");
            }
        }
    }
}
