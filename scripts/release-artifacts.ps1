param([switch]$Verify)
$ErrorActionPreference = 'Stop'
# Cargo emits UTF-8 JSON; Windows PowerShell otherwise decodes native stdout with the OEM code page.
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$dist = Join-Path $root 'dist/gpui'
$portable = Join-Path $root 'dist/LumaDesk-GPUI'
$exe = Join-Path $portable 'LumaDesk-GPUI.exe'
$manifestPath = Join-Path $dist 'release.json'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Hash([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
    finally { $stream.Dispose(); $sha.Dispose() }
}
function Check-Package([string]$Path, [string]$ExpectedHash) {
    $archive = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $expected = @('LumaDesk-GPUI/LumaDesk-GPUI.exe', 'LumaDesk-GPUI/THIRD-PARTY.txt')
        if ($archive.Entries.Count -ne $expected.Count -or @($archive.Entries | Where-Object { $_.FullName -notin $expected }).Count -ne 0) { throw 'Portable ZIP must contain only the executable and distribution notices; never package user data.' }
        $stream = $archive.GetEntry('LumaDesk-GPUI/LumaDesk-GPUI.exe').Open()
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
    $metadataJson = & (Join-Path $root 'scripts/cargo-msvc.bat') metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq 'luma-desk' })
    if ($package.Count -ne 1) { throw 'Expected exactly one luma-desk package.' }
    $version = $package[0].version
    & (Join-Path $root 'scripts/cargo-msvc.bat') build --release --locked --target x86_64-pc-windows-msvc
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
    $notices = Join-Path $stage 'THIRD-PARTY.txt'
    $text = New-Object Text.StringBuilder
    [void]$text.AppendLine('LumaDesk GPUI - third-party software notices')
    foreach ($dependency in ($metadata.packages | Sort-Object name, version)) {
        [void]$text.AppendLine("`n=== $($dependency.name) $($dependency.version) ===")
        [void]$text.AppendLine("License: $($dependency.license)`nSource: $($dependency.repository)")
        $directory = Split-Path -Parent $dependency.manifest_path
        $licenseFiles = @(Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([.\-_]|$)' })
        if ($dependency.license_file) { $licenseFiles += Get-Item -LiteralPath (Join-Path $directory $dependency.license_file) }
        foreach ($license in ($licenseFiles | Sort-Object FullName -Unique)) {
            [void]$text.AppendLine("--- $($license.Name) ---")
            [void]$text.AppendLine([IO.File]::ReadAllText($license.FullName))
        }
    }
    $referencePath = Join-Path $root '../LumaKit/crates/lumakit-ai/src/model_reference.json'
    # Windows PowerShell JSON objects are case-insensitive; model IDs are not. Decode only the license string.
    $referenceJson = [IO.File]::ReadAllText($referencePath)
    $licenseMatch = [regex]::Match($referenceJson, '"license"\s*:\s*("(?:\\.|[^"\\])*")')
    if (!$licenseMatch.Success) { throw 'Bundled model catalog license is missing.' }
    $referenceLicense = $licenseMatch.Groups[1].Value | ConvertFrom-Json
    [void]$text.AppendLine("`n=== OMP bundled model reference catalog ===")
    [void]$text.AppendLine('Source: https://github.com/can1357/oh-my-pi (revision and source SHA-256 embedded in the application catalog)')
    [void]$text.AppendLine($referenceLicense)
    [IO.File]::WriteAllText($notices, $text.ToString(), (New-Object Text.UTF8Encoding $false))
    $zipName = 'LumaDesk-GPUI_' + $version + '_x64-portable.zip'
    $zip = Join-Path $stage $zipName
    $archive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Create)
    try {
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $source, 'LumaDesk-GPUI/LumaDesk-GPUI.exe', [IO.Compression.CompressionLevel]::Optimal) | Out-Null
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $notices, 'LumaDesk-GPUI/THIRD-PARTY.txt', [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
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
    if (Test-Path -LiteralPath $exe) { [IO.File]::Replace($exeTemp, $exe, [NullString]::Value) }
    else { [IO.File]::Move($exeTemp, $exe) }
    Copy-Item -LiteralPath (Join-Path $final 'THIRD-PARTY.txt') -Destination (Join-Path $portable 'THIRD-PARTY.txt')
    if (Test-Path -LiteralPath $manifestPath) { [IO.File]::Replace($manifestTemp, $manifestPath, (Join-Path $final 'previous-release.json')); Remove-Item -LiteralPath (Join-Path $final 'previous-release.json') }
    else { [IO.File]::Move($manifestTemp, $manifestPath) }
    Write-Host "Published: $exe"
    Write-Host "Package: $(Join-Path $final $zipName)"
} finally { Pop-Location }
