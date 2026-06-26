# ReviewGate Windows 安装脚本。
#   irm https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.ps1 | iex
# 可选环境变量：
#   REVIEWGATE_VERSION      指定版本（默认 latest）
#   REVIEWGATE_INSTALL_DIR  安装目录（默认 %LOCALAPPDATA%\ReviewGate\bin）
$ErrorActionPreference = "Stop"

$repo = "dengmengmian/ReviewGate"
$version = if ($env:REVIEWGATE_VERSION) { $env:REVIEWGATE_VERSION } else { "latest" }
$installDir = if ($env:REVIEWGATE_INSTALL_DIR) { $env:REVIEWGATE_INSTALL_DIR } else { "$env:LOCALAPPDATA\ReviewGate\bin" }

$arch = if ([Environment]::Is64BitOperatingSystem) { "x64" } else { throw "仅支持 64 位 Windows" }
$asset = "reviewgate-windows-$arch.exe"

if ($version -eq "latest") {
    $url = "https://github.com/$repo/releases/latest/download/$asset"
} else {
    $url = "https://github.com/$repo/releases/download/$version/$asset"
}

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$dest = Join-Path $installDir "reviewgate.exe"
Write-Host "下载 $url …"
Invoke-WebRequest -Uri $url -OutFile $dest

# 确保 installDir 在用户 PATH 中
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    Write-Host "已把 $installDir 加入用户 PATH（重开终端生效）"
}

Write-Host "已安装到 $dest"
& $dest --version
