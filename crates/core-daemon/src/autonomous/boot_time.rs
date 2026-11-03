//! Suspend-aware time for bounded diagnostic authority and resource ownership.
//! Condvar timeouts use CLOCK_MONOTONIC on Android/Linux, so active waits must
//! recheck CLOCK_BOOTTIME in short slices after resume. This does not wake a
//! sleeping device or claim cleanup can execute while the CPU is suspended.
use std::{io, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct BootInstant(Duration);

#[derive(Clone, Copy, Debug)]
pub(super) struct BootDeadline {
    start: BootInstant,
    end: BootInstant,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid diagnostic boot clock")
}

impl BootInstant {
    pub fn now() -> io::Result<Self> {
        let mut time: libc::timespec = unsafe { std::mem::zeroed() };
        if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Self::from_timespec(time)
    }

    fn from_timespec(time: libc::timespec) -> io::Result<Self> {
        let seconds = u64::try_from(time.tv_sec).map_err(|_| invalid())?;
        let nanos = u32::try_from(time.tv_nsec).map_err(|_| invalid())?;
        if nanos >= 1_000_000_000 {
            return Err(invalid());
        }
        Ok(Self(Duration::new(seconds, nanos)))
    }

    pub fn millis(self) -> io::Result<u64> {
        self.0.as_millis().try_into().map_err(|_| invalid())
    }

    pub fn ticks(self, frequency: u64) -> io::Result<u64> {
        if frequency == 0 {
            return Err(invalid());
        }
        let ticks = self
            .0
            .as_nanos()
            .checked_mul(u128::from(frequency))
            .ok_or_else(invalid)?
            / 1_000_000_000;
        ticks.try_into().map_err(|_| invalid())
    }

    pub fn after(self, duration: Duration) -> io::Result<BootDeadline> {
        Ok(BootDeadline {
            start: self,
            end: Self(self.0.checked_add(duration).ok_or_else(invalid)?),
        })
    }

    #[cfg(test)]
    pub fn at(seconds: u64) -> Self {
        Self(Duration::from_secs(seconds))
    }
}

impl BootDeadline {
    pub fn from_now(duration: Duration) -> io::Result<Self> {
        BootInstant::now()?.after(duration)
    }

    pub fn expires_millis(self) -> io::Result<u64> {
        self.end.millis()
    }

    pub fn remaining(self, now: BootInstant) -> Option<Duration> {
        // A regressed or inconsistent clock cannot extend authority.
        if now < self.start {
            return None;
        }
        self.end.0.checked_sub(now.0).filter(|d| !d.is_zero())
    }

    pub fn expired(self, now: BootInstant) -> bool {
        self.remaining(now).is_none()
    }

    pub fn wait_slice(self, now: BootInstant) -> Option<Duration> {
        self.remaining(now)
            .map(|remaining| remaining.min(Duration::from_millis(250)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspend_time_expires_authority_and_bounds_monotonic_wait_after_resume() {
        let deadline = BootInstant::at(100).after(Duration::from_secs(20)).unwrap();
        assert_eq!(
            deadline.remaining(BootInstant::at(100)),
            Some(Duration::from_secs(20))
        );
        assert_eq!(
            deadline.wait_slice(BootInstant::at(100)),
            Some(Duration::from_millis(250))
        );
        // No advance of the Condvar's monotonic clock is needed to expire it.
        assert!(deadline.expired(BootInstant::at(121)));
        assert_eq!(deadline.wait_slice(BootInstant::at(121)), None);
        assert!(deadline.expired(BootInstant::at(120)));
        assert!(deadline.expired(BootInstant::at(99)));
    }

    #[test]
    fn clock_fields_conversions_and_overflow_are_checked() {
        let time = BootInstant::from_timespec(libc::timespec {
            tv_sec: 3,
            tv_nsec: 990_000_000,
        })
        .unwrap();
        assert_eq!(time.millis().unwrap(), 3990);
        assert_eq!(time.ticks(100).unwrap(), 399);
        assert!(time.ticks(0).is_err());
        for (tv_sec, tv_nsec) in [(-1, 0), (0, -1), (0, 1_000_000_000)] {
            assert!(BootInstant::from_timespec(libc::timespec { tv_sec, tv_nsec }).is_err());
        }
        let maximum = BootInstant(Duration::MAX);
        assert!(maximum.after(Duration::from_nanos(1)).is_err());
        assert!(maximum.millis().is_err());
        assert!(maximum.ticks(u64::MAX).is_err());
        let first = BootInstant::now().unwrap();
        assert!(BootInstant::now().unwrap() >= first);
    }
}
