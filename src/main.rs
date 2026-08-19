const USAGE: &str = "\
lg — a terminal UI for git

Usage:
  lg              Start the interactive TUI in the current repository
  lg review       Print an assisted review of this branch against main, then exit
  lg --help       Show this message
  lg --version    Show the version
";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => lg::app::App::new()?.run(),
        Some("review") if args.len() == 1 => {
            print!("{}", lg::git::assisted_review_against_main()?);
            Ok(())
        }
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version" | "version") => {
            println!("lg {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => {
            // Silently ignoring an argument makes a typo look like it worked.
            eprint!("lg: unrecognized argument '{other}'\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}
