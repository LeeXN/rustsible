#!/usr/bin/env sh
set -eu

cargo build --release --locked "$@"
