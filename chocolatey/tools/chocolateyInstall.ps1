$ErrorActionPreference = 'Stop'

$packageName = 'caboose'
$version = $env:chocolateyPackageVersion
$url = "https://downloads.trycaboose.dev/v${version}/caboose-x86_64-pc-windows-msvc.zip"

$installDir = "$(Get-ToolsLocation)\$packageName"

$packageArgs = @{
    packageName    = $packageName
    unzipLocation  = $installDir
    url64bit       = $url
    checksum64     = '__CHECKSUM__'
    checksumType64 = 'sha256'
}

Install-ChocolateyZipPackage @packageArgs
Install-BinFile -Name 'caboose' -Path "$installDir\caboose.exe"
