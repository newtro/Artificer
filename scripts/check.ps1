# Engine CI-style check: format, lint, build, test — reproducible from clean checkout.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

Write-Host "== cargo fmt --check =="
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "== cargo clippy (workspace, all targets) =="
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "== cargo test (workspace) =="
cargo test --workspace
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "== sample generality test (samples/minimal builds with public APIs only) =="
cargo build -p minimal
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "ENGINE CHECK: PASS"
