# Build the dump tool using MSVC
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$SRCDIR = Join-Path $ScriptDir "Open_AlteryxYXDB"
$OUTDIR = $ScriptDir

# Find Visual Studio Build Tools
$VCVarsCandidates = @(
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
)
$VCVARS = $VCVarsCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $VCVARS) {
    Write-Error "Visual Studio 2022 Build Tools not found"
    exit 1
}

# Setup MSVC environment
Write-Host "Setting up MSVC environment..."
cmd /c "`"$VCVARS`" >nul 2>&1 && set" | ForEach-Object {
    if ($_ -match "^([^=]+)=(.*)$") {
        [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
    }
}

Write-Host "Building dump tool..."

& cl.exe /nologo /EHsc /O2 /DUNICODE /D_CRT_SECURE_NO_WARNINGS /D_SCL_SECURE_NO_DEPRECATE `
    "/I$SRCDIR" "/I$SRCDIR\RecordLib" "/I$SRCDIR\liblzf-3.6" `
    "/Fe$OUTDIR\open_yxdb_dump.exe" `
    "$OUTDIR\dump.cpp" `
    "$SRCDIR\Open_AlteryxYXDB.cpp" `
    "$SRCDIR\RecordLib\Record.cpp" `
    "$SRCDIR\RecordLib\FieldBase.cpp" `
    "$SRCDIR\liblzf-3.6\lzf_c.c" `
    "$SRCDIR\liblzf-3.6\lzf_d.c" `
    /link /MACHINE:X64

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed"
    exit 1
}

Write-Host ""
Write-Host "Build successful: $OUTDIR\open_yxdb_dump.exe"

# Clean up obj files
Remove-Item -Path "$OUTDIR\*.obj" -Force -ErrorAction SilentlyContinue
