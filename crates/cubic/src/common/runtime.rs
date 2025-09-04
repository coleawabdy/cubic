use crate::common::primitive::NativeSend;

#[cfg(not(target_arch = "wasm32"))]
pub type Handle = native::Handle;
#[cfg(target_arch = "wasm32")]
pub type Handle = web::Handle;

#[cfg(not(target_arch = "wasm32"))]
pub type TaskHandle<T> = native::TaskHandle<T>;
#[cfg(target_arch = "wasm32")]
pub type TaskHandle<T> = web::TaskHandle<T>;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Complete,
    Error,
}

pub trait HandleInterface: Clone {
    fn stop(&self);
    fn should_stop(&self) -> bool;
    fn stopped(&self) -> impl Future<Output = ()>;

    fn launch_task_async<Fn, Fut, T>(&self, future: Fn) -> TaskHandle<T>
    where
        Fn: FnOnce(Self) -> Fut + 'static,
        T: NativeSend + 'static,
        Fut: Future<Output = T> + NativeSend + 'static;
}

pub trait TaskHandleInterface<T> {
    fn status(&self) -> TaskStatus;
    fn join(self) -> T;
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{cell::RefCell, thread, time::Duration};

    use tokio::{
        signal,
        sync::{
            mpsc::{UnboundedSender, unbounded_channel},
            oneshot::{self, error::TryRecvError},
        },
        task::JoinHandle,
    };
    use tokio_util::sync::CancellationToken;

    use super::*;

    enum InterruptKind {
        Timeout,
        Cancel(bool), // Was cancel external?
    }

    enum TaskHandleState<T> {
        Pending(oneshot::Receiver<T>),
        Complete(T),
        Error,
    }

    #[derive(Clone)]
    pub struct Handle {
        tokio_rt_handle: tokio::runtime::Handle,
        stop_token: CancellationToken,
        async_handle_tx: UnboundedSender<JoinHandle<()>>,
    }

    impl HandleInterface for Handle {
        fn should_stop(&self) -> bool {
            self.stop_token.is_cancelled()
        }

        fn stop(&self) {
            self.stop_token.cancel();
        }

        fn stopped(&self) -> impl Future<Output = ()> {
            self.stop_token.cancelled()
        }

        fn launch_task_async<Fn, Fut, T>(&self, func: Fn) -> TaskHandle<T>
        where
            Fn: FnOnce(Handle) -> Fut + 'static,
            Fut: Future<Output = T> + Send + 'static,
            T: Send + 'static,
        {
            let (tx, rx) = oneshot::channel::<T>();
            let handle = self.clone();

            let future = func(handle);
            self.async_handle_tx
                .send(self.tokio_rt_handle.spawn(async move {
                    tx.send(future.await).ok();
                }))
                .unwrap();

            TaskHandle::new(rx)
        }
    }

    pub struct TaskHandle<T> {
        state: RefCell<Option<TaskHandleState<T>>>,
    }

    impl<T> TaskHandle<T> {
        fn new(rx: oneshot::Receiver<T>) -> Self {
            Self {
                state: RefCell::new(Some(TaskHandleState::Pending(rx))),
            }
        }
    }

    impl<T> TaskHandleInterface<T> for TaskHandle<T> {
        fn status(&self) -> TaskStatus {
            let mut state_cell = self.state.borrow_mut();
            let mut state = state_cell.take().unwrap();
            state = match state {
                TaskHandleState::Pending(mut rx) => match rx.try_recv() {
                    Ok(result) => TaskHandleState::Complete(result),
                    Err(TryRecvError::Empty) => TaskHandleState::Pending(rx),
                    Err(TryRecvError::Closed) => TaskHandleState::Error,
                },
                _ => state,
            };

            let status = match state {
                TaskHandleState::Pending(_) => TaskStatus::Pending,
                TaskHandleState::Complete(_) => TaskStatus::Complete,
                TaskHandleState::Error => TaskStatus::Error,
            };

            *state_cell = Some(state);
            status
        }

        fn join(self) -> T {
            let mut state_cell = self.state.borrow_mut();
            let state = state_cell.take().unwrap();
            match state {
                TaskHandleState::Pending(rx) => match rx.blocking_recv() {
                    Ok(result) => result,
                    Err(_) => panic!("underlying task panicked"),
                },
                TaskHandleState::Complete(result) => result,
                TaskHandleState::Error => panic!("underlying task panicked"),
            }
        }
    }

    pub fn start<T>(task: T)
    where
        T: FnOnce(Handle),
    {
        thread::scope(|s| {
            let (handle_tx, mut async_handle_rx) = unbounded_channel::<JoinHandle<()>>();
            let (tx, rx) = oneshot::channel::<tokio::runtime::Handle>();
            let stop_token = tokio_util::sync::CancellationToken::new();

            let async_stop_token = stop_token.clone();
            let async_handle_tx = handle_tx.clone();
            s.spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();

                tx.send(rt.handle().clone()).unwrap();

                rt.block_on(async move {
                    let mut ctrl_c = Box::pin(async {
                        signal::ctrl_c()
                            .await
                            .expect("failed to install Ctrl+C handler");
                    });

                    #[cfg(unix)]
                    let mut terminate = Box::pin(async {
                        let mut signal = signal::unix::signal(signal::unix::SignalKind::terminate())
                            .expect("failed to install signal handler");
                        signal.recv().await;
                    });

                    loop {
                        #[cfg(not(unix))]
                        let interrupt_kind = tokio::select! {
                            _ = &mut ctrl_c => InterruptKind::Cancel(true),
                            _ = async_stop_token.cancelled() => InterruptKind::Cancel(false),
                            _ = tokio::time::sleep(Duration::from_secs(1)) => InterruptKind::Timeout,
                        };

                        #[cfg(unix)]
                        let interrupt_kind = tokio::select! {
                            _ = &mut ctrl_c => InterruptKind::Cancel(true),
                            _ = &mut terminate => InterruptKind::Cancel(true),
                            _ = async_stop_token.cancelled() => InterruptKind::Cancel(false),
                            _ = tokio::time::sleep(Duration::from_secs(1)) => InterruptKind::Timeout,
                        };

                        match interrupt_kind {
                            InterruptKind::Cancel(true) => {
                                async_stop_token.cancel();
                                break;
                            }
                            InterruptKind::Cancel(false) => break,
                            InterruptKind::Timeout => (),
                        }

                        let task_count = async_handle_rx.len();
                        let mut tasks = Vec::<JoinHandle<()>>::new();
                        async_handle_rx.recv_many(&mut tasks, task_count).await;
                        for task in tasks {
                            if !task.is_finished() {
                                async_handle_tx.send(task).unwrap();
                            }
                        }
                    }

                    async_handle_rx.close();
                    while let Some(task) = async_handle_rx.recv().await {
                        task.await.ok();
                    }
                });
            });

            let tokio_rt_handle = rx.blocking_recv().unwrap();
            let handle = Handle {
                tokio_rt_handle,
                stop_token,
                async_handle_tx: handle_tx,
            };

            task(handle);
        })
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;

    #[derive(Clone)]
    pub struct Handle {}

    impl HandleInterface for Handle {
        fn stop(&self) {
            todo!()
        }
        fn should_stop(&self) -> bool {
            todo!()
        }
        async fn stopped(&self) -> () {
            todo!()
        }

        fn launch_task_async<Fn, Fut, T>(&self, func: Fn) -> TaskHandle<T>
        where
            Fn: FnOnce(Self) -> Fut + 'static,
            T: 'static,
            Fut: Future<Output = T> + 'static,
        {
            let task_handle = self.clone();
            let fut = func(task_handle);
            wasm_bindgen_futures::spawn_local(async move {
                fut.await;
            });
            todo!()
        }
    }

    pub struct TaskHandle<T> {
        _internal: T,
    }

    impl<T> TaskHandleInterface<T> for TaskHandle<T> {
        fn status(&self) -> TaskStatus {
            todo!()
        }

        fn join(self) -> T {
            todo!()
        }
    }
}

pub fn start<T>(_task: T)
where
    T: FnOnce(Handle),
{
    #[cfg(not(target_arch = "wasm32"))]
    native::start(_task)
}
