# Update feeds

Only the unified Core has a current release update channel. After a release is
published, `scripts/update-json.sh` generates `update-core.json` for the public
`vpnhide-core.zip` asset. It must not advertise draft/unpublished assets.

The existing kmod/KPM/Zygisk JSON files are frozen legacy compatibility data.
They are not the 2.0 distribution and the generator no longer updates them.
Do not redirect an old module feed to Core: its module identity and migration
requirements differ. Internal backend build inputs carry no updateJson field.
