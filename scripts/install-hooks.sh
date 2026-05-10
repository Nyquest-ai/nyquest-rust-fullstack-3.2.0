#!/usr/bin/env bash
# Install repo git hooks so formatting drift can't accumulate.
# Run once per fresh clone:
#
#     bash scripts/install-hooks.sh
#
# What it does:
#   - Symlinks scripts/hooks/* into .git/hooks/
#   - Hooks are tracked in-repo, so updates propagate to everyone who's installed them.

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo "Not inside a git repo." >&2
    exit 1
}

src_dir="$repo_root/scripts/hooks"
dst_dir="$repo_root/.git/hooks"

if [ ! -d "$src_dir" ]; then
    echo "Missing $src_dir" >&2
    exit 1
fi

mkdir -p "$dst_dir"

shopt -s nullglob
for hook_path in "$src_dir"/*; do
    name=$(basename "$hook_path")
    target="$dst_dir/$name"

    # Back up an existing non-symlink hook so we don't surprise anyone.
    if [ -e "$target" ] && [ ! -L "$target" ]; then
        mv "$target" "$target.local-backup"
        echo "Backed up existing $name -> $name.local-backup"
    fi

    ln -sf "$hook_path" "$target"
    chmod +x "$hook_path"
    echo "Installed: $name"
done

echo
echo "Done. Hooks installed in .git/hooks/. Test by editing a .rs file and committing."
