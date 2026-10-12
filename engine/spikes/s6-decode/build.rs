//! Bindings for the two software decoders the spike drives directly (dav1d and
//! libvpx), generated from Homebrew's headers so struct layouts are exact.
//! FFmpeg's wrappers for them allocate from internal buffer pools and ignore
//! `get_buffer2`, so direct sessions are the only way to decode them into
//! Metal-shared memory without a copy.

use std::path::PathBuf;

fn main() {
    let brew = std::env::var("HOMEBREW_PREFIX").unwrap_or_else(|_| "/opt/homebrew".into());
    let dav1d = PathBuf::from(&brew).join("opt/dav1d");
    let vpx = PathBuf::from(&brew).join("opt/libvpx");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=HOMEBREW_PREFIX");
    println!("cargo:rustc-link-search=native={}", dav1d.join("lib").display());
    println!("cargo:rustc-link-search=native={}", vpx.join("lib").display());
    println!("cargo:rustc-link-lib=dylib=dav1d");
    println!("cargo:rustc-link-lib=dylib=vpx");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindgen::Builder::default()
        .header_contents(
            "s6.h",
            "#include <dav1d/dav1d.h>\n#include <vpx/vpx_decoder.h>\n#include <vpx/vp8dx.h>\n#include <vpx/vpx_frame_buffer.h>\n",
        )
        .clang_arg(format!("-I{}", dav1d.join("include").display()))
        .clang_arg(format!("-I{}", vpx.join("include").display()))
        .allowlist_function("dav1d_.*")
        .allowlist_type("Dav1d.*")
        .allowlist_var("DAV1D_.*")
        .allowlist_function("vpx_.*")
        .allowlist_type("vpx_.*")
        .allowlist_var("VPX_.*")
        .allowlist_var("vpx_codec_vp[89]_dx_algo")
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate()
        .expect("bindgen")
        .write_to_file(out.join("swdec.rs"))
        .expect("write bindings");
}
