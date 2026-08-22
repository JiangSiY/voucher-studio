# Voucher Studio 开发环境激活脚本
# 用法：.\activate-dev.ps1 （必须在当前会话直接运行；pwsh -File 会开子进程，环境变量不会保留）
# 作用：把 cargo / cmake / opencv / llvm 加到 PATH，并设置 OpenCV 4.10 链接变量

$env:PATH = @(
    "$env:USERPROFILE\.cargo\bin"
    "C:\Users\GGboy\scoop\apps\cmake\4.4.2\cmake-4.4.2-windows-x86_64\bin"
    "C:\opencv\opencv\build\x64\vc16\bin"
    "C:\LLVM\bin"
    $env:PATH
) -join ';'

$env:OpenCV_DIR          = 'C:\opencv\opencv\build'
$env:LIBCLANG_PATH       = 'C:\LLVM\bin'
$env:OPENCV_INCLUDE_PATHS = 'C:\opencv\opencv\build\include'
$env:OPENCV_LINK_PATHS    = 'C:\opencv\opencv\build\x64\vc16\lib'
$env:OPENCV_LINK_LIBS     = 'opencv_world4100'

Write-Host '✓ Voucher Studio 开发环境已激活' -ForegroundColor Green
Write-Host "  cargo:    $(Get-Command cargo -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source)"
Write-Host "  cmake:    $(Get-Command cmake -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source)"
Write-Host "  OpenCV:   $env:OpenCV_DIR"
Write-Host "  libclang: $env:LIBCLANG_PATH"
Write-Host ''
Write-Host '现在可以运行:'
Write-Host '  cargo test -p voucher-core --lib enhance'
Write-Host '  cargo build -p voucher-core --release'
Write-Host '  npm run tauri dev'
