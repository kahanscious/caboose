$ErrorActionPreference = 'Stop'

$packageName = 'caboose'
$installDir = "$(Get-ToolsLocation)\$packageName"

Uninstall-BinFile -Name 'caboose'

if (Test-Path $installDir) {
    Remove-Item -Path $installDir -Recurse -Force
}
