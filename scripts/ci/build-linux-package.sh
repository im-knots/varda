#!/usr/bin/env bash
# Build a native Linux package for the distribution this is running on.
# Usage: ./scripts/ci/build-linux-package.sh --version <x.y.z> [--outdir dist]
#
# Runs INSIDE a distribution container. The distribution is detected from
# /etc/os-release rather than passed in, so the build dependencies, the package format
# and the resulting filename all come from one source of truth.
#
# Produces: dist/varda_<version>_amd64_<target>.deb   (Debian family)
#           dist/varda-<version>-1.<target>.x86_64.rpm (RPM family)
#           dist/PKGBUILD                              (Arch: a source recipe, not a binary)
#
# Why native packages rather than one portable tarball: spec/release-strategy.md section
# 5. The short version is that the package manager computes the dependency closure
# (dpkg-shlibdeps, rpm find-requires), installs it, and security-patches it. Varda
# bundling those libraries meant shipping a frozen copy that no distribution could
# update, and getting the closure wrong shipped issue #134.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

VERSION=""
SOURCE_REF=""
OUTDIR="dist"
SKIP_DEPS=false
SKIP_BUILD=false
while [ $# -gt 0 ]; do
  case "$1" in
    --version)    VERSION="$2"; shift 2 ;;
    --source-ref) SOURCE_REF="$2"; shift 2 ;;
    --outdir)     OUTDIR="$2"; shift 2 ;;
    --skip-deps)  SKIP_DEPS=true; shift ;;
    --skip-build) SKIP_BUILD=true; shift ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

if [ -z "$VERSION" ]; then
  VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
fi
VERSION="${VERSION#v}"

# Read os-release fields in a subshell rather than sourcing the file into this one.
# /etc/os-release defines VERSION (e.g. "13 (trixie)"), NAME, PRETTY_NAME and a dozen
# other names, and sourcing it clobbered this script's own $VERSION with the
# distribution's, producing `Version: 13 (trixie)` in the control file. It is a
# third-party file that may define anything, so nothing from it reaches this shell
# except the three fields asked for by name.
os_release_field() {
  ( . /etc/os-release && eval "printf '%s' \"\${$1-}\"" )
}

DISTRO_ID="$(os_release_field ID)"
DISTRO_ID="${DISTRO_ID:-unknown}"
DISTRO_VER="$(os_release_field VERSION_ID)"
DISTRO_VER="${DISTRO_VER:-rolling}"
DISTRO_PRETTY="$(os_release_field PRETTY_NAME)"

# TARGET_ID goes in the filename so a user can tell which package is theirs, and FORMAT
# selects the packaging path below.
case "$DISTRO_ID" in
  debian)   FORMAT=deb;   TARGET_ID="debian${DISTRO_VER%%.*}" ;;
  ubuntu)   FORMAT=deb;   TARGET_ID="ubuntu${DISTRO_VER//./}" ;;
  fedora)   FORMAT=rpm;   TARGET_ID="fc${DISTRO_VER%%.*}" ;;
  opensuse*|sles) FORMAT=rpm; TARGET_ID="opensuse${DISTRO_VER%%.*}" ;;
  arch|cachyos|manjaro|endeavouros) FORMAT=arch; TARGET_ID="arch" ;;
  *) echo "::error::unsupported distribution '$DISTRO_ID' (see spec/release-strategy.md section 5)"; exit 1 ;;
esac

echo "==> ${DISTRO_PRETTY:-$DISTRO_ID $DISTRO_VER}"
echo "    version: $VERSION  format: $FORMAT  target: $TARGET_ID"

# --- Cargo features per target ---
# Independent builds let a target drop one feature rather than be dropped entirely
# (spec section 5, "Per-target feature degradation"). The bar is high: a feature goes only
# when the dependency cannot be obtained on that distribution at all. openSUSE is the one
# case today, where libfreenect is in no repository serving Leap 16. Arch gets it from the
# AUR (see below); Debian, Ubuntu and Fedora package it directly.
CARGO_ARGS=(--release)
case "$DISTRO_ID" in
  opensuse*|sles)
    echo "    note: building without 'depth' (libfreenect is not packaged for openSUSE)"
    CARGO_ARGS+=(--no-default-features --features face-detection,html,screen-capture)
    ;;
esac

# --- Build dependencies ---
# Kept here rather than in the workflow so the dependency list and the distribution
# detection above stay in one place.
#
# These are NOT just the README's build instructions. The README assumes a developer's
# machine or a GitHub runner image, where several things are already present that a bare
# distribution container does not have, so they are explicit here:
#
#   clang + libclang - bindgen needs libclang.so at build time (libspa-sys for the
#                      screen-capture PipeWire backend, among others).
#   OpenSSL headers  - openssl-sys resolves the system OpenSSL through pkg-config.
#                      Reached via native-tls, for the WebSocket/HTTP API stack.
#   python3          - mozjs_sys (SpiderMonkey, under the `html` feature) builds with it.
#
# Anything added here because a container build failed belongs in this list, not in a
# workflow step: the container is the only place the requirement is visible.
install_build_deps() {
  case "$DISTRO_ID" in
    debian|ubuntu)
      export DEBIAN_FRONTEND=noninteractive
      apt-get update
      apt-get install -y --no-install-recommends \
        build-essential cmake pkg-config curl ca-certificates git \
        dpkg-dev debhelper fakeroot lintian \
        clang libclang-dev libssl-dev python3 \
        libvulkan-dev \
        libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev \
        libsrt-gnutls-dev libasound2-dev libv4l-dev libfreenect-dev \
        libpipewire-0.3-dev libshaderc-dev \
        libwayland-dev libxkbcommon-dev libx11-dev libxrandr-dev libxi-dev libgtk-3-dev
      ;;
    fedora)
      dnf install -y \
        "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm"
      dnf install -y \
        gcc-c++ cmake pkgconf-pkg-config curl ca-certificates git rpm-build rpmdevtools \
        clang clang-devel openssl-devel python3 \
        vulkan-loader-devel \
        ffmpeg-devel srt-devel alsa-lib-devel libv4l-devel libfreenect-devel \
        pipewire-devel libshaderc-devel \
        wayland-devel libxkbcommon-devel libX11-devel libXrandr-devel libXi-devel gtk3-devel
      ;;
    opensuse*|sles)
      # Stock Leap ships no usable FFmpeg development packages; they live in Packman,
      # which is not configured by default. Without this the build fails at pkg-config,
      # not at link time, so add it before the first refresh.
      zypper --non-interactive --gpg-auto-import-keys addrepo -cfp 90 \
        "https://ftp.gwdg.de/pub/linux/misc/packman/suse/openSUSE_Leap_${DISTRO_VER}/" packman || true
      zypper --non-interactive --gpg-auto-import-keys refresh
      zypper --non-interactive install -t pattern devel_C_C++ || true
      zypper --non-interactive install \
        cmake pkgconf curl ca-certificates git rpm-build \
        clang clang-devel libopenssl-devel python3 \
        vulkan-devel \
        ffmpeg-7-libavcodec-devel ffmpeg-7-libavformat-devel ffmpeg-7-libavutil-devel \
        ffmpeg-7-libswscale-devel ffmpeg-7-libswresample-devel \
        srt-devel alsa-devel libv4l-devel pipewire-devel shaderc-devel \
        wayland-devel libxkbcommon-devel libX11-devel libXrandr-devel libXi-devel gtk3-devel
      ;;
    arch|cachyos|manjaro|endeavouros)
      pacman -Syu --noconfirm --needed \
        base-devel cmake pkgconf curl git \
        clang openssl python \
        vulkan-icd-loader \
        ffmpeg srt alsa-lib v4l-utils pipewire shaderc \
        wayland libxkbcommon libx11 libxrandr libxi gtk3
      # libfreenect is in no Arch repository. The container build needs it the same way
      # the PKGBUILD does, and for the same reason: `depth` links -lfreenect. This still
      # uses the AUR copy because it is only building the package here, not installing
      # one; the published PKGBUILD builds and statically links libfreenect itself, so a
      # user needs nothing from the AUR.
      "$SCRIPT_DIR/install-aur-package.sh" libfreenect
      ;;
  esac
}

if [ "$SKIP_DEPS" = false ]; then
  echo "==> Installing build dependencies"
  install_build_deps
fi

# Rust comes from rustup rather than the distribution, so every target builds with the
# same toolchain and a distribution's Rust lagging does not become a build failure.
if ! command -v cargo >/dev/null 2>&1; then
  echo "==> Installing Rust toolchain"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

if [ "$SKIP_BUILD" = false ]; then
  echo "==> Building release binary"
  cargo build "${CARGO_ARGS[@]}"
fi

# =============================================================================
# Stage the install tree (FHS)
# =============================================================================
# /usr/bin/varda resolves its shaders at ../share/varda/shaders
# (internal/registry/mod.rs, BUNDLED_SHADERS_RELATIVE). Keep the two in step.
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

install -Dm755 target/release/varda           "$STAGE/usr/bin/varda"
install -Dm644 packaging/linux/varda.desktop  "$STAGE/usr/share/applications/varda.desktop"
install -Dm644 assets/icon.png                "$STAGE/usr/share/icons/hicolor/256x256/apps/varda.png"
install -Dm644 LICENSE                        "$STAGE/usr/share/licenses/varda/LICENSE"
mkdir -p "$STAGE/usr/share/varda/shaders"
cp -r shaders/* "$STAGE/usr/share/varda/shaders/"

mkdir -p "$OUTDIR"

DESCRIPTION="Live visual mixer and router for VJ performance and installation art.
Sources (video, cameras, generative shaders, streams, images) flow through a routing
graph of decks, channels and surfaces to reach outputs (projectors, streams, recordings)."

case "$FORMAT" in

  # ---------------------------------------------------------------------------
  deb)
    echo "==> Building .deb"
    mkdir -p "$STAGE/DEBIAN"

    # dpkg-shlibdeps wants a source package context, so give it a minimal one. This is
    # the step that computes Depends from the binary's DT_NEEDED entries and maps each
    # SONAME to the package providing it. It is the closure walk, done properly.
    DEBDIR="$(mktemp -d)"
    mkdir -p "$DEBDIR/debian"
    cat > "$DEBDIR/debian/control" <<CTL
Source: varda
Section: video
Priority: optional
Maintainer: im-knots <noreply@users.noreply.github.com>
CTL
    touch "$DEBDIR/debian/substvars"
    SHLIB_DEPS="$(cd "$DEBDIR" && dpkg-shlibdeps -O --ignore-missing-info \
      "$STAGE/usr/bin/varda" 2>/dev/null | sed 's/^shlibs:Depends=//')"
    rm -rf "$DEBDIR"

    if [ -z "$SHLIB_DEPS" ]; then
      echo "::error::dpkg-shlibdeps computed no dependencies; refusing to ship a package that declares none"
      exit 1
    fi
    echo "    computed Depends: $SHLIB_DEPS"

    # Two runtime dependencies have no DT_NEEDED entry, so dpkg-shlibdeps structurally
    # cannot find them and they have to be declared by hand:
    #   ffmpeg    - SRT and recording output shell out to the executable
    #               (internal/renderer/subprocess.rs).
    #   libvulkan1 - wgpu dlopens libvulkan.so.1. Without it Varda starts and then cannot
    #               create a GPU device at all, which is the whole program.
    cat > "$STAGE/DEBIAN/control" <<CTL
Package: varda
Version: $VERSION
Architecture: amd64
Maintainer: im-knots <noreply@users.noreply.github.com>
Section: video
Priority: optional
Homepage: https://github.com/im-knots/varda
Depends: $SHLIB_DEPS, ffmpeg, libvulkan1
Description: Live visual mixer and router
$(echo "$DESCRIPTION" | sed 's/^/ /')
CTL

    PKG="$OUTDIR/varda_${VERSION}_amd64_${TARGET_ID}.deb"
    dpkg-deb --root-owner-group --build "$STAGE" "$PKG"
    dpkg-deb --info "$PKG"
    ;;

  # ---------------------------------------------------------------------------
  rpm)
    echo "==> Building .rpm"
    RPMTOP="$(mktemp -d)"
    mkdir -p "$RPMTOP"/{BUILD,RPMS,SOURCES,SPECS,BUILDROOT}

    # No %prep or %build: the binary is already built above. rpmbuild's job here is
    # packaging and, crucially, its automatic find-requires pass, which derives Requires
    # from the binary's SONAMEs the same way dpkg-shlibdeps does.
    cat > "$RPMTOP/SPECS/varda.spec" <<SPEC
# The binary is already built and stripped by cargo, so there is nothing for rpmbuild's
# debuginfo pass to work from. Left on, find-debuginfo runs anyway and fails the build on
# an empty debugsourcefiles.list, and its -debuginfo subpackage would also land a second
# .rpm in RPMS for the copy step below to pick up.
%global debug_package %{nil}
# With no debuginfo package, the build-id symlinks under /usr/lib/.build-id have nothing
# to point at and land unpackaged, which rpmbuild treats as an error.
%global _build_id_links none
Name:           varda
Version:        $VERSION
Release:        1%{?dist}
Summary:        Live visual mixer and router
License:        MIT
URL:            https://github.com/im-knots/varda
BuildArch:      x86_64
# Neither of these is derivable by rpm's find-requires: the ffmpeg executable is shelled
# out to for SRT and recording output, and libvulkan is dlopened by wgpu.
#
# Expressed as a file dependency and a soname dependency rather than package names,
# because the names differ across RPM distributions (vulkan-loader on Fedora, libvulkan1
# on openSUSE). rpm resolves both against whatever package actually provides them, so a
# naming difference stops being something this script has to know about.
Requires:       /usr/bin/ffmpeg
Requires:       libvulkan.so.1()(64bit)
%description
$DESCRIPTION
%install
rm -rf %{buildroot}
mkdir -p %{buildroot}
cp -a $STAGE/. %{buildroot}/
%files
/usr/bin/varda
/usr/share/varda
/usr/share/applications/varda.desktop
/usr/share/icons/hicolor/256x256/apps/varda.png
/usr/share/licenses/varda
%changelog
* $(LC_ALL=C date '+%a %b %d %Y') im-knots <noreply@users.noreply.github.com> - $VERSION-1
- Release $VERSION
SPEC

    rpmbuild --define "_topdir $RPMTOP" -bb "$RPMTOP/SPECS/varda.spec"

    mapfile -t built < <(find "$RPMTOP/RPMS" -name '*.rpm' ! -name '*debuginfo*' ! -name '*debugsource*')
    if [ "${#built[@]}" -ne 1 ]; then
      echo "::error::expected exactly one rpm, found ${#built[@]}:"
      printf '    %s\n' "${built[@]}"
      exit 1
    fi
    PKG="$OUTDIR/varda-${VERSION}-1.${TARGET_ID}.x86_64.rpm"
    cp "${built[0]}" "$PKG"
    echo "==> Requires:"
    rpm -qpR "$PKG" | sed 's/^/    /'
    rm -rf "$RPMTOP"
    ;;

  # ---------------------------------------------------------------------------
  arch)
    # Arch ships a source recipe, not a binary (spec section 5, decision 17): a binary
    # built against a rolling distribution's libraries goes stale as soon as a SONAME
    # moves, which is the failure this whole change is about. A PKGBUILD is rebuilt on
    # the user's machine against whatever they currently have.
    echo "==> Generating PKGBUILD"

    # Which ref the recipe fetches. A tag for a real release; a commit SHA for a dry run
    # on a pull request, where v<version> does not exist yet. GitHub names the directory
    # inside the archive after the ref, and makepkg has to cd into it, so derive both.
    REF="${SOURCE_REF:-v$VERSION}"
    case "$REF" in
      v[0-9]*)
        SRC_URL="https://github.com/im-knots/varda/archive/refs/tags/${REF}.tar.gz"
        # Literal, so the published recipe reads the way an Arch packager expects
        # rather than carrying a hardcoded version in two places.
        SRC_DIR='$pkgname-$pkgver'
        ;;
      *)
        SRC_URL="https://github.com/im-knots/varda/archive/${REF}.tar.gz"
        SRC_DIR="varda-${REF}"
        ;;
    esac

    echo "    ref:    $REF"
    echo "    source: $SRC_URL"
    # A real checksum rather than SKIP: AUR review rejects SKIP on a release tarball,
    # and computing it here proves the source archive is actually reachable.
    if SUM="$(curl -fsSL "$SRC_URL" | sha256sum | cut -d' ' -f1)" && [ -n "$SUM" ]; then
      echo "    sha256: $SUM"
    else
      echo "::error::could not fetch $SRC_URL to checksum it"
      exit 1
    fi

    sed -e "s/@VERSION@/$VERSION/g" \
        -e "s/@SHA256@/$SUM/g" \
        -e "s|@SRCURL@|$SRC_URL|g" \
        -e "s/@SRCDIR@/$SRC_DIR/g" \
      packaging/linux/PKGBUILD.in > "$OUTDIR/PKGBUILD"

    # `bash -n` on a PKGBUILD proves very little: a stray token inside a function body is
    # syntactically a valid command invocation and only fails when that function runs.
    # makepkg --printsrcinfo parses the recipe properly and is the real check that it is
    # well formed before it reaches a release. (It refuses to run as root.)
    bash -n "$OUTDIR/PKGBUILD"
    if id builder >/dev/null 2>&1; then
      su builder -c "cd '$PWD/$OUTDIR' && makepkg --printsrcinfo" > "$OUTDIR/.SRCINFO"
      echo "==> .SRCINFO"
      sed 's/^/    /' "$OUTDIR/.SRCINFO"
    fi

    cat "$OUTDIR/PKGBUILD"
    ;;
esac

echo "==> Done"
ls -lh "$OUTDIR"
