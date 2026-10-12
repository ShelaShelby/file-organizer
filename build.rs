//! Embeds the app icon and manifest (assets/app.rc) in the Windows exe and
//! makes the exe load its own DLL imports from System32 only.

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if msvc {
        // LOAD_LIBRARY_SEARCH_SYSTEM32: a DLL planted next to the exe (e.g.
        // in Downloads) is never loaded for the exe's own imports.
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
        // The manifest comes from app.rc; the linker must not add its own.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
    }
    let result = embed_resource::compile("assets/app.rc", embed_resource::NONE);
    // The real (MSVC) build must embed it; other targets may lack a resource compiler.
    if msvc {
        result
            .manifest_required()
            .expect("could not embed the icon and manifest");
    } else if let Err(e) = result.manifest_optional() {
        println!("cargo:warning=icon not embedded: {e}");
    }
}
