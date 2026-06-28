#!/bin/bash
# Guarantee a Node that satisfies the Angular CLI is on PATH for the session.
#
# The web sandbox image ships Node 22.22.2, but @angular/build requires
# >= 22.22.3 (or 24.15 / 26), so `ng build` / `ng test` refuse to run. This
# hook installs a current Node 22 via the image's nvm and symlinks it into
# ~/.local/bin, which already precedes the stock Node on PATH — so every shell
# the session spawns picks it up. Idempotent and quiet on the happy path.
set -euo pipefail

# Local machines have the user's own Node; only fix the remote sandbox.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

# Minimum the Angular CLI accepts.
need_major=22 need_minor=22 need_patch=3

# True if $1 (vX.Y.Z) is >= the required minimum.
node_ok() {
  local v="${1#v}" ma mi pa
  IFS=. read -r ma mi pa <<<"$v"
  [ -n "$ma" ] || return 1
  if [ "$ma" -ne "$need_major" ]; then [ "$ma" -gt "$need_major" ]; return; fi
  if [ "$mi" -ne "$need_minor" ]; then [ "$mi" -gt "$need_minor" ]; return; fi
  [ "$pa" -ge "$need_patch" ]
}

if command -v node >/dev/null 2>&1 && node_ok "$(node -v)"; then
  exit 0 # already good (e.g. container state cached from a prior session)
fi

export NVM_DIR=/opt/nvm
if [ ! -s "$NVM_DIR/nvm.sh" ]; then
  echo "[session-start] nvm not found at $NVM_DIR; cannot upgrade Node" >&2
  exit 0 # don't block the session — just surface the problem
fi
# shellcheck disable=SC1091
. "$NVM_DIR/nvm.sh"

nvm install 22 >/dev/null 2>&1
new_bin="$(dirname "$(nvm which 22)")"

# Drop symlinks where they win on PATH (no system dirs overwritten).
dest="$HOME/.local/bin"
mkdir -p "$dest"
for b in node npm npx; do
  [ -x "$new_bin/$b" ] && ln -sf "$new_bin/$b" "$dest/$b"
done

echo "[session-start] Node $("$dest/node" -v) ready for the Angular CLI"
