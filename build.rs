//! Embeds the app icon (assets/icon.ico) in the Windows exe.

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    let result = embed_resource::compile("assets/app.rc", embed_resource::NONE);
    // The real (MSVC) build must embed it; other targets may lack a resource compiler.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        result.manifest_required().expect("could not embed the icon");
    } else if let Err(e) = result.manifest_optional() {
        println!("cargo:warning=icon not embedded: {e}");
    }
}
