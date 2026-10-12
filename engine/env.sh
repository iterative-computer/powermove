# Source this file to use the pinned toolchain: `source engine/env.sh`
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
# Shared compile cache across worktrees (https://github.com/kunobi-ninja/kache).
command -v kache >/dev/null 2>&1 && export RUSTC_WRAPPER=kache
# Keep the machine responsive and target/ small: several worktrees build at once on one Mac.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=line-tables-only
# Our LGPL FFmpeg build (engine/third_party/ffmpeg), shared across worktrees.
export PM_FFMPEG_DIR="${PM_FFMPEG_DIR:-$HOME/Library/Caches/powermove-engine/ffmpeg/out}"
