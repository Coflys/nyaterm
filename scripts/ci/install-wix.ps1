# Install the release's pinned WiX CLI in an isolated directory, never globally.
$ErrorActionPreference = 'Stop'
$version = '6.0.2'
$toolPath = Join-Path $env:RUNNER_TEMP "nyaterm-wix-$version"

dotnet tool install wix --version $version --tool-path $toolPath
if ($LASTEXITCODE -ne 0) { throw 'WiX installation failed' }

$wix = Join-Path $toolPath 'wix.exe'
$installedVersion = & $wix --version
if ($LASTEXITCODE -ne 0 -or ($installedVersion.Trim() -split '\+')[0] -ne $version) {
    throw "Expected WiX $version, found $installedVersion"
}
$toolPath | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
