//! The terrarium ASCII banner printed at the start of a run.
//!
//! It doubles as a reminder of which MCP tools the session has, so it lists the
//! preset's build tools inside the glass. A sheen on the walls means the domain
//! proxy is filtering outbound traffic.

/// The build tools a preset contributes to the banner listing.
fn tools_for_preset(name: &str) -> Vec<&'static str> {
    match name {
        "kotlin" => vec![
            "gradle_check",
            "gradle_test",
            "gradle_build",
            "gradle_fmt_check",
        ],
        "swift" => vec!["swift_build", "swift_test", "swift_fmt_check"],
        "none" => vec![],
        _ => vec![
            "cargo_check",
            "cargo_test",
            "cargo_build",
            "cargo_clippy",
            "cargo_fmt",
            "cargo_update",
            " _breaking",
        ],
    }
}

/// Tools shown for every preset.
const COMMON_TOOLS: &[&str] = &["git_status", "git_log", "git_diff", "report_violation"];

/// Prints the banner to stderr, so it never mixes into piped stdout.
pub(super) fn print_banner(preset: &str, proxy_enabled: bool) {
    let glass = "\x1b[38;2;144;238;144m";
    let sheen = "\x1b[1;38;2;220;255;220m"; // bright white-green sparkle
    let frame = "\x1b[38;2;60;140;60m";
    let title = "\x1b[1;38;2;100;255;150m";
    let plant = "\x1b[38;2;34;180;34m";
    let soil = "\x1b[38;2;80;120;50m";
    let cmd = "\x1b[38;2;180;255;180m";
    let reset = "\x1b[0m";

    // A workspace preset is several presets joined with `+`.
    let mut preset_tools: Vec<&str> = Vec::new();
    for part in preset.split('+') {
        for tool in tools_for_preset(part) {
            if !preset_tools.contains(&tool) {
                preset_tools.push(tool);
            }
        }
    }
    let tools: Vec<&str> = preset_tools
        .iter()
        .chain(COMMON_TOOLS.iter())
        .copied()
        .collect();

    let inner_w = tools.iter().map(|t| t.len()).max().unwrap_or(0) + 2;
    let bar = "─".repeat(inner_w);
    let proxy_label = if proxy_enabled { "proxy" } else { "no proxy" };
    let subtitle = format!("{preset} · {proxy_label}");

    // When proxy is enabled, glass walls get a sheen (sparkle characters)
    let (lwall, rwall) = if proxy_enabled {
        (format!("{sheen}✧{glass}"), format!("{sheen}✧{glass}"))
    } else {
        (format!("{glass}|"), format!("{glass}|"))
    };

    eprintln!(
        "\n\
        {glass}            ___________________\n\
        {glass}           /                   \\\n\
        {title}          /  t e r r a r i u m  \\\n\
        {frame}         /  {subtitle:<19}  \\\n\
        {glass}        {lwall}  {frame}┌{bar}┐{glass}  {rwall}",
        subtitle = subtitle,
        bar = bar,
        lwall = lwall,
        rwall = rwall
    );
    for (i, t) in tools.iter().enumerate() {
        // Alternate sheen on left/right walls for a glint effect
        let (l, r) = if proxy_enabled && i % 3 == 0 {
            (format!("{sheen}✧{glass}"), format!("{sheen}✧{glass}"))
        } else {
            (lwall.clone(), rwall.clone())
        };
        eprintln!(
            "        {glass}{l}  {frame}│{cmd} {t:<width$}{frame}│{glass}  {r}",
            l = l,
            r = r,
            t = t,
            width = inner_w - 1
        );
    }
    eprintln!(
        "        {glass}{lwall}  {frame}└{bar}┘{glass}  {rwall}\n\
        {plant}        {lwall}  \\|/  \\|/  \\|/  \\|/    {glass}{rwall}\n\
        {plant}        {lwall}  .|.  .|.  .|.  .|.    {glass}{rwall}\n\
        {soil}        |.:.:.:.:.:.:.:.:.:.:.:.:|{glass}\n\
        {glass}        \\________________________/{reset}\n",
        bar = bar,
        lwall = lwall,
        rwall = rwall
    );
}
