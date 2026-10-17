$ErrorActionPreference = 'Stop'
$version = '4.5.2'
if ($IsWindows) {
    $asset = "Godot_v$version-stable_win64.exe.zip"
    $expected = 'b21115cf3620438e17959d13946bf9c7dceabb6ff71ed32a68996a21e08615edd2d1a3d25641bb3e260d084b31f500231aa59fcf026a5228bcc20ac4e8bade60'
    $binary = "Godot_v$version-stable_win64_console.exe"
} else {
    $asset = "Godot_v$version-stable_linux.x86_64.zip"
    $expected = 'e3ce6194b6d4d2dcef5e5b5136752c084d9d8b9071c10bc2d2011b947ec7439a257c8d23c541cb30699f16d9edea6dff36e6e84d2bec14a8fe9b4e9aacbe5a8d'
    $binary = "Godot_v$version-stable_linux.x86_64"
}
$directory = Join-Path $env:RUNNER_TEMP 'godot'
New-Item -ItemType Directory -Force $directory | Out-Null
$archive = Join-Path $env:RUNNER_TEMP $asset
Invoke-WebRequest "https://github.com/godotengine/godot-builds/releases/download/$version-stable/$asset" -OutFile $archive
$actual = (Get-FileHash $archive -Algorithm SHA512).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "Godot checksum mismatch: expected $expected, got $actual" }
Expand-Archive -Path $archive -DestinationPath $directory -Force
$godot = Join-Path $directory $binary
if (!$IsWindows) { & chmod +x $godot; if ($LASTEXITCODE) { throw 'chmod failed' } }
"GODOT_BIN=$godot" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
& $godot --version
if ($LASTEXITCODE) { throw 'Godot version probe failed' }
