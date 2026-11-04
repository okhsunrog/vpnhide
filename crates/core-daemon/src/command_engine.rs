//! Single command owner. RPC waits for durable admission, internal maintenance
//! waits for completion; neither waiter owns or cancels accepted execution.
use crate::server::CoreState;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use vpnhide_core_rpc::{Accepted, CoreCommand, OperationId, SubmitError};

pub(crate) type AdmissionReply = mpsc::Sender<Result<Accepted, SubmitError>>;

mod queue_thread_state {
    thread_local! {
        pub(super) static RPC_QUEUE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
}
use queue_thread_state::RPC_QUEUE;
// The blocking RPC owner scopes its queue policy. Internal boot, restoration and
// shutdown retain completion semantics; the writer thread never inherits this flag.
pub(crate) struct RpcQueueBudget(bool);
impl RpcQueueBudget {
    pub(crate) fn enter() -> Self {
        Self(RPC_QUEUE.replace(true))
    }
}
impl Drop for RpcQueueBudget {
    fn drop(&mut self) {
        RPC_QUEUE.set(self.0);
    }
}

const QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

// Only Queued -> Started grants execution. Dropping a caller or expiring its
// queue budget may win Queued -> Cancelled, never cancel Started/durable work.
#[derive(Clone)]
pub(crate) struct AdmissionTicket {
    state: Arc<AtomicU8>,
    deadline: std::time::Instant,
    preparation_cancelled: Option<tokio_util::sync::CancellationToken>,
}
impl AdmissionTicket {
    pub(crate) fn new(deadline: std::time::Instant) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(0)),
            preparation_cancelled: None,
            deadline: deadline.min(std::time::Instant::now() + QUEUE_WAIT),
        }
    }
    pub(crate) fn after_preparation(
        deadline: std::time::Instant,
        cancelled: tokio_util::sync::CancellationToken,
    ) -> Self {
        let mut ticket = Self::new(deadline);
        ticket.preparation_cancelled = Some(cancelled);
        ticket
    }
    fn start(&self) -> bool {
        if std::time::Instant::now() >= self.deadline
            || self
                .preparation_cancelled
                .as_ref()
                .is_some_and(|token| token.is_cancelled())
        {
            self.cancel();
        }
        self.state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    pub(crate) fn cancel(&self) {
        let _ = self
            .state
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
    }
}
pub(crate) struct CancelPreparation(pub(crate) tokio_util::sync::CancellationToken);
impl Drop for CancelPreparation {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

type InternalWork = Box<dyn FnOnce(&CoreState) + Send>;
enum Message {
    Command {
        op: OperationId,
        command: Box<CoreCommand>,
        maintenance: Option<Box<crate::maintenance::PreparedMaintenance>>,
        replacement: Option<Box<crate::maintenance::replacement::PreparedReplacement>>,
        backend_choice: Option<Box<crate::server::backend_choice::PreparedBackendChoice>>,
        reply: AdmissionReply,
        ticket: AdmissionTicket,
    },
    Internal(InternalWork),
    Stop,
}

#[derive(Default)]
pub(crate) struct CommandEngine {
    pub(crate) store: Mutex<crate::server::ConfigStore>,
    sender: OnceLock<mpsc::SyncSender<Message>>,
    pub(crate) admission: Mutex<()>,
    closed: AtomicBool,
    pub(crate) stopping: AtomicBool,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl CommandEngine {
    pub(crate) fn with_store(store: crate::server::ConfigStore) -> Self {
        Self {
            store: Mutex::new(store),
            ..Default::default()
        }
    }
    pub(crate) fn start(&self, state: Weak<CoreState>) {
        let (sender, receiver) = mpsc::sync_channel(16);
        assert!(self.sender.set(sender).is_ok(), "engine initialized once");
        let thread = std::thread::Builder::new()
            .name("core-command-writer".into())
            .spawn(move || {
                loop {
                    let message = receiver.recv_timeout(std::time::Duration::from_secs(1));
                    let Some(state) = state.upgrade() else { break };
                    match message {
                        Ok(Message::Command {
                            op,
                            command,
                            maintenance,
                            replacement,
                            backend_choice,
                            reply,
                            ticket,
                        }) => {
                            let result = if ticket.start() {
                                state.execute_command_prepared(
                                    op,
                                    *command,
                                    Some(reply.clone()),
                                    maintenance.map(|p| *p),
                                    replacement.map(|p| *p),
                                    backend_choice.map(|p| *p),
                                )
                            } else {
                                Err(SubmitError::Busy)
                            };
                            // Sent only if admission did not already acknowledge; the
                            // waiter drops after its first response. Execution survives.
                            let _ = reply.send(result);
                        }
                        Ok(Message::Internal(work)) => work(&state),
                        Ok(Message::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    state.expire_debug_leases();
                }
            })
            .expect("start command engine");
        *self.join.lock().expect("engine join") = Some(thread);
    }
    fn enqueue(&self, message: Message, internal_release: bool) -> Result<(), SubmitError> {
        if self.closed.load(Ordering::SeqCst)
            || (self.stopping.load(Ordering::SeqCst) && !internal_release)
        {
            return Err(SubmitError::Unavailable);
        }
        let sender = self.sender.get().ok_or(SubmitError::Unavailable)?;
        if internal_release {
            sender.send(message).map_err(|_| SubmitError::Unavailable)
        } else {
            sender.try_send(message).map_err(|error| match error {
                mpsc::TrySendError::Full(_) => SubmitError::Busy,
                mpsc::TrySendError::Disconnected(_) => SubmitError::Unavailable,
            })
        }
    }
    fn gate(&self, bounded: bool) -> Result<std::sync::MutexGuard<'_, ()>, SubmitError> {
        if bounded {
            self.admission.try_lock().map_err(|error| match error {
                std::sync::TryLockError::WouldBlock => SubmitError::Busy,
                std::sync::TryLockError::Poisoned(_) => SubmitError::Unavailable,
            })
        } else {
            self.admission.lock().map_err(|_| SubmitError::Unavailable)
        }
    }
    pub(crate) fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&CoreState) -> T + Send + 'static,
        internal_release: bool,
    ) -> Result<T, SubmitError> {
        let (sender, receiver) = mpsc::channel();
        let ticket = AdmissionTicket::new(std::time::Instant::now() + QUEUE_WAIT);
        let work_ticket = ticket.clone();
        let bounded = RPC_QUEUE.get() && !internal_release;
        {
            let _gate = self.gate(bounded)?;
            self.enqueue(
                Message::Internal(Box::new(move |state| {
                    let result = if bounded && !work_ticket.start() {
                        Err(SubmitError::Busy)
                    } else {
                        Ok(work(state))
                    };
                    let _ = sender.send(result);
                })),
                internal_release,
            )?;
        }
        if !bounded {
            return receiver.recv().map_err(|_| SubmitError::Unavailable)?;
        }
        match receiver.recv_timeout(
            ticket
                .deadline
                .saturating_duration_since(std::time::Instant::now()),
        ) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(SubmitError::Unavailable),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                ticket.cancel();
                if ticket.state.load(Ordering::SeqCst) == 2 {
                    Err(SubmitError::Busy)
                } else {
                    receiver.recv().map_err(|_| SubmitError::Unavailable)?
                }
            }
        }
    }
    pub(crate) fn submit(
        &self,
        op: OperationId,
        command: CoreCommand,
        ticket: AdmissionTicket,
        maintenance: Option<crate::maintenance::PreparedMaintenance>,
        replacement: Option<crate::maintenance::replacement::PreparedReplacement>,
        backend_choice: Option<crate::server::backend_choice::PreparedBackendChoice>,
    ) -> Result<Accepted, SubmitError> {
        let (reply, receiver) = mpsc::channel();
        {
            let _gate = self.gate(true)?;
            self.enqueue(
                Message::Command {
                    op,
                    command: Box::new(command),
                    maintenance: maintenance.map(Box::new),
                    replacement: replacement.map(Box::new),
                    backend_choice: backend_choice.map(Box::new),
                    reply,
                    ticket: ticket.clone(),
                },
                false,
            )?;
        }
        match receiver.recv_timeout(
            ticket
                .deadline
                .saturating_duration_since(std::time::Instant::now()),
        ) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(SubmitError::Unavailable),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                ticket.cancel();
                if ticket.state.load(Ordering::SeqCst) == 2 {
                    Err(SubmitError::Busy)
                } else {
                    receiver.recv().map_err(|_| SubmitError::Unavailable)?
                }
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn enqueue_test(&self, work: InternalWork) -> Result<(), SubmitError> {
        let _gate = self.admission.lock().unwrap();
        self.enqueue(Message::Internal(work), false)
    }
    #[cfg(test)]
    pub(crate) fn is_joined(&self) -> bool {
        self.join.lock().unwrap().is_none()
    }

    pub(crate) fn close(&self) {
        {
            let _gate = self.admission.lock().expect("engine admission");
            self.closed.store(true, Ordering::SeqCst);
            if let Some(sender) = self.sender.get() {
                let _ = sender.send(Message::Stop);
            }
        }
        if let Some(thread) = self.join.lock().expect("engine join").take() {
            let _ = thread.join();
        }
    }
}
