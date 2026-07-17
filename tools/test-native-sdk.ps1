$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# 始终从仓库根目录解析头文件和构建输出。
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $RepoRoot
try {
    & cargo build --locked -p linguamesh-ffi
    if ($LASTEXITCODE -ne 0) {
        throw "Rust native library build failed."
    }

    $DllPath = Join-Path $RepoRoot "target\debug\linguamesh_ffi.dll"
    $ImportCandidates = @(
        (Join-Path $RepoRoot "target\debug\linguamesh_ffi.dll.lib"),
        (Join-Path $RepoRoot "target\debug\linguamesh_ffi.lib")
    )
    $ImportLibrary = $ImportCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not (Test-Path $DllPath) -or $null -eq $ImportLibrary) {
        throw "Rust DLL or import library was not found."
    }

    # 仅在系统临时目录中放置可执行测试产物。
    $NativeTestDir = Join-Path ([System.IO.Path]::GetTempPath()) ("linguamesh-native-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $NativeTestDir | Out-Null
    try {
        $CExecutable = Join-Path $NativeTestDir "c_header_smoke.exe"
        $CppExecutable = Join-Path $NativeTestDir "cpp_wrapper_smoke.exe"
        & cl.exe "/nologo" "/std:c11" "/W4" "/WX" "/Icontracts\abi" `
            "tests\native\c_header_smoke.c" $ImportLibrary `
            "/Fo:$NativeTestDir\c_header_smoke.obj" "/Fe:$CExecutable"
        if ($LASTEXITCODE -ne 0) {
            throw "C ABI smoke test compilation failed."
        }
        & cl.exe "/nologo" "/std:c++20" "/EHsc" "/W4" "/WX" `
            "/Icontracts\abi" "/Ibindings\cpp\include" `
            "tests\native\cpp_wrapper_smoke.cpp" $ImportLibrary `
            "/Fo:$NativeTestDir\cpp_wrapper_smoke.obj" "/Fe:$CppExecutable"
        if ($LASTEXITCODE -ne 0) {
            throw "C++ wrapper smoke test compilation failed."
        }
        Copy-Item $DllPath (Join-Path $NativeTestDir "linguamesh_ffi.dll")
        & $CExecutable
        if ($LASTEXITCODE -ne 0) {
            throw "C ABI smoke test failed."
        }
        & $CppExecutable
        if ($LASTEXITCODE -ne 0) {
            throw "C++ wrapper smoke test failed."
        }
        Write-Output "Native SDK smoke tests passed."
    }
    finally {
        Remove-Item -Recurse -Force $NativeTestDir
    }
}
finally {
    Pop-Location
}
