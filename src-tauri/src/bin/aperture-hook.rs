//! Optional observer helper. `config PROVIDER` prints a manual-merge snippet;
//! `collect PROVIDER` is fail-open, silent, and never modifies provider settings.
use aperture_lib::observer::hook_bridge;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    match (
        args.get(1).map(String::as_str),
        args.get(2).map(String::as_str),
    ) {
        (Some("config"), Some(provider)) => {
            let result =
                std::env::current_exe().and_then(|exe| hook_bridge::config(provider, &exe));
            match result {
                Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
        }
        (Some("collect"), Some(provider)) => {
            // Nothing below has a deadline of its own: stdin may never close and
            // the inbox write may hang. Leave silently once the budget is spent.
            std::thread::spawn(|| {
                std::thread::sleep(hook_bridge::COLLECT_BUDGET);
                std::process::exit(0);
            });
            if let Some(root) = hook_bridge::inbox() {
                let _ = hook_bridge::collect(provider, std::io::stdin().lock(), &root);
            }
            // No decision, context, stdout, stderr, or nonzero exit code.
        }
        _ => {
            eprintln!("Usage: aperture-hook config|collect claude_code|codex");
            std::process::exit(1);
        }
    }
}
