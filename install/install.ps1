#Requires -Version 5.1
$ErrorActionPreference = 'Stop'

$DownloadsBaseUrl = "https://downloads.trycaboose.dev"
$BinaryName = "caboose"
$InstallDir = "$env:USERPROFILE\.caboose\bin"

function Main {
    param([string]$Version)

    # Detect architecture
    $arch = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq [System.Runtime.InteropServices.Architecture]::X64) {
        "x86_64"
    } elseif ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq [System.Runtime.InteropServices.Architecture]::Arm64) {
        "aarch64"
    } else {
        Write-Error "Unsupported architecture"
        return
    }

    $target = "${arch}-pc-windows-msvc"
    $artifact = "${BinaryName}-${target}.zip"

    # Resolve version
    if (-not $Version) {
        Write-Host "Fetching latest version..."
        $Version = (Invoke-RestMethod -Uri "$DownloadsBaseUrl/latest.txt").Trim()
    }

    Write-Host "Installing $BinaryName $Version ($target)..."

    # Create temp directory
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    try {
        # Download artifact and checksums
        $artifactPath = Join-Path $tmpDir $artifact
        $checksumsPath = Join-Path $tmpDir "checksums.txt"

        Invoke-WebRequest -Uri "$DownloadsBaseUrl/$Version/$artifact" -OutFile $artifactPath
        Invoke-WebRequest -Uri "$DownloadsBaseUrl/$Version/checksums.txt" -OutFile $checksumsPath

        # Verify checksum
        $checksums = Get-Content $checksumsPath
        $expectedLine = $checksums | Where-Object { $_ -match $artifact }
        if (-not $expectedLine) {
            Write-Error "Checksum not found for $artifact"
            return
        }
        $expectedChecksum = ($expectedLine -split '\s+')[0]

        $actualChecksum = (Get-FileHash -Path $artifactPath -Algorithm SHA256).Hash.ToLower()
        if ($actualChecksum -ne $expectedChecksum) {
            Write-Error "Checksum verification failed`n  Expected: $expectedChecksum`n  Got:      $actualChecksum"
            return
        }

        # Extract
        $extractDir = Join-Path $tmpDir "extracted"
        Expand-Archive -Path $artifactPath -DestinationPath $extractDir

        # Install
        if (-not (Test-Path $InstallDir)) {
            New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        }

        $exePath = Get-ChildItem -Path $extractDir -Recurse -Filter "${BinaryName}.exe" | Select-Object -First 1
        if (-not $exePath) {
            Write-Error "Could not find ${BinaryName}.exe in archive"
            return
        }
        Copy-Item -Path $exePath.FullName -Destination (Join-Path $InstallDir "${BinaryName}.exe") -Force

        # Add to PATH if not already there
        $userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
        if ($userPath -notlike "*$InstallDir*") {
            [Environment]::SetEnvironmentVariable("PATH", "$InstallDir;$userPath", "User")
            $env:PATH = "$InstallDir;$env:PATH"
            Write-Host "Added $InstallDir to your PATH."
        }

        Write-Host ""
        Write-Host "$BinaryName $Version installed to $InstallDir\${BinaryName}.exe"
        Write-Host ""
        Write-Host "Restart your terminal, then run 'caboose' to get started."
        Write-Host "Run 'caboose update' to update in the future."
    }
    finally {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Main @args
