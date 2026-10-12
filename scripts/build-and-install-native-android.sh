#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
install=false
serial="${ANDROID_SERIAL:-}"
for argument in "$@"; do
  case "$argument" in
    --install) install=true ;;
    --serial=*) serial="${argument#--serial=}" ;;
    --help) echo 'Usage: scripts/build-and-install-native-android.sh [--install] [--serial=SERIAL]'; exit 0 ;;
    *) echo "Unknown argument: $argument" >&2; exit 2 ;;
  esac
done
export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
export PRESTO_BUILD_TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
(cd "$ROOT/native-android" && ./gradlew :core:test :app:assembleDebug :app:lintDebug --console=plain)
apk="$ROOT/native-android/app/build/outputs/apk/debug/app-debug.apk"
cp "$apk" "$ROOT/PrestoNative.apk"
printf 'APK: %s\n' "$ROOT/PrestoNative.apk"
if $install; then
  adb_bin="$ANDROID_HOME/platform-tools/adb"
  selection=()
  if [[ -n "$serial" ]]; then selection=(-s "$serial"); fi
  "$adb_bin" "${selection[@]}" install --no-streaming -r "$apk"
  "$adb_bin" "${selection[@]}" shell am start -n com.prestosupuesto.nativeapp/com.presto.nativeapp.MainActivity
fi
