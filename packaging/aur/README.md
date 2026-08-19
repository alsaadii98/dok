# AUR packages

Two packages, one source tree:

- `PKGBUILD` → **dok**, built from the release tarball with cargo.
- `PKGBUILD-bin` → **dok-bin**, the static musl binary from the same release,
  so no Rust toolchain is needed.

`SRCINFO-src` and `SRCINFO-bin` are the generated `.SRCINFO` files for each.
Both are produced by `makepkg --printsrcinfo`; regenerate them whenever a
PKGBUILD changes, because the AUR reads metadata from `.SRCINFO`, not from
the PKGBUILD.

## Build locally

```sh
makepkg -si -p PKGBUILD-bin   # prebuilt binary
makepkg -si                   # build from source
```

## Publish to the AUR

Needs an AUR account with an SSH public key registered at
<https://aur.archlinux.org/account/>. One repo per package name:

```sh
git clone ssh://aur@aur.archlinux.org/dok-bin.git /tmp/aur-dok-bin
cp packaging/aur/PKGBUILD-bin /tmp/aur-dok-bin/PKGBUILD
cp packaging/aur/SRCINFO-bin  /tmp/aur-dok-bin/.SRCINFO
cd /tmp/aur-dok-bin && git add PKGBUILD .SRCINFO && git commit -m "dok-bin 0.1.3" && git push
```

Same shape for `dok`, using `PKGBUILD` and `SRCINFO-src`. A brand-new package
name clones an empty repo; that is expected.

## On every release

Bump `pkgver`, reset `pkgrel=1`, and replace the checksums. The binary sums
are published as release assets:

```sh
v=0.1.3
base=https://github.com/alsaadii98/cool-docker-commands/releases/download/v$v
curl -sL $base/dok-$v-x86_64-unknown-linux-musl.tar.gz.sha256
curl -sL $base/dok-$v-aarch64-unknown-linux-musl.tar.gz.sha256
curl -sL https://github.com/alsaadii98/cool-docker-commands/archive/refs/tags/v$v.tar.gz | sha256sum
```

Then regenerate the `.SRCINFO` files and push. Both PKGBUILDs are verified by
building them inside `archlinux:base-devel` before release.
