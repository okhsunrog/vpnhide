//! Genuine Zygote worker + exact collector thread admission. No root-setuid
//! surrogate, caller-supplied birth identity, PID-lookup grant or UID bypass.
use super::super::native_exchange::NativeExchange;
use super::*;

pub(super) fn thread_identity(
    root: &Path,
    worker: WorkerIdentity,
    tid: u32,
) -> Result<DiagnosticThreadIdentity, DiagnosticError> {
    if tid == 0 || tid > i32::MAX as u32 {
        return Err(DiagnosticError::WrongProcess);
    }
    let process = root.join(worker.process.pid.to_string());
    let threads = process.join("task");
    let thread = threads.join(tid.to_string());
    if fs::metadata(&thread).map(|m| m.uid()).ok() != Some(worker.uid) {
        return Err(DiagnosticError::WrongProcess);
    }
    let birth =
        image::process_identity(&threads, tid).map_err(|_| DiagnosticError::WrongProcess)?;
    if birth.start_ticks < worker.process.start_ticks || birth.start_ticks == 0 {
        return Err(DiagnosticError::WrongProcess);
    }
    for namespace in ["pid", "user", "net"] {
        let own = namespace_identity(&root.join("self/ns").join(namespace))?;
        let current = namespace_identity(&thread.join("ns").join(namespace))?;
        let leader = namespace_identity(&process.join("ns").join(namespace))?;
        if own != current || own != leader {
            return Err(DiagnosticError::WrongProcess);
        }
    }
    // Re-read the leader after membership/namespace observations: a replaced
    // numeric process ID cannot lend its thread directory to the old worker.
    if image::process_identity(root, worker.process.pid)
        .map_err(|_| DiagnosticError::WrongProcess)?
        != worker.process
    {
        return Err(DiagnosticError::WrongProcess);
    }
    Ok(DiagnosticThreadIdentity {
        uid: worker.uid,
        pid: worker.process.pid,
        process_start_ticks: worker.process.start_ticks,
        tid,
        thread_start_ticks: birth.start_ticks,
    })
}
impl Coordinator {
    pub fn finish_worker(
        &self,
        peer: PeerCred,
        session: u64,
        epoch: u64,
    ) -> Result<(), DiagnosticError> {
        let mut inner = self.inner.lock().expect("diagnostic coordinator");
        let worker = worker_identity(
            Path::new("/proc"),
            inner.app_dir.as_ref().ok_or(DiagnosticError::Disabled)?,
            peer,
        )?;
        if inner.cancelled {
            return Err(DiagnosticError::Expired);
        }
        let active = inner.active.as_mut().ok_or(DiagnosticError::StaleSession)?;
        active.finish(worker, session, epoch, Instant::now(), |expected| {
            let current = thread_identity(Path::new("/proc"), worker, expected.tid)?;
            if current != expected {
                return Err(DiagnosticError::WrongProcess);
            }
            Ok(())
        })?;
        self.changed.notify_all();
        Ok(())
    }

    pub fn native_step(
        &self,
        peer: PeerCred,
        backend: Option<u32>,
        request: NativeWorkerRequest,
    ) -> Result<NativeWorkerInstruction, DiagnosticError> {
        let mut inner = self.inner.lock().expect("diagnostic coordinator");
        let worker = worker_identity(
            Path::new("/proc"),
            inner.app_dir.as_ref().ok_or(DiagnosticError::Disabled)?,
            peer,
        )?;
        if inner.cancelled {
            return Err(DiagnosticError::Expired);
        }
        let active = inner.active.as_mut().ok_or(DiagnosticError::StaleSession)?;
        let task = active.task.as_ref().ok_or(DiagnosticError::StaleSession)?;
        if active.deadline <= Instant::now() {
            return Err(DiagnosticError::Expired);
        }
        if active.worker != Some(worker) {
            return Err(DiagnosticError::WrongProcess);
        }
        if task.session != request.session
            || task.core_epoch != request.core_epoch
            || active.report.is_some()
        {
            return Err(DiagnosticError::StaleSession);
        }
        let identity = thread_identity(Path::new("/proc"), worker, request.tid)?;
        if request.phase == NativeWorkerPhase::Prepared {
            if active.native.is_some() {
                return Err(DiagnosticError::StaleSession);
            }
            // Reserve the attempt BEFORE IO. A lost/failed arm cannot trigger a
            // new nonce on repeated Prepared requests in this session.
            active.native = Some(NativeExchange::failed(identity, NativeRawStage::Arm));
            let (exchange, instruction) = NativeExchange::begin(identity, backend)?;
            active.native = Some(exchange);
            Ok(instruction)
        } else {
            active
                .native
                .as_mut()
                .ok_or(DiagnosticError::StaleSession)?
                .step(identity, request.phase)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stat(pid: u32, ticks: u64) -> String {
        format!("{pid} (collector) S {}{ticks}\n", "0 ".repeat(18))
    }
    #[test]
    fn thread_membership_birth_and_each_namespace_are_independently_revalidated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let process = root.join("42");
        let thread = process.join("task/43");
        for dir in [root.join("self/ns"), process.join("ns"), thread.join("ns")] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(process.join("stat"), stat(42, 100)).unwrap();
        fs::write(thread.join("stat"), stat(43, 101)).unwrap();
        for name in ["pid", "user", "net"] {
            let source = root.join("self/ns").join(name);
            fs::write(&source, name).unwrap();
            fs::hard_link(&source, process.join("ns").join(name)).unwrap();
            fs::hard_link(source, thread.join("ns").join(name)).unwrap();
        }
        let worker = WorkerIdentity {
            uid: fs::metadata(&thread).unwrap().uid(),
            process: ProcessIdentity {
                pid: 42,
                start_ticks: 100,
            },
            zygote: ProcessIdentity {
                pid: 2,
                start_ticks: 1,
            },
            network_namespace: (1, 2),
        };
        let original = thread_identity(root, worker, 43).unwrap();
        assert_eq!(original.thread_start_ticks, 101);
        assert_eq!(
            thread_identity(root, worker, 0),
            Err(DiagnosticError::WrongProcess)
        );
        assert_eq!(
            thread_identity(root, worker, 44),
            Err(DiagnosticError::WrongProcess)
        );
        let mut wrong_uid = worker;
        wrong_uid.uid ^= 1;
        assert_eq!(
            thread_identity(root, wrong_uid, 43),
            Err(DiagnosticError::WrongProcess)
        );
        for name in ["pid", "user", "net"] {
            let path = thread.join("ns").join(name);
            fs::remove_file(&path).unwrap();
            fs::write(&path, b"other namespace").unwrap();
            assert_eq!(
                thread_identity(root, worker, 43),
                Err(DiagnosticError::WrongProcess),
                "{name}"
            );
            fs::remove_file(&path).unwrap();
            fs::hard_link(root.join("self/ns").join(name), path).unwrap();
        }
        fs::write(thread.join("stat"), stat(43, 102)).unwrap();
        assert_ne!(thread_identity(root, worker, 43).unwrap(), original);
        fs::write(process.join("stat"), stat(42, 102)).unwrap();
        assert_eq!(
            thread_identity(root, worker, 43),
            Err(DiagnosticError::WrongProcess)
        );
    }
}
