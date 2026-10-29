//! Single command owner. RPC waits for durable admission, internal maintenance
//! waits for completion; neither waiter owns or cancels accepted execution.
use crate::server::CoreState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, Weak, mpsc};
use vpnhide_core_rpc::{Accepted, CoreCommand, OperationId, SubmitError};

pub(crate) type AdmissionReply = mpsc::Sender<Result<Accepted, SubmitError>>;
type InternalWork = Box<dyn FnOnce(&CoreState) + Send>;
enum Message {
    Command {
        op: OperationId,
        command: Box<CoreCommand>,
        reply: AdmissionReply,
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
                        Ok(Message::Command { op, command, reply }) => {
                            let result =
                                state.execute_command_admitted(op, *command, Some(reply.clone()));
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
    pub(crate) fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&CoreState) -> T + Send + 'static,
        internal_release: bool,
    ) -> Result<T, SubmitError> {
        let (sender, receiver) = mpsc::channel();
        {
            let _gate = self.admission.lock().expect("engine admission");
            self.enqueue(
                Message::Internal(Box::new(move |state| {
                    let _ = sender.send(work(state));
                })),
                internal_release,
            )?;
        }
        receiver.recv().map_err(|_| SubmitError::Unavailable)
    }
    pub(crate) fn submit(
        &self,
        op: OperationId,
        command: CoreCommand,
    ) -> Result<Accepted, SubmitError> {
        let (reply, receiver) = mpsc::channel();
        {
            let _gate = self.admission.lock().expect("engine admission");
            self.enqueue(
                Message::Command {
                    op,
                    command: Box::new(command),
                    reply,
                },
                false,
            )?;
        }
        receiver.recv().map_err(|_| SubmitError::Unavailable)?
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
