fn main() -> anyhow::Result<()> {
    lg::app::trace_event("PROCESS", "main enter");
    if std::env::args().nth(1).as_deref() == Some("review") {
        lg::app::trace_event("PROCESS", "review mode enter");
        print!("{}", lg::git::assisted_review_against_main()?);
        lg::app::trace_event("PROCESS", "review mode exit ok");
        return Ok(());
    }
    let result = lg::app::App::new().and_then(|mut app| app.run());
    if let Err(err) = &result {
        lg::app::trace_event("ERROR", format!("{err:#}"));
    } else {
        lg::app::trace_event("PROCESS", "main exit ok");
    }
    result
}
