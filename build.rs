fn main() {
    println!("cargo:rerun-if-changed=src/menu.m");

    #[cfg(target_os = "macos")]
    {
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
