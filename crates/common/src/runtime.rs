use std::{
    collections::HashSet,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use tokio::sync::mpsc::{
    Sender, UnboundedReceiver, UnboundedSender, WeakUnboundedSender, channel, unbounded_channel,
};

type SyncTask = Box<dyn Fn() + Send + Sync>;
type AsyncTask = Pin<Box<dyn Future<Output = ()> + Send>>;
type TaskId = u32;

enum WorkerMessage {
    LaunchTaskSync(SyncTask, bool),
    LaunchTaskAsync(AsyncTask),
    TaskComplete(TaskId),
}
enum MainMessage {
    RunTaskSync(SyncTask, TaskId),
    Terminate,
}

pub struct Handle {
    worker_tx: UnboundedSender<WorkerMessage>,
    terminate_flag: Arc<AtomicBool>,
}

impl Handle {
    pub fn is_terminating(&self) -> bool {
        self.terminate_flag.load(Ordering::SeqCst)
    }

    pub fn launch_task_sync<F>(&self, func: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.send_worker_message(WorkerMessage::LaunchTaskSync(Box::new(func), true));
    }

    pub fn launch_task_async<F, Fut>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.send_worker_message(WorkerMessage::LaunchTaskAsync(Box::pin(future)));
    }

    fn send_worker_message(&self, msg: WorkerMessage) {
        self.worker_tx.send(msg).unwrap()
    }
}

struct AsyncWorker {
    worker_rx: UnboundedReceiver<WorkerMessage>,
    worker_weak_tx: WeakUnboundedSender<WorkerMessage>,
    main_tx: Sender<MainMessage>,
    active_tasks: HashSet<TaskId>,
    next_task_id: TaskId,
    current_main_task_id: Option<TaskId>,
}

impl AsyncWorker {
    fn new(
        worker_rx: UnboundedReceiver<WorkerMessage>,
        worker_weak_tx: WeakUnboundedSender<WorkerMessage>,
        main_tx: Sender<MainMessage>,
    ) -> Self {
        Self {
            worker_rx,
            worker_weak_tx,
            main_tx,
            active_tasks: HashSet::new(),
            next_task_id: 0,
            current_main_task_id: None,
        }
    }

    async fn run(mut self) {
        while let Some(msg) = self.worker_rx.recv().await {
            self.process_message(msg).await;
        }

        self.main_tx.send(MainMessage::Terminate).await.unwrap();
        println!("worker terminated")
    }

    async fn process_message(&mut self, msg: WorkerMessage) {
        match msg {
            WorkerMessage::LaunchTaskSync(task, main_required) => {
                let task_id = self.create_task();
                if main_required {
                    let tx_permit = self.main_tx.try_reserve().unwrap();
                    if self.current_main_task_id.is_some() {
                        panic!("another task is already running on main")
                    }
                    tx_permit.send(MainMessage::RunTaskSync(task, task_id));
                } else {
                    let worker_tx = self.worker_weak_tx.upgrade().unwrap();
                    thread::spawn(move || {
                        task();
                        worker_tx
                            .send(WorkerMessage::TaskComplete(task_id))
                            .unwrap()
                    });
                }
            }
            WorkerMessage::LaunchTaskAsync(task) => {
                tokio::spawn(task);
            }
            WorkerMessage::TaskComplete(task_id) => {
                if !self.active_tasks.remove(&task_id) {
                    panic!("got TaskComplete for non-existant task ID: {}", task_id)
                }
                self.current_main_task_id
                    .take_if(|main_task_id| *main_task_id == task_id);
            }
        }

        if self.active_tasks.is_empty() && !self.worker_rx.is_closed() {
            self.worker_rx.close();
        }
    }

    fn create_task(&mut self) -> TaskId {
        let new_task_id = self.next_task_id;
        self.next_task_id += 1;
        self.active_tasks.insert(new_task_id);
        new_task_id
    }
}

pub fn start<F>(on_initialize: F)
where
    F: FnOnce(Handle),
{
    let (worker_tx, worker_rx) = unbounded_channel::<WorkerMessage>();
    let (main_tx, mut main_rx) = channel::<MainMessage>(1);
    let terminate_flag = Arc::new(AtomicBool::new(false));

    let rt = Handle {
        worker_tx: worker_tx.clone(),
        terminate_flag,
    };

    on_initialize(rt);

    let weak_worker_tx = worker_tx.clone().downgrade();
    let rt_async_worker = AsyncWorker::new(worker_rx, weak_worker_tx, main_tx);

    thread::scope(|scope| {
        scope.spawn(|| {
            let tokio_rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            tokio_rt.block_on(rt_async_worker.run());
        });

        while let Some(msg) = main_rx.blocking_recv() {
            match msg {
                MainMessage::RunTaskSync(func, task_id) => {
                    func();
                    worker_tx
                        .send(WorkerMessage::TaskComplete(task_id))
                        .unwrap();
                }
                MainMessage::Terminate => return,
            }
        }
    });
}
