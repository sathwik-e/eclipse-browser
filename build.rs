fn main() {
    println!("cargo:rerun-if-changed=src/menu.m");

    // Build scripts are compiled for the host, so `#[cfg(target_os)]` would
    // describe the build machine; the target comes from Cargo's environment.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/menu.m")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .compile("eclipse_menu");

        // AppKit's standard editing commands (copy/paste/undo) are dispatched
        // through the main menu, so this framework is part of the app's UX.
        println!("cargo:rustc-link-lib=framework=Cocoa");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=WebKit");
    }
}
