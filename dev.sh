#!/usr/bin/env bash
set -e

lang="$1"

case "$lang" in
    rust)
        nix develop .#rust
        ;;
    *)
        echo "Error: unknown env: '$lang'"
        exit 1
        ;;
esac
