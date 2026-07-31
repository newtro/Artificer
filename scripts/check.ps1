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

Write-Host "== WASM safety: artificer_assets must not pull in the model parsers =="
# artificer_assets compiles into the browser client. The parsers live in
# artificer_assets_import, and the ONE thing keeping an FBX reader out of the
# bundle is that this dependency edge does not exist. Assert it rather than
# trusting it.
$assetsTree = cargo tree -p artificer_assets 2>&1 | Out-String
foreach ($forbidden in @("ufbx", "artificer_assets_import")) {
    if ($assetsTree -match [regex]::Escape($forbidden)) {
        Write-Host "FAIL: artificer_assets depends on '$forbidden' - that reaches the WASM bundle"
        exit 1
    }
}

Write-Host "== sample generality test (samples/minimal builds with public APIs only) =="
cargo build -p minimal
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "ENGINE CHECK: PASS"
