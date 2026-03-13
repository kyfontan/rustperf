@echo off
setlocal EnableExtensions EnableDelayedExpansion

set "RUSTPERF_NORM_ROOT=__RUST_PERF_NORM_ROOT__"
set "LINT_LIBRARY_PATH=%RUSTPERF_NORM_ROOT:\=/%/crates/machine-oriented-lints"

if "%~1"=="init" (
  shift
  if not "%~1"=="" (
    echo Error: rustperf init does not accept extra arguments 1>&2
    exit /b 1
  )

  if not exist "Cargo.toml" (
    echo Error: no Cargo.toml found in %CD% 1>&2
    exit /b 1
  )

  findstr /C:"[workspace.metadata.dylint]" "Cargo.toml" >nul 2>nul
  if errorlevel 1 (
    powershell -NoProfile -ExecutionPolicy Bypass -Command ^
      "$path = 'Cargo.toml';" ^
      "$block = @'" ^
[workspace.metadata.dylint]
libraries = [
  { path = "%LINT_LIBRARY_PATH%" },
]
'@;" ^
      "$content = if (Test-Path $path) { Get-Content $path -Raw } else { '' };" ^
      "if ($content.Length -gt 0) {" ^
      "  if ($content.EndsWith(\"`n\")) { $content += \"`n\" + $block } else { $content += \"`r`n`r`n\" + $block }" ^
      "} else {" ^
      "  $content = $block" ^
      "}" ^
      "[System.IO.File]::WriteAllText($path, $content, [System.Text.UTF8Encoding]::new($false))"
  )

  if not exist "dylint.toml" (
    > "dylint.toml" (
      echo [machine_oriented_lints]
      echo # Warn when Vec::with_capacity uses a tiny compile-time constant.
      echo small_vec_capacity_threshold = 64
      echo.
      echo # Warn when Vec::new() is followed by N or more consecutive push calls.
      echo vec_new_then_push_min_pushes = 2
    )
  ) else (
    findstr /C:"[machine_oriented_lints]" "dylint.toml" >nul 2>nul
    if errorlevel 1 (
      powershell -NoProfile -ExecutionPolicy Bypass -Command ^
        "$path = 'dylint.toml';" ^
        "$block = @'" ^
[machine_oriented_lints]
# Warn when Vec::with_capacity uses a tiny compile-time constant.
small_vec_capacity_threshold = 64

# Warn when Vec::new() is followed by N or more consecutive push calls.
vec_new_then_push_min_pushes = 2
'@;" ^
        "$content = Get-Content $path -Raw;" ^
        "if ($content.EndsWith(\"`n\")) { $content += \"`n\" + $block } else { $content += \"`r`n`r`n\" + $block }" ^
        "[System.IO.File]::WriteAllText($path, $content, [System.Text.UTF8Encoding]::new($false))"
    )
  )

  echo Updated %CD%\Cargo.toml
  echo Updated %CD%\dylint.toml
  exit /b 0
)

cargo dylint --all %*
