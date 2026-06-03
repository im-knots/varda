#!/usr/bin/env bash
# Cross-compile FFmpeg shared libraries for x86_64 on arm64 macOS
# Produces: /tmp/ffmpeg-x86_64/lib/*.dylib
#
# Uses clang -arch x86_64 (no Rosetta emulation needed)
set -euo pipefail

PREFIX="/tmp/ffmpeg-x86_64"
BUILDDIR="/tmp/ffmpeg-x86_64-build"
NPROC=$(sysctl -n hw.ncpu)

export CC="clang -arch x86_64"
export CXX="clang++ -arch x86_64"
export CFLAGS="-arch x86_64 -mmacosx-version-min=10.15"
export CXXFLAGS="-arch x86_64 -mmacosx-version-min=10.15"
export LDFLAGS="-arch x86_64 -mmacosx-version-min=10.15"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"

rm -rf "$BUILDDIR"
mkdir -p "$BUILDDIR" "$PREFIX"

echo "==> Building x86_64 FFmpeg dependencies + FFmpeg"
echo "    prefix: $PREFIX"
echo "    cpus: $NPROC"

# --- nasm (needed for x264/x265 asm optimizations) ---
echo "==> [1/5] Building nasm..."
cd "$BUILDDIR"
curl -sL https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/nasm-2.16.03.tar.gz | tar xz
cd nasm-2.16.03
# nasm needs native compiler to build itself, then produces x86_64 output
CC=clang CFLAGS="" LDFLAGS="" ./configure --prefix="$PREFIX"
make -j"$NPROC"
make install
export PATH="$PREFIX/bin:$PATH"

# --- x264 ---
echo "==> [2/5] Building x264 (x86_64)..."
cd "$BUILDDIR"
git clone --depth 1 https://code.videolan.org/videolan/x264.git
cd x264
./configure \
  --prefix="$PREFIX" \
  --host=x86_64-apple-darwin \
  --enable-shared \
  --disable-cli \
  --extra-cflags="$CFLAGS" \
  --extra-ldflags="$LDFLAGS"
make -j"$NPROC"
make install

# --- x265 ---
echo "==> [3/5] Building x265 (x86_64)..."
cd "$BUILDDIR"
git clone --depth 1 -b Release_4.1 https://bitbucket.org/multicoreware/x265_git.git
cd x265_git/build
cmake ../source \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DCMAKE_OSX_ARCHITECTURES=x86_64 \
  -DCMAKE_C_FLAGS="$CFLAGS" \
  -DCMAKE_CXX_FLAGS="$CXXFLAGS" \
  -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
  -DBUILD_SHARED_LIBS=ON \
  -DENABLE_CLI=OFF
make -j"$NPROC"
make install

# --- libsrt ---
echo "==> [4/5] Building libsrt (x86_64)..."
cd "$BUILDDIR"
git clone --depth 1 -b v1.5.4 https://github.com/Haivision/srt.git
cd srt
cmake . \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DCMAKE_OSX_ARCHITECTURES=x86_64 \
  -DCMAKE_C_FLAGS="$CFLAGS" \
  -DCMAKE_CXX_FLAGS="$CXXFLAGS" \
  -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
  -DENABLE_SHARED=ON \
  -DENABLE_STATIC=OFF \
  -DENABLE_APPS=OFF
make -j"$NPROC"
make install

# --- FFmpeg ---
echo "==> [5/5] Building FFmpeg (x86_64)..."
cd "$BUILDDIR"
curl -sL https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.gz | tar xz
cd ffmpeg-7.1.1
./configure \
  --prefix="$PREFIX" \
  --enable-cross-compile \
  --arch=x86_64 \
  --cc="clang -arch x86_64" \
  --cxx="clang++ -arch x86_64" \
  --extra-cflags="-I$PREFIX/include $CFLAGS" \
  --extra-ldflags="-L$PREFIX/lib $LDFLAGS" \
  --enable-shared \
  --disable-static \
  --disable-programs \
  --disable-doc \
  --enable-gpl \
  --enable-libx264 \
  --enable-libx265 \
  --enable-libsrt \
  --enable-protocol=srt \
  --enable-videotoolbox \
  --enable-audiotoolbox
make -j"$NPROC"
make install

echo ""
echo "==> x86_64 FFmpeg libs installed to $PREFIX/lib/"
ls -la "$PREFIX/lib/"*.dylib 2>/dev/null || ls -la "$PREFIX/lib/"*.dylib.*
echo ""
echo "==> Verify architecture:"
file "$PREFIX/lib/libavcodec"*.dylib | head -3
