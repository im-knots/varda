#!/usr/bin/env python3
"""Extract the prebuilt ONNX Runtime static library for the Flatpak build.

ort-sys normally downloads this itself, which a Flatpak build sandbox forbids. The archive
is declared as a manifest source instead and unpacked here.

It needs unpacking by hand because it is a **raw LZMA2 stream**, not an xz-wrapped tarball:
ort-sys reads it with `lzma_rust2::Lzma2Reader::new(reader, 1 << 26, None)`, a 64 MiB
dictionary and no container. `tar`, `xz` and `unlzma` do not read that. Python's stdlib
does, with FORMAT_RAW and a matching filter spec, which is why this is a script rather than
a shell command.

Produces <destdir>/libonnxruntime.a, which is the filename ort-sys looks for under
ORT_LIB_LOCATION (build/static_link/mod.rs: `format!("lib{}.a", a)`).

Usage: extract-ort.py <archive.tar.lzma2> <destdir>
"""

import io
import lzma
import sys
import tarfile
from pathlib import Path

# Must match Lzma2Reader's dictionary size. A smaller value fails to decode; the format
# carries no header to negotiate it.
DICT_SIZE = 1 << 26
EXPECTED = "libonnxruntime.a"


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2

    archive = Path(sys.argv[1])
    destdir = Path(sys.argv[2])

    if not archive.is_file():
        print(f"error: {archive} does not exist", file=sys.stderr)
        return 1

    decompressor = lzma.LZMADecompressor(
        format=lzma.FORMAT_RAW,
        filters=[{"id": lzma.FILTER_LZMA2, "dict_size": DICT_SIZE}],
    )
    raw = decompressor.decompress(archive.read_bytes())

    destdir.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(raw)) as tar:
        members = tar.getnames()
        # Refuse paths that escape destdir. The archive is checksummed in the manifest, so
        # this is belt and braces rather than a live threat.
        for name in members:
            if name.startswith("/") or ".." in Path(name).parts:
                print(f"error: refusing unsafe member {name!r}", file=sys.stderr)
                return 1
        # filter="data" is the safe extraction mode and becomes the default in Python
        # 3.14, which the SDK may already ship. Passed explicitly so behaviour does not
        # change under us, with a fallback for interpreters older than 3.12.
        try:
            tar.extractall(destdir, filter="data")
        except TypeError:
            tar.extractall(destdir)

    produced = destdir / EXPECTED
    if not produced.is_file():
        print(
            f"error: expected {EXPECTED} in the archive, found {members}",
            file=sys.stderr,
        )
        return 1

    size_mb = produced.stat().st_size / 1024 / 1024
    print(f"extracted {EXPECTED} ({size_mb:.1f} MB) to {destdir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
