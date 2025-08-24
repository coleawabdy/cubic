use common::runtime;

fn main() {
    runtime::start(|rt| {
        rt.launch_task_sync(|| println!("Hello, World!"));
    });
}
