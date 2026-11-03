//! PackageManager text observations shared by Core and local APK adapters.
//! No commands, Android APIs, readiness decision or authorization live here.
use std::collections::{BTreeMap, BTreeSet};

pub const PM_USERS_STATUS: &str = "__VPNHIDE_PM_USERS_STATUS__:";
pub const PM_USER_BEGIN: &str = "__VPNHIDE_PM_USER_BEGIN__:";
pub const PM_USER_END: &str = "__VPNHIDE_PM_USER_END__:";
pub const MAX_OBSERVATION_BYTES: usize = 32 * 1024 * 1024;
pub const UIDS_PER_USER: u32 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageRow {
    pub package_name: String,
    pub apk_path: Option<String>,
    pub uids: Vec<u32>,
}

/// Both `-U` and `-U -f` rows; UID lists are normalized once. Android UIDs are
/// nonnegative signed 32-bit integers, including platform AIDs. Do not wrap an
/// oversized observation through Kotlin Int or admit negative UIDs as targets.
pub fn parse_package_row(line: &str) -> Option<PackageRow> {
    let body = line.trim().strip_prefix("package:")?;
    let (package, uids) = body.rsplit_once(" uid:")?;
    let package = package.trim();
    let (path, name) = package.rsplit_once('=').map_or((None, package), |(p, n)| {
        (Some(p.trim()).filter(|p| !p.is_empty()), n.trim())
    });
    if name.is_empty() {
        return None;
    }
    let uids: BTreeSet<_> = uids
        .split(',')
        .filter_map(|v| v.trim().parse::<u32>().ok())
        .filter(|v| *v <= i32::MAX as u32)
        .collect();
    if uids.is_empty() {
        return None;
    }
    Some(PackageRow {
        package_name: name.into(),
        apk_path: path.map(str::to_owned),
        uids: uids.into_iter().collect(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryError {
    InputTooLarge,
}
fn bounded(raw: &str) -> Result<(), InventoryError> {
    if raw.len() > MAX_OBSERVATION_BYTES {
        Err(InventoryError::InputTooLarge)
    } else {
        Ok(())
    }
}

pub fn parse_uid_map(raw: &str) -> Result<BTreeMap<String, Vec<u32>>, InventoryError> {
    bounded(raw)?;
    let mut packages = BTreeMap::<String, BTreeSet<u32>>::new();
    for row in raw.split(['\n', '\r']).filter_map(parse_package_row) {
        packages
            .entry(row.package_name)
            .or_default()
            .extend(row.uids);
    }
    Ok(packages
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageEntry {
    pub apk_path: Option<String>,
    pub uids_by_user: BTreeMap<u32, Vec<u32>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageInventory {
    pub packages: BTreeMap<String, PackageEntry>,
    pub failed_user_ids: Vec<u32>,
    pub user_list_complete: bool,
}

/// Display inventory may retain readable rows from a failed or truncated scan.
/// Only an explicit successful END establishes that profile's completeness;
/// exit-0 with no packages is legitimate. Core admission obtains its own fresh
/// complete inventory; neither these markers nor APK backstop rows grant it.
pub fn parse_inventory(
    packages_raw: &str,
    users_raw: &str,
    expected_users: &[u32],
) -> Result<PackageInventory, InventoryError> {
    bounded(packages_raw)?;
    bounded(users_raw)?;
    let mut packages = BTreeMap::<String, PackageEntry>::new();
    let mut statuses = BTreeMap::<u32, i32>::new();
    let mut current_user = None;
    for line in packages_raw.split(['\n', '\r']) {
        if let Some(raw) = line.strip_prefix(PM_USER_BEGIN) {
            current_user = raw
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|id| *id <= i32::MAX as u32);
            if let Some(id) = current_user {
                // A later truncated retry must not inherit an earlier success.
                statuses.remove(&id);
            }
        } else if let Some(raw) = line.strip_prefix(PM_USER_END) {
            if let Some((id, status)) = raw.split_once(':')
                && let (Ok(id), Ok(status)) = (id.parse::<u32>(), status.parse::<i32>())
                && current_user == Some(id)
            {
                statuses.insert(id, status);
            }
            current_user = None;
        } else if let Some(row) = parse_package_row(line) {
            let entry = packages.entry(row.package_name).or_default();
            if entry.apk_path.is_none() {
                entry.apk_path = row.apk_path;
            }
            for uid in row.uids {
                let user = current_user.unwrap_or(uid / UIDS_PER_USER);
                entry.uids_by_user.entry(user).or_default().push(uid);
            }
        }
    }
    for entry in packages.values_mut() {
        for uids in entry.uids_by_user.values_mut() {
            uids.sort_unstable();
            uids.dedup();
        }
    }
    let user_list_complete = !expected_users.is_empty()
        && users_raw.split(['\n', '\r']).any(|line| {
            line.strip_prefix(PM_USERS_STATUS)
                .and_then(|v| v.rsplit_once(':'))
                .and_then(|(_, status)| status.trim().parse::<i32>().ok())
                == Some(0)
        });
    Ok(PackageInventory {
        packages,
        failed_user_ids: expected_users
            .iter()
            .copied()
            .filter(|id| statuses.get(id) != Some(&0))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        user_list_complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_keep_paths_with_spaces_union_profiles_and_reject_uid_overflow() {
        let map = parse_uid_map("package:/data/app/a b/base.apk=com.example uid:10123, 1010123,10123\r\npackage:com.example uid:1000,bad,-1,2147483648,4294967295\npackage: uid:1\nnoise uid:2").unwrap();
        assert_eq!(
            map,
            BTreeMap::from([("com.example".into(), vec![1000, 10123, 1010123])])
        );
        let row =
            parse_package_row(" package:/data/app/a=b/base.apk=com.example uid:10123 ").unwrap();
        assert_eq!(row.apk_path.as_deref(), Some("/data/app/a=b/base.apk"));
        assert_eq!(row.package_name, "com.example");
        assert!(parse_package_row("package:com.bad uid:-1").is_none());
        assert_eq!(
            parse_package_row("package:com.max uid:2147483647")
                .unwrap()
                .uids,
            [2147483647]
        );
    }
    #[test]
    fn partial_rows_survive_failed_and_truncated_profiles_but_empty_success_is_complete() {
        let users = "__VPNHIDE_PM_USERS_STATUS__:plain:0";
        let raw = "__VPNHIDE_PM_USER_BEGIN__:0\npackage:com.a uid:10123\n__VPNHIDE_PM_USER_END__:0:0\n__VPNHIDE_PM_USER_BEGIN__:10\n__VPNHIDE_PM_USER_END__:10:0\n__VPNHIDE_PM_USER_BEGIN__:11\npackage:com.b uid:1110123\n__VPNHIDE_PM_USER_END__:11:7\n__VPNHIDE_PM_USER_BEGIN__:12\npackage:com.c uid:1210123";
        let v = parse_inventory(raw, users, &[12, 10, 0, 11]).unwrap();
        assert_eq!(v.failed_user_ids, [11, 12]);
        assert!(v.user_list_complete);
        assert_eq!(v.packages["com.c"].uids_by_user[&12], [1210123]);
        assert!(
            !parse_inventory(raw, "__VPNHIDE_PM_USERS_STATUS__:plain:-1", &[0])
                .unwrap()
                .user_list_complete
        );
        assert!(!parse_inventory(raw, users, &[]).unwrap().user_list_complete);
    }
    #[test]
    fn unframed_multi_user_rows_partition_uids_instead_of_copying_every_uid_to_every_profile() {
        let v = parse_inventory(
            "package:com.a uid:10123,1010123,10123\rpackage:com.a uid:110123",
            "",
            &[],
        )
        .unwrap();
        assert_eq!(
            v.packages["com.a"].uids_by_user,
            BTreeMap::from([(0, vec![10123]), (1, vec![110123]), (10, vec![1010123])])
        );
    }
    #[test]
    fn unmatched_end_and_truncated_retry_cannot_inherit_a_successful_scan() {
        let users = "__VPNHIDE_PM_USERS_STATUS__:plain:0";
        let no_begin = parse_inventory("__VPNHIDE_PM_USER_END__:0:0", users, &[0]).unwrap();
        assert_eq!(no_begin.failed_user_ids, [0]);
        let retry = parse_inventory("__VPNHIDE_PM_USER_BEGIN__:0\npackage:com.a uid:10123\n__VPNHIDE_PM_USER_END__:0:0\n__VPNHIDE_PM_USER_BEGIN__:0\npackage:com.b uid:10234", users, &[0]).unwrap();
        assert_eq!(retry.failed_user_ids, [0]);
        assert_eq!(retry.packages.len(), 2);
    }
    #[test]
    fn too_large_observations_are_not_an_empty_successful_inventory() {
        let raw = "x".repeat(MAX_OBSERVATION_BYTES + 1);
        assert_eq!(parse_uid_map(&raw), Err(InventoryError::InputTooLarge));
        assert_eq!(
            parse_inventory("", &raw, &[0]),
            Err(InventoryError::InputTooLarge)
        );
    }
}
