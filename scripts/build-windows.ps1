# Build and test MeshMind on Windows (MinGW/GNU toolchain)
# Run from project root. Requires: rustup, MSYS2 MinGW (C:\msys64\mingw64\bin in PATH)

$ErrorActionPreference = "Stop"
$env:PATH = "C:\msys64\mingw64\bin;" + $env:PATH
$target = "x86_64-pc-windows-gnu"

Write-Host "Building and testing for $target..."
cargo test --workspace --target $target
