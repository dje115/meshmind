# generate_certs.ps1 — Generate dev CA + node certificate using node_crypto
# Usage: .\generate_certs.ps1 [-OutputDir ./data/certs]

param(
    [string]$OutputDir = "./data/certs"
)

Write-Host "MeshMind Dev CA Generator"
Write-Host "========================="
Write-Host ""
Write-Host "Certificates are generated automatically when the node starts."
Write-Host "To generate manually, run:"
Write-Host ""
Write-Host "  cargo run -p node_app"
Write-Host ""
Write-Host "Certificates will be written to: $OutputDir"
Write-Host ""
Write-Host "The node_crypto crate handles:"
Write-Host "  - Self-signed CA generation (MeshMind Dev CA)"
Write-Host "  - Node certificate signed by the dev CA"
Write-Host "  - Node ID derived from certificate SHA-256 fingerprint"
Write-Host "  - mTLS ServerConfig and ClientConfig for peer communication"
