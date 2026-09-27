#!/usr/bin/env bash
# Generate the scoop manifest for a released tag, on stdout.
#
#   ./scripts/gen-scoop.sh v0.3.0 > bucket/dok.json
#
# Same contract as gen-formula.sh: it reads the `.sha256` the release workflow
# uploads next to the archive, so the checksum always matches the artefact
# people download. This repo doubles as its own scoop bucket, which means
# nothing external runs `checkver` for it — the release workflow calls this
# instead, and the manifest stopped drifting four releases behind.
set -euo pipefail

TAG=${1:?usage: gen-scoop.sh <tag>   e.g. gen-scoop.sh v0.3.0}
VERSION=${TAG#v}
REPO=${REPO:-alsaadii98/dok}
BASE="https://github.com/$REPO/releases/download/$TAG"
ARCHIVE="dok-$VERSION-x86_64-pc-windows-msvc"

sum=$(curl -fsSL --retry 3 --retry-delay 2 "$BASE/$ARCHIVE.zip.sha256" | tr -d ' \n\r-')
if [ ${#sum} -ne 64 ]; then
  echo "error: no valid sha256 at $BASE/$ARCHIVE.zip.sha256 (got '${sum:0:80}')" >&2
  exit 1
fi

# The autoupdate block stays: it costs nothing and lets anyone who forks this
# bucket into a scoop org get excavator updates for free.
cat <<EOF
{
    "version": "$VERSION",
    "description": "Docker output, made readable - what eza is to ls",
    "homepage": "https://github.com/$REPO",
    "license": "MIT",
    "architecture": {
        "64bit": {
            "url": "$BASE/$ARCHIVE.zip",
            "hash": "$sum",
            "extract_dir": "$ARCHIVE"
        }
    },
    "bin": "dok.exe",
    "checkver": {
        "github": "https://github.com/$REPO"
    },
    "autoupdate": {
        "architecture": {
            "64bit": {
                "url": "https://github.com/$REPO/releases/download/v\$version/dok-\$version-x86_64-pc-windows-msvc.zip",
                "extract_dir": "dok-\$version-x86_64-pc-windows-msvc",
                "hash": {
                    "url": "\$url.sha256"
                }
            }
        }
    }
}
EOF
