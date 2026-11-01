//! macOS: a command-line tool needs an embedded Info.plist with the
//! Bluetooth usage description, or CoreBluetooth refuses to start. The
//! template gets the package version.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let template = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/macos/Info.plist");
        println!("cargo:rerun-if-changed={}", template.display());
        let text = std::fs::read_to_string(&template).expect("Info.plist");
        let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("Info.plist");
        std::fs::write(
            &out,
            text.replace("@VERSION@", &std::env::var("CARGO_PKG_VERSION").unwrap()),
        )
        .expect("write Info.plist");
        println!(
            "cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            out.display()
        );
    }
}
