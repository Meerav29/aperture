//! Diagnostic reader using the same adapters and reducer as the desktop.
//! Emits summaries only. Does not launch or communicate with either provider.
use aperture_lib::observer::{passive::Observer, state::Store};
fn main() {
    let polls: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mut observer = Observer::default();
    let mut store = Store::default();
    for i in 0..polls {
        observer.poll(&mut store);
        println!("{}", serde_json::to_string(&store.snapshot()).unwrap());
        if i + 1 < polls {
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
}
