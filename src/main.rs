fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("review") {
        print!("{}", lg::git::assisted_review_against_main()?);
        return Ok(());
    }
    let result = lg::app::App::new().and_then(|mut app| app.run());
    if let Err(err) = &result {
        lg::app::trace_event("ERROR", format!("{err:#}"));
    }
    result
}
