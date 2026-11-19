#!/usr/bin/env bash
# The release workflow's FFmpeg build (tools/build-ffmpeg.sh) for one target, quiet unless it fails: then the tail of
# its log and of the configure logs become error annotations, as the run's own logs are hard to get at. The sources
# are removed afterwards, so only <out-dir> is cached.
#
#   tools/ci-ffmpeg.sh <out-dir> <target>
set -uo pipefail
out=$1
target=$2
src=$(dirname "$out")/src
log=${RUNNER_TEMP:-/tmp}/ffmpeg-$target.log
mkdir -p "$(dirname "$out")"
# FFmpeg's configure finds x264 through pkg-config.
if ! command -v pkg-config >/dev/null && command -v brew >/dev/null; then
    brew install pkgconf >/dev/null
fi
note() {
    awk '{ gsub(/%/, "%25"); buf = (n ? buf "%0A" : "") $0; n++ }
         n == 40 { print "::error title=ffmpeg " t "::" buf; buf = ""; n = 0 }
         END { if (n) print "::error title=ffmpeg " t "::" buf }' t="$target"
}
if ! HONEYDEW_FFMPEG_SRC=$src bash "$(dirname "$0")/build-ffmpeg.sh" "$out" "$target" >"$log" 2>&1; then
    { tail -n 60 "$log"
      tail -n 40 "$src"/FFmpeg-*/ffbuild/config.log 2>/dev/null
      tail -n 20 "$src"/x264-*/config.log 2>/dev/null; } | note
    exit 1
fi
tail -n 3 "$log"
rm -rf "$src"
