#!/bin/sh
# Build the Alpine .apk from a released musl archive. Runs *inside* an Alpine
# container, so the package is produced by the same abuild that Alpine itself
# uses.
#
#   docker run --rm -v "$PWD:/w" -w /w alpine:3.20 \
#       scripts/build-apk.sh 0.1.1 x86_64 dist/dok-0.1.1-x86_64-unknown-linux-musl.tar.gz out
#
# The binary inside the archive is static musl, so packaging it needs no
# toolchain and the result has no dependencies.
#
# Signing: abuild always signs. With ALPINE_ABUILD_KEY set (a private key, in
# PEM), the package is signed with the project key and anyone who trusts the
# matching .rsa.pub can install it normally. Without it a throwaway key is
# generated, and the package installs with `apk add --allow-untrusted`.
set -eu

VERSION=${1:?usage: build-apk.sh <version> <arch> <tarball> <outdir>}
ARCH=${2:?missing arch}
TARBALL=${3:?missing tarball}
OUTDIR=${4:?missing outdir}

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARBALL=$(cd "$(dirname "$TARBALL")" && pwd)/$(basename "$TARBALL")
mkdir -p "$OUTDIR"
OUTDIR=$(cd "$OUTDIR" && pwd)

apk add --no-cache alpine-sdk >/dev/null

# abuild refuses to run as root unless forced, and wants a packager identity.
export PACKAGER="Ali H. Abdulhadi <a.h.alsaady1998@gmail.com>"
export ABUILD_USERDIR=/root/.abuild
mkdir -p "$ABUILD_USERDIR"

if [ -n "${ALPINE_ABUILD_KEY:-}" ]; then
	printf '%s\n' "$ALPINE_ABUILD_KEY" > "$ABUILD_USERDIR/dok.rsa"
	chmod 600 "$ABUILD_USERDIR/dok.rsa"
	openssl rsa -in "$ABUILD_USERDIR/dok.rsa" -pubout \
		-out "$ABUILD_USERDIR/dok.rsa.pub" 2>/dev/null
	echo "PACKAGER_PRIVKEY=\"$ABUILD_USERDIR/dok.rsa\"" > "$ABUILD_USERDIR/abuild.conf"
	cp "$ABUILD_USERDIR/dok.rsa.pub" /etc/apk/keys/
else
	echo "ALPINE_ABUILD_KEY not set - signing with a throwaway key" >&2
	# -i would install the public key for us, but it shells out to doas, which
	# a root-only container does not have. Copy it across by hand instead.
	abuild-keygen -a -n >/dev/null 2>&1
	cp "$ABUILD_USERDIR"/*.rsa.pub /etc/apk/keys/
fi

BUILD=/tmp/apkbuild
rm -rf "$BUILD"
mkdir -p "$BUILD"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@ARCH@/$ARCH/g" \
	"$REPO_ROOT/packaging/alpine/APKBUILD.in" > "$BUILD/APKBUILD"
cp "$TARBALL" "$BUILD/dok-$VERSION-$ARCH.tar.gz"

cd "$BUILD"
abuild -F checksum
abuild -F -r

# abuild drops the package under ~/packages/<dir>/<arch>/.
found=$(find /root/packages -name "dok-$VERSION-r0.apk" | head -1)
[ -n "$found" ] || { echo "no .apk produced" >&2; exit 1; }
cp "$found" "$OUTDIR/dok-$ARCH.apk"
# The checksum is written here, not by the caller: the output directory is
# owned by root inside the container, so a CI runner cannot add files to it.
sha256sum "$OUTDIR/dok-$ARCH.apk" | cut -d' ' -f1 > "$OUTDIR/dok-$ARCH.apk.sha256"
chmod -R a+rX "$OUTDIR"

# Prove the package installs and the binary runs before it reaches a release.
apk add --allow-untrusted "$OUTDIR/dok-$ARCH.apk" >/dev/null
dok --version
dok ps --demo --color=never --icons=none | head -1

echo "wrote $OUTDIR/dok-$ARCH.apk"
