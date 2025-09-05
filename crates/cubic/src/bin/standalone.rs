use cubic::{client, common};

fn main() {
    common::runtime::start(|handle| {
        client::start(handle);
    });
}
