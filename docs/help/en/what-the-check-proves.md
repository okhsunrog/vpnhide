# What diagnostics checks

Core owns background diagnostic runs and retains their evidence. Opening the app,
Dashboard, Diagnostics or an export reads that evidence; it does not start a suite.
Use **Retry** to request a new run explicitly.

Diagnostics uses its own bounded test fixtures and a dedicated Android worker.
Your VPN can be off, and VPN Hide can be excluded from your VPN client's tunnel.
Neither condition blocks the Dashboard or diagnostics.

## What evidence proves

A vector is proven only with a visible RAW test signal, the protected observation,
and controls showing that ordinary network objects were preserved. An empty
response alone is not proof. A leak remains a leak when another check fails.
Required vectors come from the supported contract and enabled features, rather
than just the hooks that happened to install.

## Scope and limits

Evidence describes the recorded worker, policy and loaded generations. A fresh
worker does not prove that an already-running game has consumed new hooks or
configuration. After Save, restart the target app and check its actual behavior.
Network transitions and callback registrations need their own controls.

This beta still has unverified vectors and device acceptance gates. Missing proof
is shown explicitly; installation or successful config delivery is not a measured
protection result. See [Result meanings](check-result-meanings.md).

A third-party app can also refuse because of server-side IP checks, cached state
or a different detection vector. Include reproduction steps and the app version
in a [debug report](collect-report.md).
