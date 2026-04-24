#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "usage: run_radarr_parser.sh <input.jsonl> <output.jsonl> [radarr-source-dir]" >&2
  exit 2
fi

input_path="$1"
output_path="$2"
radarr_source_dir="${3:-${RADARR_SOURCE_DIR:-/Users/jeremy/dev/supporting-codebases/Radarr}}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
runtime="${CONTAINER_RUNTIME:-}"
image="${RADARR_DOTNET_IMAGE:-mcr.microsoft.com/dotnet/sdk:8.0}"
nuget_cache="${RADARR_NUGET_CACHE:-${HOME}/.cache/scryer-radarr-nuget}"

if [[ -z "$runtime" ]]; then
  if command -v docker >/dev/null 2>&1; then
    runtime="docker"
  elif command -v podman >/dev/null 2>&1; then
    runtime="podman"
  else
    echo "docker or podman is required to run the Radarr parser fixture" >&2
    exit 127
  fi
fi

input_path="$(cd "$(dirname "$input_path")" && pwd)/$(basename "$input_path")"
output_dir="$(dirname "$output_path")"
mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"
output_file="$(basename "$output_path")"
radarr_source_dir="$(cd "$radarr_source_dir" && pwd)"
mkdir -p "$nuget_cache"
nuget_cache="$(cd "$nuget_cache" && pwd)"

"$runtime" run --rm \
  -e DOTNET_CLI_TELEMETRY_OPTOUT=1 \
  -e DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1 \
  -e NUGET_PACKAGES=/nuget \
  -v "$script_dir:/fixture-src:ro" \
  -v "$radarr_source_dir:/radarr-src:ro" \
  -v "$nuget_cache:/nuget" \
  -v "$input_path:/input.jsonl:ro" \
  -v "$output_dir:/out" \
  "$image" \
  bash -lc 'set -euo pipefail
    mkdir -p /tmp/fixture /tmp/radarr
    cp -a /fixture-src/. /tmp/fixture/
    cp -a /radarr-src/. /tmp/radarr/
    dotnet restore /tmp/fixture/RadarrParserRunner.csproj \
      --configfile /tmp/radarr/src/NuGet.config \
      -p:RadarrSourceRoot=/tmp/radarr/ \
      -p:RunAnalyzers=false \
      -p:EnableNETAnalyzers=false \
      -p:TreatWarningsAsErrors=false \
      -p:WarningsAsErrors=
    dotnet run --no-restore --project /tmp/fixture/RadarrParserRunner.csproj \
      -p:RadarrSourceRoot=/tmp/radarr/ \
      -p:RunAnalyzers=false \
      -p:EnableNETAnalyzers=false \
      -p:TreatWarningsAsErrors=false \
      -p:WarningsAsErrors= \
      -- /input.jsonl "/out/$0"
  ' "$output_file"
