$ErrorActionPreference = 'Stop'

Write-Host '=== 构建 arm64-v8a + armeabi-v7a ===' -ForegroundColor Cyan

$release = $false
foreach ($a in $args) {
    if ($a -eq '--release') {
        $release = $true
    }
}

cargo ndk -t armeabi-v7a -t arm64-v8a -o ./jniLibs build $(if ($release) { '--release' })

if (-not $?) {
    exit 1
}

Write-Host "`n=== 输出文件 ===" -ForegroundColor Green
Write-Host "arm64-v8a: jniLibs/arm64-v8a/libglsl_tools.so" -ForegroundColor Yellow
Write-Host "armeabi-v7a: jniLibs/armeabi-v7a/libglsl_tools.so" -ForegroundColor Yellow