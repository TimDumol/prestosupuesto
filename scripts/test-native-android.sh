#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
device=false
serial="${ANDROID_SERIAL:-}"
for argument in "$@"; do
  case "$argument" in
    --device) device=true ;;
    --serial=*) serial="${argument#--serial=}" ;;
    --help) echo 'Usage: scripts/test-native-android.sh [--device] [--serial=SERIAL]'; exit 0 ;;
    *) echo "Unknown argument: $argument" >&2; exit 2 ;;
  esac
done
if [[ -n "$serial" ]]; then export ANDROID_SERIAL="$serial"; fi
export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
(cd "$ROOT/native-android" && ./gradlew :core:test :app:lintDebug :app:assembleDebug :app:assembleDeviceTestAndroidTest --console=plain)
if $device; then
  (cd "$ROOT/native-android" && ./gradlew :app:connectedDeviceTestAndroidTest --console=plain)
fi
