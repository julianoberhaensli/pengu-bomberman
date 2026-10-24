#!/bin/sh
set -e
cd "$(dirname "$0")/.."
cargo build --release --target wasm32-unknown-unknown --bin bomberpengu
rm -rf dist
mkdir dist
cp web/index.html web/gl.js web/audio.js web/bp_net.js web/config.js target/wasm32-unknown-unknown/release/bomberpengu.wasm dist/
cp -r assets dist/assets
rm -f dist/assets/assets.md
echo 'dist/ is ready: upload it to any static web host.'
