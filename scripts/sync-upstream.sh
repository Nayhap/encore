#!/usr/bin/env bash
# Show what changed upstream since Encore's resolver was ported, and refresh
# the vendored yt-dlp-ejs bundles (D37). docs/upstream.md explains the rest.
#
#   scripts/sync-upstream.sh [<yt-dlp commit or branch> [<ejs version>]]
#
# Defaults: yt-dlp master and the ejs version that commit pins. Needs git
# and curl; SHA3-512 comes from OpenSSL 3 when it has it (macOS's LibreSSL
# doesn't), else from a small Rust example of encore-resolver (cargo). Runs
# no Python: upstream's files are only read. The bundles are replaced only when both hashes match the ones
# yt-dlp pins in jsc/_builtin/vendor/_info.py.
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
crate="$here/crates/encore-resolver"
pins="$crate/upstream.toml"
target=${1:-master}
want_ejs=${2:-}

sha3() { # sha3 <file>: the SHA3-512 hex digest
  if openssl dgst -sha3-512 -r /dev/null >/dev/null 2>&1; then
    openssl dgst -sha3-512 -r "$1" | cut -d' ' -f1
  else
    cargo run -q --manifest-path "$here/Cargo.toml" -p encore-resolver --example sha3 -- "$1"
  fi
}

pin() { # pin <section> <key>
  awk -v s="[$1]" -v k="$2" '$0==s{on=1;next} /^\[/{on=0} on && $1==k {gsub(/"/,"",$3); print $3}' "$pins"
}
old_commit=$(pin yt_dlp commit)
old_ejs=$(pin ejs version)
old_ejs_commit=$(pin ejs commit)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fetch() { # fetch <dir> <repo url> <refs...>
  local d=$1 url=$2; shift 2
  git init -q "$d"
  git -C "$d" remote add origin "$url"
  git -C "$d" fetch -q --depth 1 origin "$@"
}

echo "== yt-dlp: pinned $old_commit, target $target"
fetch "$work/yt-dlp" https://github.com/yt-dlp/yt-dlp "$old_commit"
git -C "$work/yt-dlp" fetch -q --depth 1 origin "$target"
new_commit=$(git -C "$work/yt-dlp" rev-parse FETCH_HEAD)
echo "   target is $new_commit ($(git -C "$work/yt-dlp" show -s --format=%cs FETCH_HEAD))"
yt=yt_dlp/extractor/youtube
files=("$yt/_base.py" "$yt/_video.py" "$yt/jsc/provider.py" "$yt/jsc/_director.py" "$yt/jsc/_builtin/ejs.py" "$yt/jsc/_builtin/quickjs.py" "$yt/jsc/_builtin/vendor/_info.py" "$yt/pot")
if [ "$old_commit" = "$new_commit" ]; then
  echo "   no change"
else
  echo
  echo "== Whole diffs of the files Encore ports (docs/upstream.md has the map)"
  git -C "$work/yt-dlp" diff --stat "$old_commit" "$new_commit" -- "${files[@]}"
  git -C "$work/yt-dlp" diff "$old_commit" "$new_commit" -- "${files[@]}"
fi

git -C "$work/yt-dlp" checkout -q "$new_commit"
echo
echo "== INNERTUBE_CLIENTS at the target (compare with crates/encore-resolver/src/base/innertube_clients.json)"
awk '/^INNERTUBE_CLIENTS = \{/{on=1} on{print NR": "$0} on && /^\}/{exit}' "$work/yt-dlp/$yt/_base.py"
echo
echo "== Default clients at the target"
grep -nE "_DEFAULT_[A-Z_]*CLIENTS|_WEBPAGE_CLIENTS" "$work/yt-dlp/$yt/_video.py" | head

info="$work/yt-dlp/$yt/jsc/_builtin/vendor/_info.py"
ejs=$(sed -nE "s/^VERSION = '([^']+)'/\1/p" "$info")
lib_hash=$(sed -nE "s/.*'yt.solver.lib.min.js': '([0-9a-f]+)'.*/\1/p" "$info")
core_hash=$(sed -nE "s/.*'yt.solver.core.min.js': '([0-9a-f]+)'.*/\1/p" "$info")
if [ -n "$want_ejs" ] && [ "$want_ejs" != "$ejs" ]; then
  echo "!! yt-dlp at $new_commit pins ejs $ejs, not $want_ejs: its hashes can't check $want_ejs. Stopping." >&2
  exit 1
fi
echo
echo "== yt-dlp-ejs: pinned $old_ejs, target pins $ejs"
# The vendored bundles are checked every time, not only when the version
# moves: a damaged or stale copy is replaced too.
have_lib=$(sha3 "$crate/vendor/ejs/yt.solver.lib.min.js")
have_core=$(sha3 "$crate/vendor/ejs/yt.solver.core.min.js")
if [ "$ejs" != "$old_ejs" ] || [ "$have_lib" != "$lib_hash" ] || [ "$have_core" != "$core_hash" ]; then
  [ "$ejs" = "$old_ejs" ] && echo "   vendor/ejs doesn't match the pinned hashes: fetching $ejs again"
  for f in yt.solver.lib.min.js yt.solver.core.min.js; do
    curl -fsSL -o "$work/$f" "https://github.com/yt-dlp/ejs/releases/download/$ejs/$f"
  done
  got_lib=$(sha3 "$work/yt.solver.lib.min.js")
  got_core=$(sha3 "$work/yt.solver.core.min.js")
  if [ "$got_lib" != "$lib_hash" ] || [ "$got_core" != "$core_hash" ]; then
    echo "!! the downloaded bundles don't match _info.py; nothing replaced." >&2
    exit 1
  fi
  cp "$work/yt.solver.lib.min.js" "$work/yt.solver.core.min.js" "$crate/vendor/ejs/"
  old_lib=$(pin ejs lib_sha3_512); old_core=$(pin ejs core_sha3_512)
  for f in "$pins" "$crate/src/jsc/ejs.rs"; do
    sed -i.bak -e "s/$old_lib/$lib_hash/" -e "s/$old_core/$core_hash/" "$f" && rm -f "$f.bak"
  done
  # git must keep them byte for byte (.gitattributes): no line-ending changes.
  sed -i.bak -e "/^\[ejs\]/,/^\[/ s/^version = \"$old_ejs\"/version = \"$ejs\"/" "$pins" && rm -f "$pins.bak"
  sed -i.bak -e "s/pub const EJS_VERSION: &str = \"$old_ejs\";/pub const EJS_VERSION: \&str = \"$ejs\";/" "$crate/src/jsc/ejs.rs" && rm -f "$crate/src/jsc/ejs.rs.bak"
  echo "   replaced vendor/ejs with $ejs (hashes verified) and updated upstream.toml and jsc/ejs.rs"
  echo "   check the bundles' licence banners against crates/encore-resolver/vendor/ejs/NOTICE"
else
  echo "   unchanged"
fi

echo
echo "== Solver fixtures: ejs tests.ts and yt-dlp test_ejs_integration.py at the target"
fetch "$work/ejs" https://github.com/yt-dlp/ejs main "$old_ejs_commit"
git -C "$work/ejs" fetch -q --depth 1 origin main
git -C "$work/ejs" checkout -q FETCH_HEAD
echo "   ejs main is $(git -C "$work/ejs" rev-parse HEAD); the pinned fixtures were read at $old_ejs_commit"
git -C "$work/ejs" diff --stat "$old_ejs_commit" HEAD -- src/yt/solver/test/tests.ts || true
grep -oE 'player: "[0-9a-f]{8}"' "$work/ejs/src/yt/solver/test/tests.ts" | sort -u | tr '\n' ' '; echo
grep -oE "Challenge\('[0-9a-f]{8}', Variant\.[a-z0-9_]+" "$work/yt-dlp/test/test_jsc/test_ejs_integration.py" | sort -u | tr '\n' ' '; echo
echo "   pinned: $(awk '/^players/' "$pins")"
echo
echo "Next: port what the diffs need, set [yt_dlp] commit = \"$new_commit\" in upstream.toml,"
echo "update the commit in base/innertube_clients.json's _upstream, run cargo test -p encore-resolver"
echo "and the live checks in docs/upstream.md."
