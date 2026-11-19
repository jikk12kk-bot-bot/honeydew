#!/usr/bin/env bash
# The minimal static FFmpeg (with x264) that the `video` feature links: the Displayer's decoding and the Save dialog's
# MP4 render (`src/video/`). Decoders H.264, HEVC, VP8, VP9 (and AV1 through Android's MediaCodec), AAC, MP3, Opus,
# Vorbis, FLAC; the MP4/MOV, Matroska/WebM, MP3, AAC, Ogg and WAV demuxers; the MP4 muxer; the AAC encoder and x264
# (and Android's hardware H.264 encoder through MediaCodec). No programs, no network, nothing autodetected.
#
#   tools/build-ffmpeg.sh <out-dir> <target>
#
# <target>:
#   android-arm64 | android-armv7   with $ANDROID_NDK_HOME
#   host                            this machine's Linux (glibc or musl), macOS or BSD, with its cc
#   ios                             iPhone (arm64), on macOS with Xcode
#   freebsd                         x86_64 FreeBSD from Linux, after tools/freebsd-cross.sh
#   windows-x86_64 | windows-arm64  MSVC: run in an MSYS2 shell with the Visual Studio environment (cl, link) on PATH
# The libraries and headers go to <out-dir>/{lib,include}; point the build at it with HONEYDEW_FFMPEG_DIR (or
# HONEYDEW_FFMPEG_DIR_<target triple>, as tools/android-dist.sh does) and build with `--features video`.
#
# FFmpeg is LGPL-2.1-or-later and x264 GPL-2.0-or-later; built with `--enable-gpl` the result is GPL-2.0-or-later,
# which the app's GPL-3.0-or-later takes (README `License and credits`, licenses/).
set -euo pipefail

FFMPEG_VERSION=7.1.1
X264_COMMIT=31e19f92f00c7003fa115047ce50978bc98c3a0d
ANDROID_API=26

if [ $# -ne 2 ]; then
    echo "usage: $0 <out-dir> <android-arm64|android-armv7|host|ios|freebsd|windows-x86_64|windows-arm64>" >&2
    exit 2
fi
mkdir -p "$1"
out=$(cd "$1" && pwd)
# pkg-config's search path, in the shell's own form (MSYS2's pkgconf splits it at colons).
pc=$out/lib/pkgconfig
target=$2
case "$target" in
    # cl takes D:/a/b paths, not MSYS2's /d/a/b.
    windows-*) out=$(cygpath -m "$out") ;;
esac
src=${HONEYDEW_FFMPEG_SRC:-$(dirname "$out")/src}
jobs=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
mkdir -p "$src"

ffmpeg_src=$src/FFmpeg-n$FFMPEG_VERSION
x264_src=$src/x264-$X264_COMMIT
if [ ! -d "$ffmpeg_src" ]; then
    curl -fsSL "https://codeload.github.com/FFmpeg/FFmpeg/tar.gz/refs/tags/n$FFMPEG_VERSION" | tar xz -C "$src"
fi
if [ ! -d "$x264_src" ]; then
    curl -fsSL "https://codeload.github.com/mirror/x264/tar.gz/$X264_COMMIT" | tar xz -C "$src"
fi

x264_flags=()
ffmpeg_flags=()
threads=--enable-pthreads
case "$target" in
    android-arm64 | android-armv7)
        : "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK}"
        tc=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64
        if [ "$target" = android-arm64 ]; then
            host=aarch64-linux-android
            arch=aarch64
            cpu=armv8-a
            cc=$tc/bin/aarch64-linux-android$ANDROID_API-clang
        else
            host=armv7a-linux-androideabi
            arch=arm
            cpu=armv7-a
            cc=$tc/bin/armv7a-linux-androideabi$ANDROID_API-clang
            # 32-bit ARM: plain C. Its hand-written assembly is the part of both projects least tried with clang's
            # integrated assembler, and armv7 phones are few.
            x264_flags+=(--disable-asm)
            ffmpeg_flags+=(--disable-asm)
        fi
        export CC=$cc AS=$cc AR=$tc/bin/llvm-ar RANLIB=$tc/bin/llvm-ranlib STRIP=$tc/bin/llvm-strip NM=$tc/bin/llvm-nm
        x264_flags+=(--host="$host" --sysroot="$tc/sysroot")
        ffmpeg_flags+=(
            --target-os=android --enable-cross-compile --arch="$arch" --cpu="$cpu"
            --cc="$cc" --cxx="$cc++" --ar="$AR" --nm="$NM" --ranlib="$RANLIB" --strip="$STRIP"
            --sysroot="$tc/sysroot"
            # MediaCodec, through the NDK's AMediaCodec (no JavaVM is handed to FFmpeg).
            --enable-jni --enable-mediacodec
            --enable-decoder=h264_mediacodec,hevc_mediacodec,vp8_mediacodec,vp9_mediacodec,av1_mediacodec
            --enable-encoder=h264_mediacodec
        )
        ;;
    host)
        # No nasm needed: plain C (a desktop build may drop these for speed).
        x264_flags+=(--disable-asm)
        ffmpeg_flags+=(--disable-x86asm)
        ;;
    ios)
        sdk=$(xcrun --sdk iphoneos --show-sdk-path)
        cc=$(xcrun --sdk iphoneos -f clang)
        # The deployment target tools/ios-dist.sh builds the app for.
        flags="-arch arm64 -miphoneos-version-min=${IPHONEOS_DEPLOYMENT_TARGET:-14.0}"
        export CC=$cc
        x264_flags+=(--host=aarch64-apple-darwin --sysroot="$sdk" --extra-cflags="$flags" --extra-ldflags="$flags"
            --disable-asm)
        ffmpeg_flags+=(
            --target-os=darwin --enable-cross-compile --arch=aarch64 --cc="$cc" --sysroot="$sdk"
            --extra-cflags="$flags" --extra-ldflags="$flags"
        )
        ;;
    freebsd)
        # tools/freebsd-cross.sh's clang wrapper (its sysroot and target) and LLVM's binutils; lld links the
        # configure scripts' test programs.
        : "${CC_x86_64_unknown_freebsd:?run tools/freebsd-cross.sh first}"
        cc=$CC_x86_64_unknown_freebsd
        export CC=$cc AR=llvm-ar RANLIB=llvm-ranlib STRIP=llvm-strip NM=llvm-nm
        x264_flags+=(--host=x86_64-unknown-freebsd --extra-ldflags=-fuse-ld=lld --disable-asm)
        ffmpeg_flags+=(
            --target-os=freebsd --enable-cross-compile --arch=x86_64 --cc="$cc" --cxx="$cc"
            --ar="$AR" --nm="$NM" --ranlib="$RANLIB" --strip="$STRIP" --extra-ldflags=-fuse-ld=lld --disable-x86asm
        )
        ;;
    windows-x86_64 | windows-arm64)
        command -v cl >/dev/null || { echo "cl is not on PATH: open the Visual Studio environment first" >&2; exit 2; }
        # MSYS2's coreutils `link` shadows MSVC's linker.
        PATH="$(dirname "$(command -v cl)"):$PATH"
        if [ "$target" = windows-x86_64 ]; then arch=x86_64; else arch=aarch64; fi
        # Plain C (no nasm, no armasm), and the DLL C runtime (/MD) Rust's MSVC targets link.
        export CC=cl
        x264_flags+=(--host="$arch-w64-mingw32" --extra-cflags=-MD --disable-asm)
        ffmpeg_flags+=(--toolchain=msvc --target-os=win32 --arch="$arch" --extra-cflags=-MD --disable-asm)
        threads=--enable-w32threads
        ;;
    *)
        echo "unknown target $target" >&2
        exit 2
        ;;
esac

echo "== x264 ($target)"
cd "$x264_src"
make distclean >/dev/null 2>&1 || true
./configure --prefix="$out" --enable-static --enable-pic --disable-cli --disable-opencl --disable-lavf \
    --disable-swscale --disable-ffms --disable-gpac --disable-lsmash --bit-depth=8 --chroma-format=420 \
    "${x264_flags[@]}"
make -j"$jobs"
make install-lib-static
# MSVC's x264 is libx264.lib; FFmpeg's configure (-lx264) and rustc (static=x264) look for x264.lib.
if [ -f "$out/lib/libx264.lib" ]; then
    cp "$out/lib/libx264.lib" "$out/lib/x264.lib"
fi

echo "== FFmpeg $FFMPEG_VERSION ($target)"
cd "$ffmpeg_src"
make distclean >/dev/null 2>&1 || true
export PKG_CONFIG_PATH=$pc PKG_CONFIG_LIBDIR=$pc
./configure --prefix="$out" --pkg-config=pkg-config --pkg-config-flags=--static \
    --extra-cflags="-I$out/include" --extra-ldflags="-L$out/lib" \
    --enable-static --disable-shared --enable-pic --enable-gpl --enable-libx264 \
    --disable-programs --disable-doc --disable-debug --disable-autodetect --disable-network \
    "$threads" \
    --disable-everything --disable-avdevice --disable-avfilter --disable-postproc \
    --enable-avcodec --enable-avformat --enable-swscale --enable-swresample \
    --enable-protocol=file \
    --enable-demuxer=mov,matroska,mp3,aac,ogg,wav \
    --enable-decoder=h264,hevc,vp8,vp9,aac,mp3,mp3float,opus,vorbis,flac,pcm_s16le,pcm_s24le,pcm_f32le \
    --enable-parser=h264,hevc,vp8,vp9,av1,aac,mpegaudio,opus,vorbis,flac \
    --enable-bsf=h264_mp4toannexb,hevc_mp4toannexb,extract_extradata,aac_adtstoasc,vp9_superframe_split \
    --enable-muxer=mp4 --enable-encoder=aac,libx264 \
    "${ffmpeg_flags[@]}"
make -j"$jobs"
make install
# FFmpeg's MSVC build names its static libraries libavformat.a and so on; rustc looks for avformat.lib.
case "$target" in
    windows-*)
        for lib in avformat avcodec swscale swresample avutil; do
            cp "$out/lib/lib$lib.a" "$out/lib/$lib.lib"
        done
        ;;
esac
# What the build is, for the cache and for anyone reading the folder.
echo "FFmpeg $FFMPEG_VERSION, x264 $X264_COMMIT, $target" >"$out/BUILD"
