# Your first app, step by step

A quick walkthrough for hiding supported VPN signals from one banking app.
The full reference is [Set up hiding](configure-hiding.md).

1. If the bank scans for installed VPN apps, first open
   [VPN app hiding](vpnhide://hidden-apps), add your VPN client to the hidden
   list and save. Do this before editing the bank's roles.
2. Open the **Hiding** tab and find your bank using search.
3. Enable **Java** and **Native**, plus **Apps** if you configured the hidden list.
4. Tap **Save** before leaving this screen. Opening another editor with unsaved
   changes can require discarding your role edits.
5. Force-stop and reopen the bank so it checks again.

Roles go on the **bank**, not on the VPN client. If it still refuses, use
[App still asks to disable VPN](app-still-requires-vpn-off.md). The server may
reject a VPN exit IP, but rejection alone does not establish the cause. Choose
whether direct access is appropriate for your needs before changing routing:
[Direct access or through the tunnel](split-tunneling.md).
