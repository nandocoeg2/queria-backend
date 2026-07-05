#!/bin/sh
set -e

if [ "$1" = "queria-api" ] || [ "$1" = "queria-mcp" ] || [ "$1" = "queria-worker" ] || [ "$1" = "queria-proxy" ] || [ "$1" = "queria-cli" ]; then
    binary="/usr/local/bin/$1"
    shift
    exec "$binary" "$@"
else
    exec "$@"
fi
