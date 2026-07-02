#!/system/bin/sh
# Called by Magisk/KSU when the module is uninstalled. Removes all our
# iptables rules so no dangling REJECT entries survive. Loops -D to
# handle duplicate OUTPUT jumps (if any snuck in).

CHAIN4="vpnhide_out"
CHAIN6="vpnhide_out6"

while iptables  -D OUTPUT -j "$CHAIN4" >/dev/null 2>&1; do :; done
iptables  -F "$CHAIN4" >/dev/null 2>&1
iptables  -X "$CHAIN4" >/dev/null 2>&1

while ip6tables -D OUTPUT -j "$CHAIN6" >/dev/null 2>&1; do :; done
ip6tables -F "$CHAIN6" >/dev/null 2>&1
ip6tables -X "$CHAIN6" >/dev/null 2>&1

PERSIST_DIR="/data/adb/vpnhide_ports"
rm -f "$PERSIST_DIR/observers.txt" "$PERSIST_DIR/load_status" "$PERSIST_DIR/load_log"
rmdir "$PERSIST_DIR" 2>/dev/null || true

log -t vpnhide_ports "uninstalled, iptables chains removed"
