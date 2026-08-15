@echo off
chcp 65001 >nul
title AI作战室 - 构建工具

echo ========================================
echo    AI作战室 - 一键构建脚本
echo ========================================
echo.

:: 检查Node.js
where node >nul 2>nul
if %errorlevel% neq 0 (
    echo [错误] 未检测到Node.js，请先安装: https://nodejs.org/
    pause
    exit /b 1
)
echo [OK] Node.js: 
node --version

:: 检查Rust
where cargo >nul 2>nul
if %errorlevel% neq 0 (
    echo [错误] 未检测到Rust，请先安装: https://rustup.rs/
    pause
    exit /b 1
)
echo [OK] Rust:
cargo --version

:: 检查Git
where git >nul 2>nul
if %errorlevel% neq 0 (
    echo [警告] 未检测到Git，建议安装
) else (
    echo [OK] Git:
    git --version
)

echo.
echo [1/4] 安装前端依赖...
call npm install
if %errorlevel% neq 0 (
    echo [错误] npm install 失败
    pause
    exit /b 1
)

echo.
echo [2/4] 下载One API二进制...
if not exist "src-tauri\binaries" mkdir "src-tauri\binaries"
if not exist "src-tauri\binaries\one-api-x86_64-pc-windows-msvc.exe" (
    echo 正在下载One API v0.6.10...
    powershell -Command "Invoke-WebRequest -Uri 'https://github.com/songquanpeng/one-api/releases/download/v0.6.10/one-api.exe' -OutFile 'src-tauri\binaries\one-api-x86_64-pc-windows-msvc.exe'"
    if %errorlevel% neq 0 (
        echo [错误] One API下载失败，请检查网络或手动下载
        pause
        exit /b 1
    )
    echo [OK] One API下载完成
) else (
    echo [OK] One API已存在，跳过下载
)

echo.
echo [3/4] 生成应用图标...
if not exist "src-tauri\icons" (
    echo [提示] 图标目录不存在，请先运行 npx tauri icon icon.png
    echo 使用默认图标继续...
)

echo.
echo [4/4] 开始Tauri构建（首次会比较慢，请耐心等待）...
echo.
call npm run tauri build
if %errorlevel% neq 0 (
    echo [错误] 构建失败
    pause
    exit /b 1
)

echo.
echo ========================================
echo    构建完成！
echo ========================================
echo.
echo 安装包位置:
echo   MSI:  src-tauri\target\release\bundle\msi\
echo   NSIS: src-tauri\target\release\bundle\nsis\
echo.
pause
