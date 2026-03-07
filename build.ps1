# MeshMind build script for Windows (MinGW)
# Ensures MSYS2 MinGW is in PATH before running cargo (fixes ld exit 53)
$mingwPath = "C:\msys64\mingw64\bin"
$usrPath = "C:\msys64\usr\bin"
if (Test-Path $mingwPath) {
    $env:PATH = "$mingwPath;$usrPath;$env:PATH"
}
& cargo @args
