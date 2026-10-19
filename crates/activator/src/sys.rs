//! Direct kernel / bionic interfaces core uses instead of spawning `dmesg`, `uname`,
//! `getprop`, `log` or `insmod`. Core runs no shell and spawns a system binary only where
//! the binary *is* the interface (pm, iptables, kpatch, logcat, dumpsys, ...).

use std::ffi::CString;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

/// The kernel ring buffer (`dmesg`), via `klogctl(SYSLOG_ACTION_READ_ALL)`.
pub fn kernel_log() -> io::Result<String> {
    const SYSLOG_ACTION_READ_ALL: libc::c_int = 3;
    const SYSLOG_ACTION_SIZE_BUFFER: libc::c_int = 10;
    // SAFETY: a size query takes no buffer.
    let size = unsafe { libc::klogctl(SYSLOG_ACTION_SIZE_BUFFER, std::ptr::null_mut(), 0) };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }
    let size = (size as usize).clamp(16 * 1024, 16 * 1024 * 1024);
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` is valid for `size` bytes for the duration of the call.
    let read = unsafe {
        libc::klogctl(
            SYSLOG_ACTION_READ_ALL,
            buf.as_mut_ptr().cast(),
            size as libc::c_int,
        )
    };
    if read < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(read as usize);
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// `dmesg -c`'s clear, via `klogctl(SYSLOG_ACTION_CLEAR)`.
pub fn clear_kernel_log() -> io::Result<()> {
    const SYSLOG_ACTION_CLEAR: libc::c_int = 5;
    // SAFETY: a clear takes no buffer.
    if unsafe { libc::klogctl(SYSLOG_ACTION_CLEAR, std::ptr::null_mut(), 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `uname(2)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Uname {
    pub sysname: String,
    pub nodename: String,
    pub release: String,
    pub version: String,
    pub machine: String,
}

impl Uname {
    /// The `uname -a` line (without the trailing "GNU/Linux"-style OS field).
    pub fn all(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.sysname, self.nodename, self.release, self.version, self.machine
        )
    }
}

pub fn uname() -> io::Result<Uname> {
    // SAFETY: utsname is plain C data; uname fills it.
    let mut raw: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `raw` is a valid, writable utsname.
    if unsafe { libc::uname(&mut raw) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // c_char is i8 on x86 hosts and u8 on ARM Android.
    #[allow(clippy::unnecessary_cast)]
    fn field(chars: &[libc::c_char]) -> String {
        let bytes: Vec<u8> = chars
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
    Ok(Uname {
        sysname: field(&raw.sysname),
        nodename: field(&raw.nodename),
        release: field(&raw.release),
        version: field(&raw.version),
        machine: field(&raw.machine),
    })
}

/// An Android system property (`getprop name`), via bionic. `None` off-device or unset.
pub fn property(name: &str) -> Option<String> {
    #[cfg(target_os = "android")]
    {
        let name = CString::new(name).ok()?;
        let mut value = [0 as libc::c_char; 92]; // PROP_VALUE_MAX
        // SAFETY: `value` holds PROP_VALUE_MAX bytes, as the API requires.
        let len = unsafe { libc::__system_property_get(name.as_ptr(), value.as_mut_ptr()) };
        if len <= 0 {
            return None;
        }
        // SAFETY: bionic NUL-terminates within PROP_VALUE_MAX.
        let value = unsafe { std::ffi::CStr::from_ptr(value.as_ptr()) };
        Some(value.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = name;
        None
    }
}

#[cfg(target_os = "android")]
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(
        prio: libc::c_int,
        tag: *const libc::c_char,
        text: *const libc::c_char,
    ) -> libc::c_int;
}

/// One INFO line in the Android log (`log -t tag msg`), via liblog. No-op off-device.
pub fn android_log(tag: &str, message: &str) {
    #[cfg(target_os = "android")]
    {
        const ANDROID_LOG_INFO: libc::c_int = 4;
        let (Ok(tag), Ok(text)) = (CString::new(tag), CString::new(message.replace('\0', " ")))
        else {
            return;
        };
        // SAFETY: both strings are valid NUL-terminated C strings for the call.
        unsafe {
            __android_log_write(ANDROID_LOG_INFO, tag.as_ptr(), text.as_ptr());
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (tag, message);
    }
}

/// The SELinux label of `path` (not following a final symlink), or `None`.
pub fn selinux_label(path: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let name = c"security.selinux";
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is writable for its length; both names are NUL-terminated.
    let len = unsafe {
        libc::lgetxattr(
            path.as_ptr(),
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    if len <= 0 {
        return None;
    }
    let raw = &buf[..len as usize];
    let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
    Some(String::from_utf8_lossy(raw).into_owned())
}

/// Load a kernel module from `artifact` with `params` (`insmod`), via `finit_module(2)`.
/// The error carries the errno, e.g. `EKEYREJECTED` for a signature the kernel refused.
pub fn finit_module(artifact: &Path, params: &str) -> io::Result<()> {
    let file = std::fs::File::open(artifact)?;
    let params =
        CString::new(params).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: a valid open fd and a NUL-terminated parameter string; flags 0.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_finit_module,
            file.as_raw_fd(),
            params.as_ptr(),
            0 as libc::c_int,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The calling process's uid/gid/groups, as `id` prints them.
pub fn id_line() -> String {
    // SAFETY: these getters cannot fail.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let mut groups = vec![0 as libc::gid_t; 256];
    // SAFETY: `groups` is writable for its length.
    let n = unsafe { libc::getgroups(groups.len() as libc::c_int, groups.as_mut_ptr()) };
    groups.truncate(n.max(0) as usize);
    let context = std::fs::read("/proc/self/attr/current")
        .map(|raw| {
            String::from_utf8_lossy(&raw)
                .trim_end_matches(['\0', '\n'])
                .to_owned()
        })
        .unwrap_or_default();
    let mut line = format!("uid={uid} gid={gid}");
    if !groups.is_empty() {
        let list: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
        line.push_str(&format!(" groups={}", list.join(",")));
    }
    if !context.is_empty() {
        line.push_str(&format!(" context={context}"));
    }
    line
}

/// `getuid()`.
pub fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uname_reports_a_release() {
        let u = uname().unwrap();
        assert!(!u.release.is_empty());
        assert!(u.all().contains(&u.release));
    }

    #[test]
    fn id_line_starts_with_the_uid() {
        assert!(id_line().starts_with(&format!("uid={} ", uid())));
    }

    #[test]
    fn finit_module_reports_a_missing_artifact() {
        let err = finit_module(Path::new("/nonexistent/vpnhide_kmod.ko"), "").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn host_has_no_android_property() {
        #[cfg(not(target_os = "android"))]
        assert_eq!(property("ro.build.fingerprint"), None);
    }
}
