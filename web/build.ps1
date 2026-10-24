$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
cargo build --release --target wasm32-unknown-unknown --bin bomberpengu
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Remove-Item dist -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory dist | Out-Null
Copy-Item web\index.html, web\gl.js, web\audio.js, web\bp_net.js, web\config.js, target\wasm32-unknown-unknown\release\bomberpengu.wasm dist\
Copy-Item assets dist\assets -Recurse
Remove-Item dist\assets\assets.md -ErrorAction SilentlyContinue
Write-Host 'dist/ is ready: upload it to any static web host.'
