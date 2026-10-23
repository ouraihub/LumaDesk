param([switch]$Verify)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$dist = Join-Path $root 'dist'
$portable = Join-Path $dist 'LumaDesk'
$exe = Join-Path $portable 'LumaDesk.exe'
$manifestPath = Join-Path $dist 'release.json'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Hash([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Check-Package([string]$Path, [string]$ExpectedHash) {
    $archive = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        if ($archive.Entries.Count -ne 1 -or $archive.Entries[0].FullName -ne 'LumaDesk/LumaDesk.exe') { throw 'Portable ZIP must contain only LumaDesk/LumaDesk.exe; never package user data.' }
        $stream = $archive.Entries[0].Open()
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $actual = ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $stream.Dispose(); $sha.Dispose() }
        if ($actual -ne $ExpectedHash) { throw 'Portable executable hash mismatch.' }
    } finally { $archive.Dispose() }
}
function Check-Release {
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ((Hash $exe) -ne $manifest.exeSha256) { throw 'Published executable hash mismatch.' }
    if (@($manifest.artifacts).Count -ne 1 -or $manifest.artifacts[0].kind -ne 'portable') { throw 'Expected one portable artifact.' }
    $artifact = $manifest.artifacts[0]
    $path = [IO.Path]::GetFullPath((Join-Path $dist $artifact.path))
    if (!$path.StartsWith($dist + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Artifact path escaped dist.' }
    if ((Hash $path) -ne $artifact.sha256) { throw 'Portable ZIP hash mismatch.' }
    Check-Package $path $manifest.exeSha256
    Write-Host "Verified: $exe"
    Write-Host "Verified: $path"
}

if ($Verify) { Check-Release; exit 0 }
Push-Location $root
try {
    $metadataJson = & cargo +1.99.0 metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq 'luma-desk' })
    if ($package.Count -ne 1) { throw 'Expected exactly one luma-desk package.' }
    $version = $package[0].version
    & cargo +1.99.0 build --release --locked --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed; previous published files unchanged.' }
    $source = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc/release/luma-desk.exe'
    $header = [IO.File]::OpenRead($source)
    try { if ($header.ReadByte() -ne 77 -or $header.ReadByte() -ne 90) { throw 'Built file is not a Windows executable.' } }
    finally { $header.Dispose() }
    $id = $version + '-' + [Guid]::NewGuid().ToString('N')
    $releases = Join-Path $dist 'installer'
    $stage = Join-Path $releases ('.stage-' + $id)
    $final = Join-Path $releases $id
    New-Item -ItemType Directory -Force -Path $stage, $portable | Out-Null
    $zipName = 'LumaDesk_' + $version + '_x64-portable.zip'
    $zip = Join-Path $stage $zipName
    $archive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Create)
    try { [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $source, 'LumaDesk/LumaDesk.exe', [IO.Compression.CompressionLevel]::Optimal) | Out-Null }
    finally { $archive.Dispose() }
    $exeHash = Hash $source
    Check-Package $zip $exeHash
    $manifest = [ordered]@{
        version = $version; buildId = $id; createdAt = [DateTime]::UtcNow.ToString('o'); exeSha256 = $exeHash
        artifacts = @(@{ kind = 'portable'; path = "installer/$id/$zipName"; sha256 = (Hash $zip) })
    }
    $exeTemp = Join-Path $portable ('.LumaDesk-' + $id + '.exe')
    $manifestTemp = Join-Path $dist ('.release-' + $id + '.json')
    Copy-Item -LiteralPath $source -Destination $exeTemp
    [IO.File]::WriteAllText($manifestTemp, ($manifest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
    Move-Item -LiteralPath $stage -Destination $final
    # Replace files only: never enumerate, copy, or remove the portable data directory.
    if (Test-Path -LiteralPath $exe) { [IO.File]::Replace($exeTemp, $exe, (Join-Path $final 'previous.exe')); Remove-Item -LiteralPath (Join-Path $final 'previous.exe') }
    else { [IO.File]::Move($exeTemp, $exe) }
    if (Test-Path -LiteralPath $manifestPath) { [IO.File]::Replace($manifestTemp, $manifestPath, (Join-Path $final 'previous-release.json')); Remove-Item -LiteralPath (Join-Path $final 'previous-release.json') }
    else { [IO.File]::Move($manifestTemp, $manifestPath) }
    Write-Host "Published: $exe"
    Write-Host "Package: $(Join-Path $final $zipName)"
} finally { Pop-Location }
