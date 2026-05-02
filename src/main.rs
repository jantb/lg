fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("review") {
        print!("{}", lg::git::assisted_review_against_main()?);
        return Ok(());
    }
    lg::app::App::new()?.run()
}
