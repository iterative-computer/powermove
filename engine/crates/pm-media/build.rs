//! Links pm-media's native dependencies and generates their bindings.
//!
//! * **FFmpeg**: Powermove's LGPL build (`engine/third_party/ffmpeg/build.sh`)
//!   at `PM_FFMPEG_DIR` (`engine/env.sh` sets it), linked as dylibs. Bindings
//!   come from that build's headers, so struct layouts are exact for the
//!   version that ships.
//! * **dav1d, OpenEXRCore, and libjpeg-turbo**: static libraries from
//!   `native/build.sh` (pinned, checksummed sources) at `PM_MEDIA_NATIVE_DIR`,
//!   by default a directory named after the stamp (the checksum of
//!   `native/sources.lock` and `native/build.sh`), so worktrees on different
//!   versions of the script never replace each other's build. When the staged
//!   build is missing, this script runs `native/build.sh` (about a minute;
//!   downloads are cached).
//! * **Apple frameworks**: VideoToolbox, CoreMedia, CoreVideo, CoreFoundation,
//!   ImageIO, CoreGraphics.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME"))
}

fn ffmpeg_dir() -> PathBuf {
    let dir = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("Library/Caches/powermove-engine/ffmpeg/out"));
    if !dir.join("include/libavcodec/avcodec.h").is_file()
        || !dir.join("lib/libavcodec.dylib").exists()
    {
        panic!(
            "pm-media needs Powermove's FFmpeg build at {} (PM_FFMPEG_DIR). Build it with \
             engine/third_party/ffmpeg/build.sh and `source engine/env.sh`.",
            dir.display()
        );
    }
    dir
}

fn native_dir(manifest: &Path) -> PathBuf {
    let native = manifest.join("native");
    let lock = native.join("sources.lock");
    let script = native.join("build.sh");
    println!("cargo:rerun-if-changed={}", lock.display());
    println!("cargo:rerun-if-changed={}", script.display());
    let mut h = Sha256::new();
    h.update(std::fs::read(&lock).expect("native/sources.lock"));
    h.update(std::fs::read(&script).expect("native/build.sh"));
    let stamp: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let cache = home().join("Library/Caches/powermove-engine/media-native");
    let dir = std::env::var_os("PM_MEDIA_NATIVE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| cache.join(format!("out-{}", &stamp[..16])));
    let current = std::fs::read_to_string(dir.join("stamp"))
        .map(|s| s.trim() == stamp)
        .unwrap_or(false);
    if !current {
        println!(
            "cargo:warning=building pm-media native libraries (dav1d, OpenEXRCore, libjpeg-turbo) into {}",
            dir.display()
        );
        let status = Command::new("bash")
            .arg(&script)
            .env("PM_MEDIA_NATIVE_DIR", &dir)
            .env(
                "PM_MEDIA_NATIVE_WORK",
                cache.join(format!("work-{}", &stamp[..16])),
            )
            .status()
            .expect("run native/build.sh");
        assert!(
            status.success(),
            "native/build.sh failed; run it by hand to see the log"
        );
        let built = std::fs::read_to_string(dir.join("stamp")).unwrap_or_default();
        assert_eq!(
            built.trim(),
            stamp,
            "native/build.sh wrote a different stamp"
        );
    }
    dir
}

fn bindings(name: &str, header: &str, include: &[&Path], allow: &[&str], out: &Path) {
    let mut b = bindgen::Builder::default()
        .header_contents(&format!("{name}.h"), header)
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .layout_tests(false)
        .derive_default(true)
        .generate_comments(false)
        .size_t_is_usize(true);
    for dir in include {
        b = b.clang_arg(format!("-I{}", dir.display()));
    }
    for pattern in allow {
        b = b
            .allowlist_function(pattern)
            .allowlist_type(pattern)
            .allowlist_var(pattern);
    }
    b.generate()
        .unwrap_or_else(|e| panic!("bindgen {name}: {e}"))
        .write_to_file(out.join(format!("{name}.rs")))
        .expect("write bindings");
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PM_FFMPEG_DIR");
    println!("cargo:rerun-if-env-changed=PM_MEDIA_NATIVE_DIR");

    // FFmpeg dylibs. Symlinked into OUT_DIR so `cargo test` and `cargo run`
    // put them on the dynamic library path for this crate and its dependents
    // (cargo only does that for search paths inside the target directory).
    let ff = ffmpeg_dir();
    let ff_lib = out.join("ffmpeg-lib");
    let _ = std::fs::remove_dir_all(&ff_lib);
    std::fs::create_dir_all(&ff_lib).unwrap();
    for entry in std::fs::read_dir(ff.join("lib")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".dylib") {
            std::os::unix::fs::symlink(entry.path(), ff_lib.join(&name)).unwrap();
        }
    }
    println!("cargo:rustc-link-search=native={}", ff_lib.display());
    for lib in ["avformat", "avcodec", "avutil"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
    // Test and example binaries of this package find the dylibs without the
    // environment too.
    println!(
        "cargo:rustc-link-arg=-Wl,-rpath,{}",
        ff.join("lib").display()
    );
    println!("cargo:ffmpeg_lib={}", ff.join("lib").display());
    let ff_inc = ff.join("include");
    bindings(
        "ffmpeg",
        "#include <libavformat/avformat.h>\n#include <libavcodec/avcodec.h>\n#include <libavutil/avutil.h>\n\
         #include <libavutil/pixdesc.h>\n#include <libavutil/imgutils.h>\n#include <libavutil/buffer.h>\n\
         #include <libavutil/frame.h>\n#include <libavutil/dict.h>\n#include <libavutil/opt.h>\n\
         #include <libavutil/mastering_display_metadata.h>\n#include <libavutil/display.h>\n\
         #include <libavutil/channel_layout.h>\n#include <libavutil/samplefmt.h>\n#include <libavutil/error.h>\n\
         #include <libavutil/log.h>\n#include <libavutil/timecode.h>\n",
        &[&ff_inc],
        &[
            "av_.*",
            "avformat_.*",
            "avcodec_.*",
            "avio_.*",
            "AV.*",
            "FF_.*",
            "LIBAV.*",
        ],
        &out,
    );

    // dav1d and OpenEXRCore, static.
    let native = native_dir(&manifest);
    println!(
        "cargo:rustc-link-search=native={}",
        native.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=dav1d");
    println!("cargo:rustc-link-lib=static=OpenEXRCore-3_5");
    println!("cargo:rustc-link-lib=static=turbojpeg");
    println!("cargo:rustc-link-lib=dylib=c++");
    let inc = native.join("include");
    bindings(
        "dav1d",
        "#include <dav1d/dav1d.h>\n",
        &[&inc],
        &["dav1d_.*", "Dav1d.*", "DAV1D_.*"],
        &out,
    );
    bindings(
        "openexr",
        "#include <openexr/openexr.h>\n",
        &[&inc, &inc.join("openexr")],
        &["exr_.*", "EXR_.*", "_priv_exr.*"],
        &out,
    );

    bindings(
        "turbojpeg",
        "#include <stddef.h>\n#include <turbojpeg/turbojpeg.h>\n",
        &[&inc],
        &["tj.*", "TJ.*"],
        &out,
    );

    for fw in [
        "CoreFoundation",
        "CoreMedia",
        "CoreVideo",
        "VideoToolbox",
        "ImageIO",
        "CoreGraphics",
    ] {
        println!("cargo:rustc-link-lib=framework={fw}");
    }
}
