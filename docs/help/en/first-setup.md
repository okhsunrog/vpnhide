# Your first app, step by step

A quick walkthrough for hiding the VPN from one banking app. The full reference
is [Set up hiding](configure-hiding.md).

1. Open the **Hiding** tab.
2. Find the app you want to hide the VPN from — your bank. Use search if the list
   is long.
3. Turn on **Java** and **Native** for it. Together they cover the VPN signals
   apps read through Android's Java APIs and through the kernel or libc.
4. If the bank also scans for installed VPN apps, turn on **Apps** for it, and
   make sure your VPN client (WireGuard, and so on) is in the hidden list —
   [VPN app hiding](vpnhide://hidden-apps).
5. Tap **Save**.
6. Force-stop and reopen the bank so it checks again.

Remember: the roles go on the **bank**, not on the VPN client. The VPN client is
the thing being hidden. If the bank still refuses, work through
[Set up hiding](configure-hiding.md). Often the remaining cause is your VPN
server's exit IP, which no hiding on the phone can change: exclude the bank from
the tunnel in your VPN client's split-tunnel settings — see
[Direct access or through the tunnel](split-tunneling.md).
