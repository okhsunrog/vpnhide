# The LSPosed / Vector framework lines of this boot's log (bounded).
logcat -d -b all -v threadtime 2>/dev/null |
  grep -Ei 'VpnHide-LSPosed|LSPosed-Bridge|VectorNative|VectorBridge|LSPosedService|LSPlt|LSPHooker|LSPosedBridge|Xposed|org[.]lsposed|lspd|modules_config|dev[.]okhsunrog[.]vpnhide' |
  tail -2000 || true
