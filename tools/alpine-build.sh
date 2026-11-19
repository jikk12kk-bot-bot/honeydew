#!/bin/sh
# The Alpine (musl) release build, run as root in an Alpine container with the repository as the working directory:
# Rust from rustup, and the binaries linked to musl dynamically, as a static binary cannot load the X11, Wayland and
# GL libraries at run time. libclang is bindgen's (signalsmith-stretch, Lemon Sampler's time stretching).
# Usage: [HONEYDEW_FFMPEG_DIR=<dir>] tools/alpine-build.sh. It leaves honeydew-native and honeydew-import in
# target/release (or $CARGO_TARGET_DIR/release), owned by the repository's owner.
set -eu
apk add --no-cache build-base pkgconf linux-headers alsa-lib-dev clang-dev libxkbcommon-dev wayland-dev rustup
rustup-init -y -q --profile minimal --default-toolchain 1.99.1
. "$HOME/.cargo/env"
# With HONEYDEW_FFMPEG_DIR, the `video` feature, FFmpeg built there first unless it already holds a build
# (tools/build-ffmpeg.sh, whose scripts need bash and curl).
features=
if [ -n "${HONEYDEW_FFMPEG_DIR:-}" ]; then
    if [ ! -f "$HONEYDEW_FFMPEG_DIR/BUILD" ]; then
        apk add --no-cache bash curl diffutils
        bash tools/ci-ffmpeg.sh "$HONEYDEW_FFMPEG_DIR" host
    fi
    features="--features video"
fi
RUSTFLAGS='-C target-feature=-crt-static' cargo +1.99.1 build --release --locked --bins $features
chown -R "$(stat -c %u:%g .)" "${CARGO_TARGET_DIR:-target}"
if [ -n "${HONEYDEW_FFMPEG_DIR:-}" ]; then
    chown -R "$(stat -c %u:%g .)" "$(dirname "$HONEYDEW_FFMPEG_DIR")"
fi
