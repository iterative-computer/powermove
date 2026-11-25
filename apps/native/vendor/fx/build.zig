// Powermove notice: trimmed from upstream fx's build.zig at the commit in
// POWERMOVE.md (patch P-CAPI). Only the C ABI core surface is built here: a
// static library rooted at src/capi_core_main.zig, plus the fake inference
// server the tests and benchmarks run against. The fx CLI, terminal UI,
// WebAssembly and Node-API surfaces, fuzzing and PGO steps are left out.
const std = @import("std");

// Kept identical to upstream's, because shared code compares against it.
const WasmSurface = enum {
    none,
    core,
    term,
};

/// The upstream commit this tree is vendored from (POWERMOVE.md). The fork has
/// no .git of its own, so the build never asks git.
const upstream_commit = "8fbe9b8";
/// `pub const version` of upstream's src/main.zig at that commit, which this
/// tree does not vendor.
const upstream_version = "0.0.18";

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});

    const options = b.addOptions();
    options.addOption([]const u8, "git_commit", upstream_commit);
    options.addOption([]const u8, "app_version", upstream_version);
    options.addOption([]const u8, "update_channel", "stable");
    options.addOption(WasmSurface, "wasm_surface", .none);
    // A libfx artifact: it leaves out what only the fx binary uses.
    options.addOption(bool, "libfx", true);
    // libfx agents run only the host's tools.
    options.addOption(bool, "builtin_tools", false);
    const options_module = options.createModule();

    const lib = b.addLibrary(.{
        .name = "fx_core",
        .linkage = .static,
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/capi_core_main.zig"),
            .target = target,
            // Forced, as upstream forces it for the N-API addon: the core
            // parses untrusted model and protocol input, so safety checks
            // stay on whatever -Doptimize says.
            .optimize = .safe,
            .link_libc = true,
            .stack_check = false,
            .stack_protector = false,
            // Apple's arm64 ABI wants frame pointers, and crash reports and
            // profilers of the app walk them through the core.
            .omit_frame_pointer = false,
            .unwind_tables = .none,
            .error_tracing = false,
            .strip = false,
            // The archive goes into Rust's link: position-independent code,
            // and compiler-rt kept out so it cannot clash with Rust's
            // compiler_builtins (see POWERMOVE.md, "Static link").
            .pic = true,
        }),
    });
    lib.bundle_compiler_rt = false;
    lib.bundle_ubsan_rt = false;
    lib.root_module.addImport("build_options", options_module);
    addSessionManager(b, lib.root_module, target);
    const install_lib = b.addInstallArtifact(lib, .{});
    b.step("capi", "Build the libfx core static library (libfx_core.a)").dependOn(&install_lib.step);

    // The deterministic fake model server, test support only.
    const fake = b.addExecutable(.{
        .name = "fake-inference-server",
        .root_module = b.createModule(.{
            .root_source_file = b.path("test-support/fake-inference-server.zig"),
            .target = target,
            .optimize = .safe,
            .link_libc = true,
        }),
    });
    const install_fake = b.addInstallArtifact(fake, .{});
    b.step("fake-server", "Build the fake inference server").dependOn(&install_fake.step);

    // No Zig test step: the C ABI is tested through Rust (crate pm-aero-core),
    // and a test build of this root would pull in every upstream test of the
    // closure.

    b.getInstallStep().dependOn(&install_lib.step);
}

/// The session manager as its own module, as upstream's build makes it (no
/// fault or trace hooks).
fn addSessionManager(b: *std.Build, importer: *std.Build.Module, target: std.Build.ResolvedTarget) void {
    const options = b.addOptions();
    options.addOption(bool, "hooks", false);
    options.addOptionPathUntracked("trace_dir", b.graph.path(.install_prefix, "session-manager-traces"));
    options.addOptionPathUntracked("src_dir", b.path("src/core/session_manager"));
    const module = b.createModule(.{
        .root_source_file = b.path("src/core/session_manager/api.zig"),
        .target = target,
        .optimize = .safe,
        .link_libc = true,
        .pic = true,
    });
    module.addImport("build_options", options.createModule());
    importer.addImport("session_manager", module);
}
