//! Compatibility entry point for reproducible research commands.
#[path = "../src/search.rs"]
mod search;
fn main() {
    if let Err(error) = search::run(&std::env::args().collect::<Vec<_>>()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
