//! Boot-thread readback/binding, before any loader worker or native selection.
//! Source hashing stays outside the command writer and main IPC startup path.
use super::*;
use crate::maintenance::zygisk_projection::{ProjectionPaths, work::ProjectionObservation};
use std::{fs, io, path::Path};
use vpnhide_core_rpc::{BackendChoice, NativeBackend};
use vpnhide_protocol::zygisk_activation::MANAGED_MODULE_ID;

impl CoreState {
    pub(crate) fn bind_projection_for_boot(
        &self,
        env: &crate::native_boot::NativeEnv,
        epoch: u64,
    ) -> Result<(), String> {
        let result = self.inspect_projection_for_boot(env);
        if let Err(reason) = &result {
            let reason = reason.clone();
            let mut env = env.clone();
            env.loader = Some(self.loader.clone());
            let boot = crate::native_boot::read_boot_id(&env.boot_id_path);
            self.call_writer(move |state| -> Result<(), String> {
                if state.epoch.load(Ordering::SeqCst) != epoch {
                    return Err("projection failure belongs to an older Core epoch".into());
                }
                if boot.is_empty() || boot.len() > 128 {
                    return Err("projection boot identity unavailable".into());
                }
                let mut journal = state
                    .engine
                    .store
                    .lock()
                    .expect("config mutex")
                    .journal
                    .clone();
                if !journal.zygisk_exposed_in(&boot) {
                    journal.note_zygisk_exposure(&boot);
                    let path = state
                        .journal_path
                        .as_ref()
                        .ok_or("projection recovery needs durable journal")?;
                    state
                        .persist_transition_journal(path, &journal, OperationId(0))
                        .map_err(|e| e.to_string())?;
                }
                state.install_native_boot(crate::native_boot::NativeBoot::blocked(
                    env, epoch, reason,
                ));
                Ok(())
            })
            .map_err(|e| format!("projection recovery writer unavailable: {e:?}"))??;
        }
        result
    }
    fn inspect_projection_for_boot(
        &self,
        env: &crate::native_boot::NativeEnv,
    ) -> Result<(), String> {
        self.inspect_projection_for_boot_owned(env, None, 0)
    }
    fn inspect_projection_for_boot_owned(
        &self,
        env: &crate::native_boot::NativeEnv,
        running: Option<&fs::File>,
        uid: u32,
    ) -> Result<(), String> {
        let Some(modules) = &env.modules else {
            return Ok(());
        };
        let active = modules.join(MANAGED_MODULE_ID);
        let exists = match fs::symlink_metadata(&active) {
            Ok(_) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("cannot inspect managed discovery: {e}")),
        };
        let before = self
            .engine
            .store
            .lock()
            .expect("config mutex")
            .journal
            .clone();
        let work = before
            .zygisk_projection
            .as_ref()
            .filter(|work| work.retained());
        if !exists && work.is_none() {
            return Ok(());
        }
        let work = work.ok_or("managed discovery has no retained Core ownership")?;
        if work.outcome != PhaseOutcome::Confirmed
            || !journal::backend_choice::validate_entries(&before)
            || !crate::maintenance::zygisk_projection::work::validate_journal(&before)
            || before.backend_choice.as_ref().is_none_or(|choice| {
                choice.operation != work.operation
                    || choice.request.choice
                        != (BackendChoice::Fixed {
                            backend: NativeBackend::Zygisk,
                        })
            })
        {
            return Err("managed discovery preparation is unverified".into());
        }
        let root = self
            .effect_paths
            .read()
            .expect("effect paths")
            .as_ref()
            .ok_or("projection profile unavailable")?
            .root
            .clone();
        if modules != &root.join("data/adb/modules") {
            return Err("projection namespace differs from Core profile".into());
        }
        let paths = ProjectionPaths {
            root,
            persistent: self
                .journal_path
                .as_deref()
                .and_then(Path::parent)
                .ok_or("projection journal unavailable")?
                .into(),
        };
        let boot = crate::native_boot::read_boot_id(&env.boot_id_path);
        let actual;
        let running = match running {
            Some(file) => file,
            None => {
                actual = fs::File::open("/proc/self/exe").map_err(|e| e.to_string())?;
                &actual
            }
        };
        let observe =
            |authority| work.observe_with_authority_owned(&paths, &boot, running, uid, authority);
        let observation = if work.boot_id == boot {
            // A new staged choice never authorizes hooks in its requested boot.
            observe(None)
        } else {
            let mut observed = observe(None);
            for authority in before.retained_zygisk_authorities() {
                if observed.is_ok() {
                    break;
                }
                observed = observe(Some(authority));
            }
            observed
        }
        .map_err(|e| format!("managed discovery source/code ownership: {e}"))?;
        if let ProjectionObservation::ActiveAfterBoot(readback) = observation {
            self.loader
                .bind_projection_owned(
                    &paths.source(),
                    &paths.active(),
                    &readback,
                    uid,
                    &boot,
                    self.epoch.load(Ordering::SeqCst),
                )
                .map_err(|e| e.to_string())?;
        }
        let after = self.engine.store.lock().expect("config mutex");
        if after.journal.zygisk_projection != before.zygisk_projection
            || after.journal.backend_choice != before.backend_choice
            || after.journal.zygisk_authority != before.zygisk_authority
            || after.journal.prior_zygisk_authority != before.prior_zygisk_authority
            || after.journal.zygisk_bootstrap != before.zygisk_bootstrap
            || crate::native_boot::read_boot_id(&env.boot_id_path) != boot
        {
            return Err("projection state changed during readback".into());
        }
        Ok(())
    }
}

mod bootstrap;
#[cfg(test)]
mod tests;
