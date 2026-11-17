#!/usr/bin/env bash
# The sound library the release builds package (sounds/honeydew-sounds.zip.NNN). When this checkout has no tracked
# parts, the published library is downloaded as one part instead: honeydew-$SOUNDS_TAG-sounds.zip from
# $SOUNDS_REPO's release (the app's own sound downloads come from there too: src/update.rs `SOUNDS_REPO`). That zip
# is the library without the web's gzipped copies; the web build then serves the plain parts.
set -euo pipefail
cd "$(dirname "$0")/.."
if compgen -G 'sounds/honeydew-sounds.zip.[0-9][0-9][0-9]' >/dev/null; then
    echo "Using the tracked sound archive parts."
    exit 0
fi
: "${SOUNDS_REPO:?}" "${SOUNDS_TAG:?}"
url="https://github.com/$SOUNDS_REPO/releases/download/$SOUNDS_TAG/honeydew-$SOUNDS_TAG-sounds.zip"
echo "Downloading $url"
curl -sSfL --retry 3 -o sounds/honeydew-sounds.zip.000 "$url"
if [ -n "${GITHUB_ENV:-}" ]; then
    echo "SOUNDS_DOWNLOADED=1" >> "$GITHUB_ENV"
fi
