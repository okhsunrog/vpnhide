//! Pure native policy. No filesystem, PackageManager, transport, loaders or root effects.
//! Core resolves an inventory; both admission and local UI preview use these rules.
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use vpnhide_protocol::hook_ids::{
    BUILTIN_HOOK_MASK, Hook, KERNEL_HOOK_MASK, KMOD_HOOK_MASK, KPM_HOOK_MASK, ZYGISK_HOOK_MASK,
};
use vpnhide_protocol::{Target, format_config};
#[cfg(test)]
mod tests;
pub const FILESYSTEM_FEATURE: &str = Hook::FilesystemIfacePaths.name();
const APP_PACKAGE: &str = "dev.okhsunrog.vpnhide";
const PER_USER_RANGE: u32 = 100_000;
fn is_app_uid(uid: u32) -> bool {
    uid % PER_USER_RANGE >= 10_000
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum NativeSelection {
    Enabled(bool),
    Hooks(Vec<String>),
    Detailed(NativeSelectionDetail),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeSelectionDetail {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub kernel: Option<Vec<String>>,
    #[serde(default)]
    pub zygisk: Option<Vec<String>>,
}

impl Default for NativeSelection {
    fn default() -> Self {
        Self::Enabled(false)
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeHookFamily {
    Kmod,
    Kpm,
    Builtin,
    Zygisk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub target_uids: usize,
    /// The minimum kernel page size is a conservative bound for kernel writes.
    pub wire_bytes: usize,
}

impl NativeHookFamily {
    pub fn limits(self) -> Limits {
        Limits {
            target_uids: vpnhide_protocol::MAX_TARGET_UIDS,
            wire_bytes: match self {
                Self::Kpm => vpnhide_protocol::KPM_ARGS_LEN - 1,
                Self::Kmod | Self::Builtin => 4096,
                Self::Zygisk => vpnhide_protocol::zygisk_activation::MAX_BYTES - 128,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub target_uids: usize,
    pub wire_bytes: usize,
    pub limits: Limits,
}

impl Usage {
    pub fn fits(self) -> bool {
        self.target_uids <= self.limits.target_uids && self.wire_bytes <= self.limits.wire_bytes
    }
}

/// Preview remains useful for an overflowing draft so removals can be permitted.
pub fn usage(
    debug: bool,
    selections: &BTreeMap<String, NativeSelection>,
    packages: &BTreeMap<String, Vec<u32>>,
    family: NativeHookFamily,
    filesystem_enabled: bool,
) -> Usage {
    let targets: Vec<_> = native_target_set(selections, packages, family, filesystem_enabled)
        .into_iter()
        .map(|(uid, (hooks, _))| Target {
            uid,
            hookmask: hooks.bits(),
        })
        .collect();
    Usage {
        target_uids: targets.len(),
        wire_bytes: format_config(debug, 0, &targets).len(),
        limits: family.limits(),
    }
}

impl NativeHookFamily {
    fn full_set(self) -> HookSet {
        match self {
            NativeHookFamily::Kmod => HookSet::from_bits(KERNEL_HOOK_MASK | KMOD_HOOK_MASK),
            NativeHookFamily::Kpm => HookSet::from_bits(KERNEL_HOOK_MASK | KPM_HOOK_MASK),
            NativeHookFamily::Builtin => HookSet::from_bits(KERNEL_HOOK_MASK | BUILTIN_HOOK_MASK),
            NativeHookFamily::Zygisk => HookSet::from_bits(ZYGISK_HOOK_MASK),
        }
    }

    /// True for backends with no separate loader ABI (no insmod module param,
    /// no kernel loader): the optional filesystem hook must be gated in the
    /// projected mask itself, because there is no load-time gate behind it.
    /// The .ko (module_param) and KPM load parameters gate at load, so their
    /// mask may carry the bit unconditionally; the compiled-in built-in driver
    /// and zygisk cannot, so their mask is the only gate.
    fn filesystem_gated_in_mask(self) -> bool {
        matches!(self, NativeHookFamily::Builtin | NativeHookFamily::Zygisk)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookSet(u32);

impl HookSet {
    const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    fn from_names(names: &[String]) -> Self {
        names
            .iter()
            .filter_map(|name| Hook::from_name(name))
            .fold(Self::default(), |hooks, hook| hooks.with(hook))
    }

    const fn with(self, hook: Hook) -> Self {
        Self(self.0 | hook.bit())
    }

    const fn without(self, hook: Hook) -> Self {
        Self(self.0 & !hook.bit())
    }

    const fn restricted_to(self, owner: Self) -> Self {
        Self(self.0 & owner.0)
    }

    fn merge(&mut self, other: Self) {
        self.0 |= other.0;
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl NativeSelection {
    pub fn hooks(&self, family: NativeHookFamily) -> Option<HookSet> {
        match self {
            NativeSelection::Enabled(false) => None,
            NativeSelection::Enabled(true) => Some(family.full_set()),
            NativeSelection::Hooks(names) => {
                if names.is_empty() {
                    return None;
                }
                let hooks = match family {
                    NativeHookFamily::Kmod | NativeHookFamily::Kpm | NativeHookFamily::Builtin => {
                        HookSet::from_names(names).restricted_to(family.full_set())
                    }
                    NativeHookFamily::Zygisk => family.full_set(),
                };
                (!hooks.is_empty()).then_some(hooks)
            }
            NativeSelection::Detailed(detail) => {
                if !detail.enabled {
                    return None;
                }
                let selected = match family {
                    NativeHookFamily::Kmod | NativeHookFamily::Kpm | NativeHookFamily::Builtin => {
                        &detail.kernel
                    }
                    NativeHookFamily::Zygisk => &detail.zygisk,
                };
                let Some(names) = selected else {
                    return Some(family.full_set());
                };
                let hooks = HookSet::from_names(names).restricted_to(family.full_set());
                (!hooks.is_empty()).then_some(hooks)
            }
        }
    }
}

/// One package's share of an over-capacity native target set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacityPackage {
    pub package: String,
    /// The app uids (every profile / clone) this package adds to the target set.
    pub uids: Vec<u32>,
}

/// The projected native target set has more distinct uids than a backend can hold.
/// Core refuses the whole projection: it never truncates, so nothing is applied and
/// the previously applied config stays in effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeCapacityExceeded {
    /// Distinct target uids the config projects to.
    pub total: usize,
    /// [`vpnhide_protocol::MAX_TARGET_UIDS`], the backends' fixed `targets[]` size.
    pub capacity: usize,
    /// Every native-target package with the uids it contributes, most uids first, so
    /// the user can see what to deselect. The self package keeps one reserved uid.
    pub packages: Vec<CapacityPackage>,
}

impl NativeCapacityExceeded {
    /// How many uids must go before the set fits.
    pub fn overflow(&self) -> usize {
        self.total.saturating_sub(self.capacity)
    }
}

impl std::fmt::Display for NativeCapacityExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "native target capacity exceeded: {} uids, capacity {} (deselect at least {})",
            self.total,
            self.capacity,
            self.overflow()
        )
    }
}

impl std::error::Error for NativeCapacityExceeded {}

fn effective_hooks(
    selection: &NativeSelection,
    family: NativeHookFamily,
    filesystem_enabled: bool,
) -> Option<HookSet> {
    let mut hooks = selection.hooks(family)?;
    if family.filesystem_gated_in_mask() && !filesystem_enabled {
        hooks = hooks.without(Hook::FilesystemIfacePaths);
    }
    (!hooks.is_empty()).then_some(hooks)
}

/// Removing/narrowing protection remains possible even if grouped wire encoding
/// grows, or the inventory is incomplete. This compares policy, not UI row counts.
pub fn is_reduction(
    before: &BTreeMap<String, NativeSelection>,
    before_filesystem: bool,
    after: &BTreeMap<String, NativeSelection>,
    after_filesystem: bool,
    family: NativeHookFamily,
) -> bool {
    after.iter().all(|(package, selection)| {
        let next = effective_hooks(selection, family, after_filesystem).map_or(0, HookSet::bits);
        let previous = before
            .get(package)
            .and_then(|selection| effective_hooks(selection, family, before_filesystem))
            .map_or(0, HookSet::bits);
        next & !previous == 0
    })
}

/// The native target set: each target uid with its merged hooks and the packages that
/// put it there. This is the one place native targets are counted — the projection,
/// core's Save refusal and the app's pre-Save counter all follow these rules:
/// uids across every profile / clone, non-app uids skipped, and the self package
/// limited to its single main-profile slot.
pub fn native_target_set(
    selections: &BTreeMap<String, NativeSelection>,
    packages: &BTreeMap<String, Vec<u32>>,
    family: NativeHookFamily,
    filesystem_enabled: bool,
) -> BTreeMap<u32, (HookSet, BTreeSet<String>)> {
    let mut by_uid = BTreeMap::<u32, (HookSet, BTreeSet<String>)>::new();
    let mut add = |uid: u32, hooks: HookSet, pkg: &str| {
        let entry = by_uid
            .entry(uid)
            .or_insert_with(|| (hooks, BTreeSet::new()));
        entry.0.merge(hooks);
        entry.1.insert(pkg.to_owned());
    };
    for (pkg, selection) in selections {
        let Some(hooks) = effective_hooks(selection, family, filesystem_enabled) else {
            continue;
        };
        // The APK is intentionally single-owner: only its main-profile copy
        // may manage the shared config. Keep its mandatory self target just as
        // singular here. An accidentally installed work/clone-profile copy is
        // blocked at startup and must not consume another backend slot.
        if pkg == APP_PACKAGE {
            if let Some(uid) = packages
                .get(pkg)
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .copied()
                .find(|uid| *uid < PER_USER_RANGE && is_app_uid(*uid))
            {
                add(uid, hooks, pkg);
            }
            continue;
        }
        for uid in packages.get(pkg).map(Vec::as_slice).unwrap_or(&[]) {
            // Below the app range a uid is not an app but a platform identity
            // shared by many components — a package declaring sharedUserId
            // "android.uid.system" resolves to 1000, the same uid as
            // system_server. Since UID is the targeting key, listing one would
            // mean "hide from everything running as 1000", which is how a
            // device ends up believing it has no route. This is NOT the same
            // set as FLAG_SYSTEM: vendor-preinstalled apps keep ordinary 10xxx
            // uids and stay targetable. `project_ports_with_resolver` has
            // filtered the same way from the start; the native path had not.
            // Both kernel backends enforce it too, so this is the polite half.
            if !is_app_uid(*uid) {
                continue;
            }
            add(*uid, hooks, pkg);
        }
    }
    by_uid
}

/// Project the native control-v2 wire, or refuse it whole when the target set does not
/// fit a backend. Never truncates: dropping the highest uids would silently leave the
/// most recently installed apps unprotected.
pub fn project_native(
    debug: bool,
    selections: &BTreeMap<String, NativeSelection>,
    packages: &BTreeMap<String, Vec<u32>>,
    family: NativeHookFamily,
    filesystem_enabled: bool,
) -> std::result::Result<String, NativeCapacityExceeded> {
    let by_uid = native_target_set(selections, packages, family, filesystem_enabled);
    if by_uid.len() > vpnhide_protocol::MAX_TARGET_UIDS {
        let mut per_package = BTreeMap::<String, Vec<u32>>::new();
        for (uid, (_, packages)) in &by_uid {
            for pkg in packages {
                per_package.entry(pkg.clone()).or_default().push(*uid);
            }
        }
        let mut packages = per_package
            .into_iter()
            .map(|(package, uids)| CapacityPackage { package, uids })
            .collect::<Vec<_>>();
        packages.sort_by(|a, b| {
            b.uids
                .len()
                .cmp(&a.uids.len())
                .then_with(|| a.package.cmp(&b.package))
        });
        return Err(NativeCapacityExceeded {
            total: by_uid.len(),
            capacity: vpnhide_protocol::MAX_TARGET_UIDS,
            packages,
        });
    }
    let targets = by_uid
        .into_iter()
        .map(|(uid, (hooks, _))| Target {
            uid,
            hookmask: hooks.bits(),
        })
        .collect::<Vec<_>>();
    Ok(format_config(debug, 0, &targets))
}
