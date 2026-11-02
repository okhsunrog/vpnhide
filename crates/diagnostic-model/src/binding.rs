//! Independent socket binding evidence. Root differential is not app RAW; a
//! libc result cannot establish the direct-syscall actor's coverage.
use crate::{ControlFailure, NativeFixtureSpec, NativeMeasurement, ProbeError, Sample};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindActor {
    Libc,
    Syscall,
}
impl BindActor {
    pub const ALL: [Self; 2] = [Self::Libc, Self::Syscall];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindAttempt {
    pub initial: Sample<String>,
    pub set: Sample<u32>,
    pub final_device: Sample<String>,
}
impl BindAttempt {
    fn bounded(&self) -> bool {
        [&self.initial, &self.final_device].into_iter().all(|v| {
            v.value().is_none_or(|name| {
                name.len() <= 15 && name.is_ascii() && !name.bytes().any(|b| b.is_ascii_control())
            })
        })
    }
    fn starts_unbound(&self) -> bool {
        self.initial.value().is_some_and(String::is_empty)
    }
    fn bound_to(&self, name: &str) -> bool {
        self.starts_unbound()
            && self.set.value() == Some(&0)
            && self.final_device.value().is_some_and(|v| v == name)
    }
    fn absent(&self) -> bool {
        self.starts_unbound()
            && self.final_device.value().is_some_and(String::is_empty)
            && matches!(self.set, Sample::Error(ProbeError::Errno(19 | 6)))
    }
    fn protected(&self) -> bool {
        self.starts_unbound()
            && self.final_device.value().is_some_and(String::is_empty)
            && (self.set.value() == Some(&0)
                || matches!(self.set, Sample::Error(ProbeError::Errno(19 | 6))))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindMeasurement {
    pub libc_fixture: BindAttempt,
    pub libc_ordinary: BindAttempt,
    pub syscall_fixture: BindAttempt,
    pub syscall_ordinary: BindAttempt,
}
impl BindMeasurement {
    pub fn bounded(&self) -> bool {
        [
            &self.libc_fixture,
            &self.libc_ordinary,
            &self.syscall_fixture,
            &self.syscall_ordinary,
        ]
        .into_iter()
        .all(BindAttempt::bounded)
    }
    fn actor(&self, actor: BindActor) -> (&BindAttempt, &BindAttempt) {
        match actor {
            BindActor::Libc => (&self.libc_fixture, &self.libc_ordinary),
            BindActor::Syscall => (&self.syscall_fixture, &self.syscall_ordinary),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedBindResult {
    pub proven: Vec<BindActor>,
    pub leaked: Vec<BindActor>,
    pub controls: Vec<ControlFailure>,
}
impl ScopedBindResult {
    pub fn complete(&self) -> bool {
        self.controls.is_empty()
            && self.leaked.is_empty()
            && BindActor::ALL.iter().all(|a| self.proven.contains(a))
    }
}
fn problem(result: &mut ScopedBindResult, reason: ControlFailure) {
    if !result.controls.contains(&reason) {
        result.controls.push(reason);
    }
}
pub fn evaluate_bind_fixture(
    spec: &NativeFixtureSpec,
    negative: &NativeMeasurement,
    root_start: &NativeMeasurement,
    worker: &NativeMeasurement,
    root_end: &NativeMeasurement,
    cleanup: &NativeMeasurement,
) -> ScopedBindResult {
    let mut result = ScopedBindResult::default();
    if !spec.valid() {
        problem(&mut result, ControlFailure::InvalidSpec);
        return result;
    }
    let [Some(negative), Some(start), Some(worker)] =
        [negative, root_start, worker].map(|m| m.binding.as_ref())
    else {
        problem(&mut result, ControlFailure::ProbeUnavailable);
        return result;
    };
    if [negative, start, worker].into_iter().any(|m| !m.bounded()) {
        problem(&mut result, ControlFailure::InvalidSpec);
        return result;
    }
    for actor in BindActor::ALL {
        let (n, no) = negative.actor(actor);
        let (s, so) = start.actor(actor);
        let (w, wo) = worker.actor(actor);
        if !n.absent() || !no.bound_to("lo") {
            problem(&mut result, ControlFailure::MissingNegativeControl);
            continue;
        }
        if !super::positive(spec, root_start)
            || !s.bound_to(&spec.interface.name)
            || !so.bound_to("lo")
        {
            problem(&mut result, ControlFailure::MissingPositiveControl);
            continue;
        }
        // A failing setsockopt may already have mutated sk_bound_dev_if. Always
        // preserve exact positive readback evidence even under later failures.
        let leaked = [&w.initial, &w.final_device]
            .into_iter()
            .any(|v| v.value().is_some_and(|name| name == &spec.interface.name));
        if leaked {
            result.leaked.push(actor);
        }
        let mut controlled = true;
        if !w.starts_unbound() {
            problem(&mut result, ControlFailure::ProbeUnavailable);
            controlled = false;
        }
        let end_valid = super::positive(spec, root_end)
            && root_end.binding.as_ref().is_some_and(|end| {
                let (e, eo) = end.actor(actor);
                end.bounded() && e.bound_to(&spec.interface.name) && eo.bound_to("lo")
            });
        if !end_valid {
            problem(&mut result, ControlFailure::FixtureChanged);
            controlled = false;
        }
        let cleanup_valid = super::cleanup_confirmed(spec, cleanup)
            && cleanup.binding.as_ref().is_some_and(|cleanup| {
                let (c, co) = cleanup.actor(actor);
                cleanup.bounded() && c.absent() && co.bound_to("lo")
            });
        if !cleanup_valid {
            problem(&mut result, ControlFailure::CleanupUnconfirmed);
            controlled = false;
        }
        if !wo.bound_to("lo") {
            let available = wo.initial.value().is_some()
                && wo.final_device.value().is_some()
                && wo.set.value() == Some(&0);
            problem(
                &mut result,
                if available {
                    ControlFailure::OrdinaryObjectsChanged
                } else {
                    ControlFailure::ProbeUnavailable
                },
            );
            controlled = false;
        }
        if !leaked && !w.protected() {
            problem(&mut result, ControlFailure::ProbeUnavailable);
            controlled = false;
        }
        if controlled && !leaked {
            result.proven.push(actor);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        InterfaceId, ObservedInterface, OrdinaryControl, ScopedNativeResult,
        evaluate_native_fixture,
    };
    fn spec() -> NativeFixtureSpec {
        NativeFixtureSpec {
            session: 1,
            interface: InterfaceId {
                name: "tun000000000001".into(),
                index: 42,
            },
            flags: 0,
            mtu: 1543,
            ordinary: OrdinaryControl {
                interface: InterfaceId {
                    name: "lo".into(),
                    index: 1,
                },
                flags: 73,
                mtu: 65536,
            },
        }
    }
    fn bound(name: &str) -> BindAttempt {
        BindAttempt {
            initial: Sample::Value(String::new()),
            set: Sample::Value(0),
            final_device: Sample::Value(name.into()),
        }
    }
    fn absent() -> BindAttempt {
        BindAttempt {
            initial: Sample::Value(String::new()),
            set: Sample::Error(ProbeError::Errno(19)),
            final_device: Sample::Value(String::new()),
        }
    }
    fn observation(present: bool) -> NativeMeasurement {
        let spec = spec();
        let target = if present {
            bound(&spec.interface.name)
        } else {
            absent()
        };
        let mut interfaces: Vec<ObservedInterface> = vec![spec.ordinary.interface.clone().into()];
        if present {
            interfaces.push(spec.interface.clone().into());
        }
        NativeMeasurement {
            binding: Some(BindMeasurement {
                libc_fixture: target.clone(),
                libc_ordinary: bound("lo"),
                syscall_fixture: target,
                syscall_ordinary: bound("lo"),
            }),
            flags: if present {
                Sample::Value(spec.flags)
            } else {
                Sample::Error(ProbeError::Errno(19))
            },
            mtu: if present {
                Sample::Value(spec.mtu)
            } else {
                Sample::Error(ProbeError::Errno(19))
            },
            ifaddrs: Sample::Value(interfaces.clone()),
            links: Sample::Value(interfaces),
            ordinary_flags: Sample::Value(spec.ordinary.flags),
            ordinary_mtu: Sample::Value(spec.ordinary.mtu),
        }
    }
    fn evaluate(worker: &NativeMeasurement) -> ScopedBindResult {
        evaluate_bind_fixture(
            &spec(),
            &observation(false),
            &observation(true),
            worker,
            &observation(true),
            &observation(false),
        )
    }
    #[test]
    fn both_actors_require_exact_positive_negative_ordinary_and_readback_controls() {
        let worker = observation(false);
        assert!(evaluate(&worker).complete());
        let mut missing = worker.clone();
        missing.binding = None;
        assert!(!evaluate(&missing).complete());
        for errno in [1, 13, 92] {
            let mut worker = worker.clone();
            worker.binding.as_mut().unwrap().libc_fixture.set =
                Sample::Error(ProbeError::Errno(errno));
            assert!(!evaluate(&worker).complete());
            assert!(evaluate(&worker).leaked.is_empty());
        }
    }
    #[test]
    fn returned_error_after_socket_mutation_is_a_leak_and_actor_coverage_is_independent() {
        let mut worker = observation(false);
        worker
            .binding
            .as_mut()
            .unwrap()
            .syscall_fixture
            .final_device = Sample::Value(spec().interface.name);
        let result = evaluate(&worker);
        assert_eq!(result.leaked, vec![BindActor::Syscall]);
        assert_eq!(result.proven, vec![BindActor::Libc]);
        assert!(!result.complete());
    }
    #[test]
    fn successful_set_without_readback_does_not_prove_neutralization() {
        let mut worker = observation(false);
        let attempt = &mut worker.binding.as_mut().unwrap().libc_fixture;
        attempt.set = Sample::Value(0);
        attempt.final_device = Sample::Error(ProbeError::Unsupported);
        let result = evaluate(&worker);
        assert!(!result.complete());
        assert!(result.leaked.is_empty());
        assert!(result.controls.contains(&ControlFailure::ProbeUnavailable));
    }
    #[test]
    fn unavailable_bind_control_cannot_erase_other_four_native_proofs() {
        let mut worker = observation(false);
        worker.binding.as_mut().unwrap().libc_ordinary.set = Sample::Error(ProbeError::Errno(1));
        assert!(!evaluate(&worker).complete());
        let result = evaluate_native_fixture(
            &spec(),
            &observation(false),
            &observation(true),
            &worker,
            &observation(true),
            &observation(false),
        );
        assert!(matches!(result, ScopedNativeResult::Proven { checks } if checks.len() == 4));
    }
    #[test]
    fn cleanup_or_fixture_change_preserves_independently_observed_bind_leaks() {
        let mut worker = observation(false);
        worker.binding.as_mut().unwrap().libc_fixture = bound(&spec().interface.name);
        for (end, cleanup) in [
            (observation(false), observation(false)),
            (observation(true), observation(true)),
        ] {
            let result = evaluate_bind_fixture(
                &spec(),
                &observation(false),
                &observation(true),
                &worker,
                &end,
                &cleanup,
            );
            assert_eq!(result.leaked, vec![BindActor::Libc]);
            assert!(!result.complete());
            assert!(!result.controls.is_empty());
        }
    }
    #[test]
    fn missing_late_binding_controls_cannot_erase_confirmed_worker_leak() {
        let mut worker = observation(false);
        worker.binding.as_mut().unwrap().libc_fixture = bound(&spec().interface.name);
        for missing_end in [false, true] {
            let mut end = observation(true);
            let mut cleanup = observation(false);
            if missing_end {
                end.binding = None;
            } else {
                cleanup.binding = None;
            }
            let result = evaluate_bind_fixture(
                &spec(),
                &observation(false),
                &observation(true),
                &worker,
                &end,
                &cleanup,
            );
            assert_eq!(result.leaked, vec![BindActor::Libc]);
            assert!(result.proven.is_empty());
            assert!(!result.controls.is_empty());
        }
    }
}
