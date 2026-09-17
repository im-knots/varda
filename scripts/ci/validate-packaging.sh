#!/usr/bin/env bash
# Every packaging check that does not require a build.
# Usage: ./scripts/ci/validate-packaging.sh
#
# Run this before pushing. It takes seconds; the matrix it gates takes forty minutes per
# attempt, and most of what went wrong in this pipeline's history would have been caught
# here: a stray token in a PKGBUILD, a manifest that does not lint, a desktop file that
# does not validate, a Cargo.lock out of step with Cargo.toml.
#
# Checks that need a distribution or a built artifact belong in the smoke scripts. This is
# only what can be decided from the repository.
set -uo pipefail

cd "$(cd "$(dirname "$0")/../.." && pwd)"

fail=0
skipped=0
pass() { printf '  \033[32mok\033[0m   %s\n' "$1"; }
bad()  { printf '  \033[31mFAIL\033[0m %s\n' "$1"; fail=1; }
skip() { printf '  \033[33m--\033[0m   %s (%s)\n' "$1" "$2"; skipped=$((skipped + 1)); }

# A missing tool is not a failing check. Reporting them as the same thing is a bug this
# pipeline has shipped three times, in the ldconfig check, the ONNX check and here.
have() { command -v "$1" >/dev/null 2>&1; }
have_py() { python3 -c "import $1" >/dev/null 2>&1; }

echo "==> Shell scripts parse"
for f in scripts/ci/*.sh; do
  if bash -n "$f" 2>/dev/null; then pass "$f"; else bad "$f"; fi
done

echo "==> Python scripts compile"
for f in packaging/flatpak/*.py; do
  [ -e "$f" ] || continue
  if python3 -m py_compile "$f" 2>/dev/null; then pass "$f"; else bad "$f"; fi
done

echo "==> YAML parses"
if have_py yaml; then
  for f in .github/workflows/*.yml packaging/flatpak/*.yml; do
    [ -e "$f" ] || continue
    if python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1]))" "$f" 2>/dev/null; then
      pass "$f"
    else
      bad "$f"
      python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1]))" "$f" 2>&1 \
        | tail -3 | sed 's/^/       /'
    fi
  done
else
  skip "YAML parsing" "python3 has no yaml module: pip install pyyaml"
fi

echo "==> Generated PKGBUILD is well formed"
# bash -n is not enough on its own: a stray token inside a function body is syntactically
# a valid command invocation and only fails when that function runs. It is still worth
# doing, and makepkg --printsrcinfo in the build catches the rest.
TMP_PKGBUILD="$(mktemp)"
sed -e 's/@VERSION@/0.0.0/g' -e 's/@SHA256@/0/g' \
    -e 's|@SRCURL@|https://example.invalid/x.tar.gz|g' -e 's/@SRCDIR@/varda-0.0.0/g' \
  packaging/linux/PKGBUILD.in > "$TMP_PKGBUILD"
if bash -n "$TMP_PKGBUILD" 2>/dev/null; then pass "PKGBUILD.in"; else bad "PKGBUILD.in"; fi
# Catch the paste artifact class: a line that is a bare number or single stray word
# outside a comment, which bash accepts and then fails on at runtime.
if grep -nE '^[[:space:]]*[0-9]+[[:space:]]*$' "$TMP_PKGBUILD" >/dev/null; then
  bad "PKGBUILD.in has a line containing only a number (stray token)"
  grep -nE '^[[:space:]]*[0-9]+[[:space:]]*$' "$TMP_PKGBUILD" | sed 's/^/       /'
else
  pass "PKGBUILD.in has no stray bare-token lines"
fi
rm -f "$TMP_PKGBUILD"

echo "==> Desktop entry validates"
DESKTOP=packaging/linux/io.github.im_knots.varda.desktop
if have desktop-file-validate; then
  DF_OUT="$(desktop-file-validate "$DESKTOP" 2>&1 || true)"
  if printf '%s' "$DF_OUT" | grep -qE 'error:|warning:'; then
    bad "$DESKTOP"
    printf '%s\n' "$DF_OUT" | grep -E 'error:|warning:' | sed 's/^/       /'
  else
    pass "$DESKTOP"
    # Hints are style advice, not spec violations. Shown, never fatal.
    if printf '%s' "$DF_OUT" | grep -q 'hint:'; then
      printf '%s\n' "$DF_OUT" | grep 'hint:' | sed 's/^/       note: /'
    fi
  fi
else
  skip "$DESKTOP" "desktop-file-validate not installed"
fi

echo "==> AppStream metadata validates"
METAINFO=packaging/linux/varda.metainfo.xml
if python3 -c "import xml.etree.ElementTree as E,sys; E.parse(sys.argv[1])" "$METAINFO" 2>/dev/null; then
  pass "$METAINFO is well-formed XML"
else
  bad "$METAINFO is not well-formed XML"
fi
if command -v appstreamcli >/dev/null 2>&1; then
  # The template carries placeholders, so validate a substituted copy.
  TMP_META="$(mktemp -d)/varda.metainfo.xml"
  sed -e 's/@VERSION@/0.0.0/' -e 's/@DATE@/2000-01-01/' "$METAINFO" > "$TMP_META"
  if appstreamcli validate --no-net "$TMP_META" >/dev/null 2>&1; then
    pass "$METAINFO passes appstreamcli"
  else
    bad "$METAINFO fails appstreamcli"
    appstreamcli validate --no-net "$TMP_META" 2>&1 | sed 's/^/       /' | head -20
  fi
  rm -rf "$(dirname "$TMP_META")"
else
  skip "appstreamcli validation" "not installed"
fi

echo "==> Identifiers agree"
APPID=io.github.im_knots.varda
for f in "$DESKTOP" packaging/flatpak/$APPID.yml; do
  [ -e "$f" ] || { bad "$f is missing"; continue; }
done
META_ID="$(python3 -c "
import xml.etree.ElementTree as E
print(E.parse('$METAINFO').getroot().findtext('id'))" 2>/dev/null)"
if have_py yaml; then
  MANIFEST_ID="$(python3 -c "import yaml;print(yaml.safe_load(open('packaging/flatpak/$APPID.yml'))['app-id'])" 2>/dev/null)"
  if [ "$MANIFEST_ID" = "$APPID" ] && [ "$META_ID" = "$APPID" ]; then
    pass "app-id consistent across manifest, metainfo and filenames ($APPID)"
  else
    bad "app-id mismatch: manifest=$MANIFEST_ID metainfo=$META_ID expected=$APPID"
  fi
elif [ "$META_ID" = "$APPID" ]; then
  pass "app-id in metainfo matches ($APPID)"
  skip "app-id in the flatpak manifest" "no yaml module"
else
  bad "app-id mismatch: metainfo=$META_ID expected=$APPID"
fi

echo "==> Desktop entry and AppStream agree on categories"
DESKTOP_CATS="$(grep '^Categories=' "$DESKTOP" | cut -d= -f2 | tr ';' '\n' | grep -v '^$' | sort | tr '\n' ' ')"
META_CATS="$(python3 -c "
import xml.etree.ElementTree as E
r = E.parse('$METAINFO').getroot()
cats = r.find('categories')
print(' '.join(sorted(c.text for c in cats)) if cats is not None else '')" 2>/dev/null)"
if [ "$(echo $DESKTOP_CATS)" = "$(echo $META_CATS)" ]; then
  pass "categories match ($(echo $DESKTOP_CATS))"
else
  bad "categories differ: desktop='$(echo $DESKTOP_CATS)' metainfo='$(echo $META_CATS)'"
fi

echo "==> Versions agree"
CT="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
CL="$(grep -A1 '^name = "varda"$' Cargo.lock | grep '^version' | cut -d'"' -f2)"
if [ "$CT" = "$CL" ]; then
  pass "Cargo.toml and Cargo.lock both $CT"
else
  bad "Cargo.toml is $CT but Cargo.lock is $CL (run: cargo check --offline)"
fi

echo "==> Flatpak linter exceptions are narrow"
# flatpak-builder-lint enforces Flathub submission policy as well as correctness, and
# Varda does not submit to Flathub, so a small exceptions file is legitimate. What is not
# legitimate is excepting a check that reports a real defect.
#
# manifest-json-warnings is the specific trap: it looks like linter noise and is how
# flatpak-builder reports that it could not load cargo-sources.json. Excepting it would
# permanently blind the only check that reads the whole manifest.
EXC="packaging/flatpak/linter-exceptions.json"
if ! have python3; then
  skip "flatpak linter exceptions" "no python3"
elif [ ! -f "$EXC" ]; then
  bad "$EXC is missing; the flatpak build passes it to --user-exceptions"
else
  EXC_OUT="$(python3 - "$EXC" "$APPID" <<'PYEOF'
import json, sys
path, appid = sys.argv[1], sys.argv[2]
never = {"manifest-json-warnings", "appstream-failed-validation", "desktop-file-failed-validation"}
try:
    data = json.load(open(path))
except Exception as e:
    print("BAD does not parse: %s" % e); raise SystemExit(0)
listed = [x for x in data.get(appid, []) if isinstance(x, str)]
if not listed:
    print("BAD no exceptions listed for %s; the file would silently do nothing" % appid)
elif "*" in listed:
    print("BAD '*' disables the linter entirely")
elif set(listed) & never:
    print("BAD excepts a check that reports real defects: %s" % ", ".join(sorted(set(listed) & never)))
else:
    print("OK %d exception(s): %s" % (len(listed), ", ".join(listed)))
PYEOF
)"
  if [ "${EXC_OUT%% *}" = "OK" ]; then
    pass "${EXC_OUT#OK }"
  else
    bad "$EXC ${EXC_OUT#BAD }"
  fi
fi

echo "==> Shader path constant matches packaging"
# The packages, the AppDir and the Flatpak all install shaders relative to the binary.
# If this constant and the install paths drift, Varda starts with an empty library.
if grep -q 'BUNDLED_SHADERS_RELATIVE: &str = "../share/varda/shaders"' src/internal/registry/mod.rs; then
  pass "BUNDLED_SHADERS_RELATIVE is ../share/varda/shaders on Linux"
else
  bad "BUNDLED_SHADERS_RELATIVE changed; check every Linux install path"
fi

echo
if [ "$fail" -ne 0 ]; then
  echo "packaging validation FAILED"
  exit 1
fi
if [ "$skipped" -gt 0 ]; then
  echo "packaging validation passed ($skipped check(s) skipped for missing tools)"
  echo "  CI installs all of them; locally, the skips are informational."
else
  echo "packaging validation passed"
fi
