# Rebuilds data\ar.fst and data\en.fst from the FrequencyWords lists (needs cargo + internet).
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$tmp = Join-Path $env:TEMP "arf-dicts"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$base = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018"
Invoke-WebRequest "$base/ar/ar_50k.txt" -OutFile "$tmp\ar_50k.txt"
Invoke-WebRequest "$base/en/en_50k.txt" -OutFile "$tmp\en_50k.txt"
Set-Location $root
cargo run --release -p arf-dictc -- build ar "$tmp\ar_50k.txt" data\ar.fst --top 30000 --short-top 2500
cargo run --release -p arf-dictc -- build en "$tmp\en_50k.txt" data\en.fst --top 30000 --short-top 2500
cargo run --release -p arf-dictc -- eval "$tmp\ar_50k.txt" "$tmp\en_50k.txt" --top 30000 --short-top 2500
