# Work profiles, clones and second space

**Install VPN Hide in your main profile only.** From there it already covers
everything — you don't need a copy in a work profile, Second Space, or a cloned
space.

## Why one copy is enough

The Java layer hooks `system_server`, which runs in the **main profile only**.
The picker in your main profile already lists apps from every profile on the
device and applies hiding to all of them. A second copy in a work profile or
Second Space can't hook anything extra — and worse, extra copies race each
other's **Save** against the shared config, so they can undo one another.

If the app detects itself installed in more than one profile, the Dashboard
warns you and points to the redundant copies. Uninstall VPN Hide from the work
profile / Second Space / other secondary profiles and keep only the main-profile
one.

## Hiding the VPN from an app inside a work profile or clone

You still control that from the main-profile picker — cloned and work-profile
apps show up there (labeled as a **Work profile** or **Cloned app** entry) and
can be given roles like any other app. There's nothing to install on the
secondary side.
