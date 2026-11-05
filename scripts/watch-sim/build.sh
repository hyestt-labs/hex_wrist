#!/bin/sh
# Builds watch-sim from the shipping watch app's Swift sources, read in
# place from a Wristper checkout (WRISTPER_SRC), so its bytes on the wire
# are the watch's. Only WatchMacTargetStore (Keychain) is replaced by a shim;
# MacRelay, PairedMac, and RelayFailure are cut verbatim out of their files.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
src=${WRISTPER_SRC:-/Users/glenhsu/orca/workspaces/wristper/coding}
out=${1:-$here/../../target/watch-sim/watch-sim}
gen=$(dirname "$out")/gen
mkdir -p "$gen"
echo "watch sources: $src" >&2
[ -f "$src/WatchProbe/WatchMacPairing.swift" ] || { echo "WRISTPER_SRC=$src has no WatchProbe/" >&2; exit 1; }

extract() { # file decl-header
  awk -v head="$2" 'index($0, head) == 1 {p=1} p {print} p && /^}/ {exit}' "$1"
}
{
  echo "import Foundation"
  extract "$src/WatchProbe/WatchMacTargetStore.swift" "enum MacRelay {"
  extract "$src/WatchProbe/WatchDelivery.swift" "struct PairedMac:"
  extract "$src/WatchProbe/WatchDelivery.swift" "enum RelayFailure:"
} > "$gen/Extracted.swift"
for decl in "enum MacRelay" "struct PairedMac" "enum RelayFailure"; do
  grep -q "^$decl" "$gen/Extracted.swift" || { echo "could not extract $decl from $src" >&2; exit 1; }
done

xcrun swiftc -O -parse-as-library -swift-version 5 -target arm64-apple-macos14 \
  -o "$out" \
  "$here/main.swift" "$here/WatchStoreShim.swift" "$gen/Extracted.swift" \
  "$src/WatchShared/WristperProtocol.swift" \
  "$src/WatchShared/RecordingLaunch.swift" \
  "$src/WatchProbe/PairingKey.swift" \
  "$src/WatchProbe/PairedMacEncryption.swift" \
  "$src/WatchProbe/EncryptedResponse.swift" \
  "$src/WatchProbe/EncryptedWatchMacBridge.swift" \
  "$src/WatchProbe/WatchMacBridge.swift" \
  "$src/WatchProbe/WatchMacPairing.swift" \
  "$src/WatchProbe/WatchMacStatus.swift"
echo "$out"
