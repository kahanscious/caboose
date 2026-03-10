$ErrorActionPreference = 'Stop'

$packageName = 'caboose'
$version = $env:chocolateyPackageVersion
$url = "https://downloads.trycaboose.dev/v${version}/caboose-x86_64-pc-windows-msvc.zip"
$checksumUrl = "https://downloads.trycaboose.dev/v${version}/checksums.txt"

# Fetch expected checksum
$checksums = (Invoke-WebRequest -Uri $checksumUrl -UseBasicParsing).Content
$expectedHash = ($checksums -split "`n" | Where-Object { $_ -match 'caboose-x86_64-pc-windows-msvc\.zip' }) -replace '\s+.*$', ''

$installDir = "$(Get-ToolsLocation)\$packageName"

$packageArgs = @{
    packageName    = $packageName
    unzipLocation  = $installDir
    url64bit       = $url
    checksum64     = $expectedHash
    checksumType64 = 'sha256'
}

Install-ChocolateyZipPackage @packageArgs
Install-BinFile -Name 'caboose' -Path "$installDir\caboose.exe"
