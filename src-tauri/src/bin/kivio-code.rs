//! Kivio Code — Rust terminal coding agent (scaffold; see .trellis/tasks/06-16-kivio-code/prd.md).
fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version") | Some("-V") => println!("kivio-code {}", env!("CARGO_PKG_VERSION")),
        Some("--help") | Some("-h") => println!(
            "kivio-code {}\n\nUsage: kivio-code [--version] [--help]\n(CLI implementation in progress — see Trellis task 06-16-kivio-code)",
            env!("CARGO_PKG_VERSION")
        ),
        _ => {
            eprintln!("kivio-code: not yet implemented. Try --version or --help.");
            std::process::exit(2);
        }
    }
}
