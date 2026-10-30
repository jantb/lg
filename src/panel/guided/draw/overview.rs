//! The overview step: what the change is, and the plan for walking it.

use super::*;

pub(super) fn draw_overview(state: &AppState, guided: &Guided, area: Rect, frame: &mut Frame) {
    let mut lines = vec![Line::from(Span::styled(
        "What this change is",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];
    match (guided.plan(), guided.planning()) {
        (Some(plan), _) => lines.extend(plan_lines(&plan, guided.walk())),
        (None, Some(planning)) => {
            let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
            lines.push(Line::from(Span::styled(
                format!(
                    "  {spinner} planning the walk: {}",
                    progress_text(planning, 24)
                ),
                Style::default().fg(Color::Cyan),
            )));
        }
        (None, None) => lines.extend(commentary_lines(
            state,
            guided.commentary.get(super::OVERVIEW_KEY),
            area.width,
        )),
    }
    lines.push(Line::from(""));
    for text in guided.overview.lines() {
        lines.push(Line::from(Span::styled(text.to_string(), LABEL)));
    }
    lines.extend(exchange_lines(
        state,
        guided,
        super::OVERVIEW_KEY,
        area.width,
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("\u{2192}", KEY),
        Span::raw(" starts the walk at the first file"),
    ]));
    let max = wrapped_rows(&lines, area.width).saturating_sub(area.height as usize) as u16;
    guided.side_max.set(max);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((guided.side_scroll.min(max), 0)),
        area,
    );
}

/// The plan the walk modes order the files by, as the overview shows it.
fn plan_lines(plan: &Plan, walk: WalkMode) -> Vec<Line<'static>> {
    let heading = |text: &str| {
        Line::from(Span::styled(
            text.to_string(),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let mut lines = Vec::new();
    if let Some(summary) = &plan.summary {
        lines.push(Line::from(Span::styled(
            format!("  {summary}"),
            Style::default().fg(Color::White),
        )));
    }
    lines.push(Line::from(""));
    lines.push(heading("Concerns"));
    for (at, concern) in plan.concerns.iter().enumerate() {
        lines.push(Line::from(Span::styled(
            format!("  {}. {}", at + 1, concern.title),
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        )));
        if let Some(why) = &concern.why {
            lines.push(Line::from(Span::raw(format!("     {why}"))));
        }
        lines.push(Line::from(Span::styled(
            format!("     {}", concern.paths.join(", ")),
            LABEL,
        )));
    }
    if !plan.watch.is_empty() {
        lines.push(Line::from(""));
        lines.push(heading("Worth your attention"));
        for watch in &plan.watch {
            lines.push(Line::from(vec![
                Span::styled("  \u{2192} ", Style::default().fg(Color::Cyan)),
                Span::raw(watch.clone()),
            ]));
        }
    }
    let mut risky: Vec<(&String, &(Risk, String))> = plan
        .risk
        .iter()
        .filter(|(_, (risk, _))| *risk != Risk::Low)
        .collect();
    risky.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    if !risky.is_empty() {
        lines.push(Line::from(""));
        lines.push(heading("Risk"));
        for (path, (risk, why)) in risky {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{25cf} {} ", risk.word()),
                    Style::default().fg(risk_color(*risk)),
                ),
                Span::styled(path.clone(), Style::default().fg(Color::White)),
                Span::styled(format!(" \u{2014} {why}"), LABEL),
            ]));
        }
    }
    if !plan.skim.is_empty() {
        let mut skim: Vec<(&String, &String)> = plan.skim.iter().collect();
        skim.sort();
        lines.push(Line::from(""));
        lines.push(heading("Safe to skim (S marks them reviewed)"));
        for (path, why) in skim {
            lines.push(Line::from(Span::styled(
                format!("  {path} \u{2014} {why}"),
                LABEL,
            )));
        }
    }
    for rest in plan.rest.iter().filter(|rest| !rest.trim().is_empty()) {
        lines.push(Line::from(Span::styled(
            format!("  {}", rest.trim()),
            LABEL,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(format!("Walk by {}", walk.label().to_lowercase()), KEY),
        Span::styled(
            format!(": {} \u{b7} M or the badge switches", walk.describe()),
            LABEL,
        ),
    ]));
    lines
}
