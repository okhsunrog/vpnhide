//! Per-user package inventory (`pm_users` / `pm_packages`), shared by the root snapshot
//! and the debug export.
//!
//! Enumerates users first, then queries each one explicitly. The all-users shortcut is
//! incomplete on some OEM ROMs, and tests assert its flag never appears in what core runs.

use std::collections::BTreeSet;
use std::time::Duration;

use super::{Out, Ran, run, trimmed};
use crate::diagnostics::prefixes;

const PM_TIMEOUT: Duration = Duration::from_secs(20);

/// The pm argv core runs for one user's packages (pinned by tests).
pub fn packages_args(user: u32) -> Vec<String> {
    ["list", "packages", "-U", "-f", "--user"]
        .iter()
        .map(|s| s.to_string())
        .chain([user.to_string()])
        .collect()
}

/// `cmd package <args>`: the binder client, never the `pm` sh wrapper.
pub(crate) fn pm(args: &[&str], out: &Out) -> Ran {
    pm_within(args, out.remaining(PM_TIMEOUT))
}

pub(crate) fn pm_within(args: &[&str], timeout: std::time::Duration) -> Ran {
    let mut argv = vec![vpnhide_activator::PACKAGE_SERVICE_ARG];
    argv.extend_from_slice(args);
    run(vpnhide_activator::PACKAGE_SERVICE_CMD, &argv, timeout)
}

/// `fold_stderr`: the bundle folds pm's stderr into the section; the snapshot drops it.
pub fn emit(out: &mut Out, fold_stderr: bool) {
    let text = |r: &Ran| {
        if fold_stderr {
            r.combined()
        } else {
            r.stdout.clone()
        }
    };
    let verbose = pm(&["list", "users", "-v"], out);
    let plain = pm(&["list", "users"], out);
    let (verbose_text, plain_text) = (text(&verbose), text(&plain));
    let mut body = format!(
        "{}verbose:{}\n{}plain:{}\n",
        prefixes::PM_USERS_STATUS,
        verbose.exit,
        prefixes::PM_USERS_STATUS,
        plain.exit
    );
    for t in [&verbose_text, &plain_text] {
        let t = trimmed(t);
        if !t.is_empty() {
            body.push_str(t);
            body.push('\n');
        }
    }
    out.section("pm_users", body);

    let ids = user_ids(&plain_text, &verbose_text);
    let mut body = String::new();
    for id in ids {
        if out.expired() {
            break;
        }
        body.push_str(&format!("{}{id}\n", prefixes::PM_USER_BEGIN));
        let args = packages_args(id);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let listed = pm(&argv, out);
        super::push_lines(&mut body, &text(&listed));
        body.push_str(&format!("{}{id}:{}\n", prefixes::PM_USER_END, listed.exit));
    }
    out.section("pm_packages", body);
}

/// User ids from `pm list users` (`UserInfo{N:`) and `-v` (`N: id=M,`), sorted, unique;
/// `[0]` when neither names one.
pub fn user_ids(plain: &str, verbose: &str) -> Vec<u32> {
    let mut ids = BTreeSet::new();
    for line in plain.lines() {
        if let Some(rest) = line.split("UserInfo{").nth(1)
            && let Some((num, _)) = rest.split_once(':')
            && let Ok(id) = num.parse()
        {
            ids.insert(id);
        }
    }
    for line in verbose.lines() {
        let line = line.trim_start();
        if let Some((index, rest)) = line.split_once(':')
            && !index.is_empty()
            && index.bytes().all(|b| b.is_ascii_digit())
            && let Some(rest) = rest.trim_start().strip_prefix("id=")
            && let Some((num, _)) = rest.split_once(',')
            && let Ok(id) = num.parse()
        {
            ids.insert(id);
        }
    }
    if ids.is_empty() {
        ids.insert(0);
    }
    ids.into_iter().collect()
}
