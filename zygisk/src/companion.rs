//! Public V2 companion API only. All descriptors close before specialization.
//! Missing/unsupported evidence never becomes a successful loader observation.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use vpnhide_loader_ipc::{
    Acknowledgement, CORE_SOCKET, Callback, Challenge, CompanionReport, VERSION, image, transport,
};
use zygisk_api::api::{ZygiskApi, v2::V2};

#[inline(never)]
fn anchor() {
    std::hint::black_box(1u8);
}

pub fn report(
    api: &mut ZygiskApi<'_, V2>,
    directory: &OwnedFd,
    target_uid: u32,
    process: String,
    generation: Option<u64>,
    policy_sha256: Option<[u8; 32]>,
) -> io::Result<()> {
    let image = image::own_backing_image(anchor as *const () as usize as u64)?;
    api.with_companion(|stream| {
        let ready = transport::receive::<Challenge>(stream, 0, false)?.value;
        if !ready.valid() {
            return Err(io::Error::other("invalid Core loader challenge"));
        }
        let boot =
            image::bounded_text(std::path::Path::new("/proc/sys/kernel/random/boot_id"), 128)?;
        if ready.boot_id != boot.trim() {
            return Err(io::Error::other("loader challenge belongs to another boot"));
        }
        let callback = Callback {
            version: VERSION,
            nonce: ready.nonce,
            anchor: anchor as *const () as usize as u64,
            target_uid,
            process,
            generation,
            policy_sha256,
        };
        if !callback.valid() {
            return Err(io::Error::other("invalid loader callback"));
        }
        transport::send(
            stream,
            &callback,
            &[image.as_raw_fd(), directory.as_raw_fd()],
        )?;
        let ack = transport::receive::<Acknowledgement>(stream, 0, false)?.value;
        if !ack.accepted || ack.nonce != ready.nonce {
            return Err(io::Error::other("loader evidence not acknowledged"));
        }
        Ok(())
    })
    .map_err(|_| io::Error::other("Zygisk companion unavailable"))?
}

pub fn entry(child: &mut UnixStream) {
    // Failure is deliberately silent; Core's pending run records unavailability.
    let _ = relay(child);
}

fn relay(child: &mut UnixStream) -> io::Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(io::Error::other("companion is not root"));
    }
    let mut core = transport::connect(std::path::Path::new(CORE_SOCKET))?;
    let ready = transport::receive::<Challenge>(&mut core, 0, false)?.value;
    if !ready.valid() {
        return Err(io::Error::other("invalid loader challenge"));
    }
    // Enable BEFORE Ready. Request-time credentials refer to the actual sender,
    // even when the manager relays this stream from a socketpair/another process.
    transport::pass_credentials(child)?;
    transport::send(child, &ready, &[])?;
    let request = transport::receive::<Callback>(child, 2, true)?;
    if !request.value.valid() || request.value.nonce != ready.nonce {
        return Err(io::Error::other("invalid callback challenge"));
    }
    let credentials = request
        .credentials
        .ok_or_else(|| io::Error::other("missing callback sender identity"))?;
    if credentials.uid != 0 {
        return Err(io::Error::other("callback arrived after specialization"));
    }
    let report = CompanionReport {
        child: credentials,
        companion_anchor: anchor as *const () as usize as u64,
        callback: request.value,
    };
    let fds: Vec<_> = request.descriptors.iter().map(AsRawFd::as_raw_fd).collect();
    transport::send(&mut core, &report, &fds)?;
    let ack = transport::receive::<Acknowledgement>(&mut core, 0, false)?.value;
    if ack.nonce != ready.nonce {
        return Err(io::Error::other("stale loader acknowledgement"));
    }
    transport::send(child, &ack, &[])
}
