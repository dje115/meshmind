# MeshMind Windows Setup

The project pins `x86_64-pc-windows-gnu` via `rust-toolchain.toml`. You need MinGW.

## Prerequisites

MeshMind requires Rust and a C linker. On Windows:

### 1. Install MSYS2 and MinGW

1. **Install MSYS2**: https://www.msys2.org/
2. Open **MSYS2 MinGW 64-bit** (or MSYS2 UCRT 64-bit) and run:
   ```bash
   pacman -S mingw-w64-x86_64-gcc
   ```
   For UCRT: `pacman -S mingw-w64-ucrt-x86_64-gcc`
3. **Add to system PATH**:
   - Classic MinGW: `C:\msys64\mingw64\bin`
   - UCRT: `C:\msys64\ucrt64\bin`

### 2. Install Rust toolchain

```bash
rustup target add x86_64-pc-windows-gnu
```

The project uses `rust-toolchain.toml` to select this target automatically.

### 3. Install OCR tools (for scanned PDF support)

Document ingestion uses OCR when PDF text extraction yields little content. Install via winget:

```powershell
winget install -e --id UB-Mannheim.TesseractOCR
winget install -e --id oschwartz10612.Poppler
```

Restart your terminal (or IDE) so PATH updates. Verify:

```powershell
pdftoppm -v
tesseract --version
```

### 4. Build and test

Ensure MinGW is first in PATH, then:

```powershell
$env:PATH = "C:\msys64\mingw64\bin;" + $env:PATH
cargo test --workspace --target x86_64-pc-windows-gnu
```

Or use the helper script:

```powershell
.\scripts\build-windows.ps1
```

## Common errors

| Error | Cause | Fix |
|-------|-------|-----|
| `gcc.exe: program not found` | MinGW not in PATH | Add `C:\msys64\mingw64\bin` to PATH |
| `ld returned 53` | MinGW linker/DLL mismatch | Reinstall: `pacman -S mingw-w64-x86_64-gcc` in MSYS2 |
| `cannot open file 'msvcrt.lib'` | Using MSVC without proper env | Use GNU toolchain; ensure rust-toolchain.toml is present |
| `stdint.h: No such file or directory` | Using MSVC; Windows SDK include path not set | Use GNU toolchain (recommended) |
