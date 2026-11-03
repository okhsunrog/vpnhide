//! Local unprivileged adapters for shared PackageManager observations.
use std::collections::HashMap;
use vpnhide_native_policy::package_inventory as model;

#[derive(Debug, PartialEq, Eq, uniffi::Error)]
pub enum CorePackageInventoryError {
    InputTooLarge,
}
impl std::fmt::Display for CorePackageInventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PackageManager observation exceeds the shared input bound")
    }
}
impl std::error::Error for CorePackageInventoryError {}
impl From<model::InventoryError> for CorePackageInventoryError {
    fn from(value: model::InventoryError) -> Self {
        match value {
            model::InventoryError::InputTooLarge => Self::InputTooLarge,
        }
    }
}

#[derive(Debug, PartialEq, Eq, uniffi::Record)]
pub struct CorePackageInventoryEntry {
    pub apk_path: Option<String>,
    pub uids_by_user: HashMap<i32, Vec<i32>>,
}
#[derive(Debug, PartialEq, Eq, uniffi::Record)]
pub struct CorePackageInventory {
    pub packages: HashMap<String, CorePackageInventoryEntry>,
    pub failed_user_ids: Vec<i32>,
    pub user_list_complete: bool,
}

#[uniffi::export]
pub fn core_parse_package_uid_map(
    raw: String,
) -> Result<HashMap<String, Vec<i32>>, CorePackageInventoryError> {
    Ok(model::parse_uid_map(&raw)?
        .into_iter()
        .map(|(name, uids)| (name, uids.into_iter().map(|uid| uid as i32).collect()))
        .collect())
}

#[uniffi::export]
pub fn core_parse_package_inventory(
    packages_raw: String,
    users_raw: String,
    expected_users: Vec<i32>,
) -> Result<CorePackageInventory, CorePackageInventoryError> {
    let users: Vec<_> = expected_users
        .into_iter()
        .filter_map(|id| u32::try_from(id).ok())
        .collect();
    let v = model::parse_inventory(&packages_raw, &users_raw, &users)?;
    Ok(CorePackageInventory {
        packages: v
            .packages
            .into_iter()
            .map(|(name, entry)| {
                (
                    name,
                    CorePackageInventoryEntry {
                        apk_path: entry.apk_path,
                        // The parser restricts Android IDs to nonnegative signed32.
                        uids_by_user: entry
                            .uids_by_user
                            .into_iter()
                            .map(|(id, uids)| {
                                (id as i32, uids.into_iter().map(|uid| uid as i32).collect())
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
        failed_user_ids: v.failed_user_ids.into_iter().map(|id| id as i32).collect(),
        user_list_complete: v.user_list_complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_boundary_retains_paths_partial_failure_and_never_wraps_an_unsigned_uid() {
        let raw = "__VPNHIDE_PM_USER_BEGIN__:10\npackage:/data/a b/base.apk=com.a uid:10123,1010123,2147483648\n__VPNHIDE_PM_USER_END__:10:7";
        let v = core_parse_package_inventory(
            raw.into(),
            "__VPNHIDE_PM_USERS_STATUS__:plain:0".into(),
            vec![0, 10],
        )
        .unwrap();
        assert_eq!(v.failed_user_ids, [0, 10]);
        assert!(v.user_list_complete);
        assert_eq!(v.packages["com.a"].uids_by_user[&10], [10123, 1010123]);
        assert_eq!(
            v.packages["com.a"].apk_path.as_deref(),
            Some("/data/a b/base.apk")
        );
        assert_eq!(
            core_parse_package_uid_map(raw.into()).unwrap()["com.a"],
            [10123, 1010123]
        );
    }
}
