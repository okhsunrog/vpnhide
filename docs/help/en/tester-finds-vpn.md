# A third-party tester still finds the VPN

Your VPN Hide checks pass, but a detector app (or a website's test) still reports
a VPN. That's not necessarily a contradiction — the two measure different things.

## Compare like with like

- VPN Hide's checks verify **on-device** hiding: what an app can read locally
  about the network. A passing run means those vectors are hidden.
- A third-party tester may also key off things the device can't hide:
  - **Your exit IP.** Databases flag known VPN/datacenter IP ranges. That's
    server-side and outside the phone — hiding on the device can't change it.
    Exclude the app from the tunnel in your VPN client instead — see
    [Direct access or through the tunnel](split-tunneling.md).
  - **A vector you didn't enable.** If the tester reads installed packages or
    localhost ports and you only turned on **Java + Native**, add **Apps** or
    **Ports**.
  - **A cached result.** Force-stop and reopen the tester so it probes again.

## If VPN Hide itself reports Leak

Then a vector really is leaking. Turn on the role or backend that covers it
(see [What each result means](check-result-meanings.md)) and re-check. If it
still leaks after Save + restart, collect a [debug report](collect-report.md).
