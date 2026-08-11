#!/usr/bin/env bash
# Install the system shared libraries Playwright's Chromium needs, WITHOUT root,
# into a local (gitignored) prefix under ./playwright-deps.
#
# Why: `playwright install --with-deps chromium` needs root/apt to install the
# browser's system libs (glib, nss, atk, X libs, ...). On CI (root) that works
# and this script is a no-op. In a rootless sandbox it can't, so we download the
# .deb packages and extract them into a home-free, repo-local prefix and expose
# them via LD_LIBRARY_PATH. Nothing touches the system or $HOME.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIR="$HERE/playwright-deps"
PREFIX="$DIR/libs"
APT="$DIR/apt"

lib_path() { find "$PREFIX" -name '*.so*' -printf '%h\n' 2>/dev/null | sort -u | paste -sd:; }

BIN="$(find "$HOME/.cache/ms-playwright" -name chrome-headless-shell -type f 2>/dev/null | head -1 || true)"
if [ -z "${BIN:-}" ]; then
  echo "note: chromium not installed yet (run 'bunx playwright install chromium'); skipping lib bootstrap"
  exit 0
fi

# Nothing to do if the system already satisfies the browser (e.g. CI with root).
if ! ldd "$BIN" 2>/dev/null | grep -q "not found"; then
  echo "browser system libs already satisfied by the system"
  exit 0
fi

# Nothing to do if a previous local bootstrap already satisfies it.
if [ -d "$PREFIX" ] && ! LD_LIBRARY_PATH="$(lib_path)" ldd "$BIN" 2>/dev/null | grep -q "not found"; then
  echo "browser system libs already present in $PREFIX"
  exit 0
fi

echo "bootstrapping Chromium system libs into $PREFIX (no root)..."
mkdir -p "$APT/state/lists/partial" "$APT/cache/archives/partial" "$PREFIX"
: > "$APT/state/status"
O=(-o "Dir::State=$APT/state" -o "Dir::Cache=$APT/cache" -o "Dir::State::status=$APT/state/status")

apt-get "${O[@]}" update >/dev/null

# Top-level packages providing the libs Chromium links against, on Debian trixie.
TOP="libglib2.0-0t64 libnspr4 libnss3 libatk1.0-0t64 libatk-bridge2.0-0t64 \
libatspi2.0-0t64 libdbus-1-3 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 \
libgbm1 libxkbcommon0 libasound2t64"

# Full dependency closure, minus core libs we must NOT shadow (would break the ABI).
PKGS="$(apt-cache "${O[@]}" depends --recurse --no-recommends --no-suggests \
  --no-conflicts --no-breaks --no-replaces --no-enhances $TOP 2>/dev/null \
  | grep -E '^[a-zA-Z0-9]' | sort -u \
  | grep -Ev '^(libc6|libgcc-s1|libstdc\+\+6|gcc-.*-base)$')"

( cd "$APT/cache/archives" && apt-get "${O[@]}" download $PKGS >/dev/null )
for d in "$APT/cache/archives"/*.deb; do dpkg -x "$d" "$PREFIX"; done

if LD_LIBRARY_PATH="$(lib_path)" ldd "$BIN" 2>/dev/null | grep -q "not found"; then
  echo "WARNING: some libraries are still missing after bootstrap:"
  LD_LIBRARY_PATH="$(lib_path)" ldd "$BIN" | grep "not found" || true
  exit 1
fi
echo "browser system libs installed into $PREFIX"
