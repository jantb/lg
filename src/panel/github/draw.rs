//! Drawing the GitHub modal: a tab bar, the list beside the selected entry's
//! details, and a bottom line that is whatever the modal is asking for.

use ratatui::{
    Frame,
    layout::{Constraint, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use super::{
    Compose, CreateField, CreateForm, MergeForm, MergeRow, Mode, Tab, clone_destination,
    clone_request, is_current_branch, selected_pr, visible_repos,
};
use crate::github::{Mergeable, PrState, PullRequest, ReviewDecision};
use crate::state::{AppState, SPINNER_FRAMES};
use crate::ui::{self, palette};

const LABEL: Style = Style::new().fg(Color::DarkGray);
const HINT_KEY: Style = Style::new().fg(Color::Yellow);

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    // Tall enough to read a description, and clear of the footer, where the
    // status line reports what the last action did.
    let modal = ui::centered(
        area,
        area.width.saturating_sub(4).min(140),
        area.height.saturating_sub(2).min(44),
    );
    let title = match &state.github.repo {
        Some(repo) => format!("GitHub \u{b7} {}", repo.name),
        None => "GitHub".to_string(),
    };
    let inner = ui::modal_frame(frame, modal, &title);
    if inner.width < 40 || inner.height < 8 {
        frame.render_widget(Paragraph::new("Terminal too small for GitHub"), inner);
        return;
    }
    let (rows, mut dividers) = ui::modal_rows(
        frame,
        inner,
        &[
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ],
    );
    draw_tabs(state, rows[0], frame);
    match (&state.github.mode, state.github.tab) {
        (Mode::Create(form), _) => draw_create(form, rows[1], frame),
        (_, Tab::PullRequests) => {
            let (panes, gaps) = ui::modal_columns(
                frame,
                rows[1],
                &[Constraint::Percentage(42), Constraint::Min(20)],
            );
            dividers.extend(gaps);
            draw_pr_list(state, panes[0], frame);
            match &state.github.mode {
                Mode::Merge(form) => draw_merge(state, form, panes[1], frame),
                _ => draw_pr_detail(state, panes[1], frame),
            }
        }
        (_, Tab::Repositories) => {
            let (panes, gaps) = ui::modal_columns(
                frame,
                rows[1],
                &[Constraint::Percentage(50), Constraint::Min(20)],
            );
            dividers.extend(gaps);
            draw_repo_list(state, panes[0], frame);
            draw_repo_detail(state, panes[1], frame);
        }
    }
    draw_bottom(state, rows[2], frame);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &dividers, frame);
    }
}

fn spinner(state: &AppState) -> &'static str {
    SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()]
}

fn draw_tabs(state: &AppState, area: Rect, frame: &mut Frame) {
    let tab = |label: &'static str, active: bool| {
        if active {
            Span::styled(
                format!(" {label} "),
                Style::default()
                    .fg(Color::Black)
                    .bg(palette::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                format!(" {label} "),
                Style::default().fg(palette::TEXT_IDLE),
            )
        }
    };
    let gh = &state.github;
    let mut spans = vec![
        tab("Pull requests", gh.tab == Tab::PullRequests),
        Span::raw(" "),
        tab("Repositories", gh.tab == Tab::Repositories),
        Span::raw("   "),
    ];
    match gh.tab {
        Tab::PullRequests => {
            spans.push(Span::styled("showing ", LABEL));
            spans.push(Span::styled(
                gh.filter.label(),
                Style::default().fg(Color::Cyan),
            ));
            spans.push(Span::styled("  (s changes)", LABEL));
            if gh.prs_loading.is_some() {
                spans.push(Span::styled(
                    format!("   {} reading pull requests", spinner(state)),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
        Tab::Repositories => {
            let used: usize = spans.iter().map(|span| span.width()).sum();
            spans.extend(owner_spans(
                state,
                (area.width as usize).saturating_sub(used),
            ));
            if gh.repos_loading.is_some() {
                spans.push(Span::styled(
                    format!("   {} reading repositories", spinner(state)),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Whose repositories are listed, among everyone whose could be: every owner
/// by name when they fit in `room`, otherwise the chosen one and where it
/// stands among them.
fn owner_spans(state: &AppState, room: usize) -> Vec<Span<'static>> {
    let gh = &state.github;
    let name = |owner: &Option<String>| match (owner, &gh.owners) {
        (Some(org), _) => org.clone(),
        (None, Some(owners)) if !owners.login.is_empty() => owners.login.clone(),
        (None, _) => "you".to_string(),
    };
    let chosen = Style::default().fg(Color::Cyan);
    let choices = gh.owner_choices();
    let mut spans = vec![Span::styled("owner ", LABEL)];
    if choices.len() < 2 {
        spans.push(Span::styled(name(&gh.owner), chosen));
        return spans;
    }
    let hint = "  (\u{2190}\u{2192} changes)";
    let all: usize = choices
        .iter()
        .map(|owner| name(owner).chars().count() + 2)
        .sum();
    if 6 + all + hint.chars().count() <= room {
        for owner in &choices {
            let style = if *owner == gh.owner {
                chosen.add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette::TEXT_IDLE)
            };
            spans.push(Span::styled(format!(" {} ", name(owner)), style));
        }
    } else {
        let at = choices
            .iter()
            .position(|owner| *owner == gh.owner)
            .unwrap_or(0);
        spans.push(Span::styled(name(&gh.owner), chosen));
        spans.push(Span::styled(
            format!(" {}/{}", at + 1, choices.len()),
            LABEL,
        ));
    }
    spans.push(Span::styled(hint, LABEL));
    spans
}

/// A message filling a pane in place of a list: loading, empty, or failed.
fn draw_placeholder(text: &str, error: bool, area: Rect, frame: &mut Frame) {
    let style = if error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(palette::TEXT_IDLE)
    };
    frame.render_widget(
        Paragraph::new(text.to_string())
            .style(style)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

fn checks_span(pr: &PullRequest) -> Option<Span<'static>> {
    let checks = &pr.checks;
    if checks.is_empty() {
        None
    } else if !checks.failed.is_empty() {
        Some(Span::styled("\u{2717}", Style::default().fg(Color::Red)))
    } else if checks.pending > 0 {
        Some(Span::styled("\u{25cc}", Style::default().fg(Color::Yellow)))
    } else {
        Some(Span::styled("\u{2713}", Style::default().fg(Color::Green)))
    }
}

fn review_style(review: ReviewDecision) -> Style {
    match review {
        ReviewDecision::Approved => Style::default().fg(Color::Green),
        ReviewDecision::ChangesRequested => Style::default().fg(Color::Red),
        ReviewDecision::ReviewRequired | ReviewDecision::None => Style::default().fg(Color::Yellow),
    }
}

fn state_style(state: PrState) -> Style {
    match state {
        PrState::Open => Style::default().fg(Color::Green),
        PrState::Merged => Style::default().fg(Color::Magenta),
        PrState::Closed => Style::default().fg(Color::Red),
    }
}

fn draw_pr_list(state: &AppState, area: Rect, frame: &mut Frame) {
    let gh = &state.github;
    if gh.prs.is_empty() {
        let (text, error) = match (&gh.prs_error, gh.prs_loading.is_some()) {
            (Some(err), _) => (err.clone(), true),
            (None, true) => ("reading pull requests\u{2026}".to_string(), false),
            (None, false) => (
                format!(
                    "no {} pull requests \u{2014} n opens one for this branch",
                    gh.filter.label()
                ),
                false,
            ),
        };
        draw_placeholder(&text, error, area, frame);
        return;
    }
    let width = area.width.saturating_sub(2) as usize;
    // Every number gets the room of the longest, so the titles line up.
    let digits = gh
        .prs
        .iter()
        .map(|pr| pr.number.to_string().len())
        .max()
        .unwrap_or(1);
    let items: Vec<ListItem> = gh
        .prs
        .iter()
        .map(|pr| {
            let here = is_current_branch(state, pr);
            let mut spans = vec![
                Span::styled(
                    if here { "\u{25cf} " } else { "  " },
                    Style::default().fg(palette::ACCENT),
                ),
                Span::styled(format!("#{:<digits$} ", pr.number), state_style(pr.state)),
            ];
            let mut marks: Vec<Span> = Vec::new();
            if let Some(checks) = checks_span(pr) {
                marks.push(Span::raw(" "));
                marks.push(checks);
            }
            if pr.review == ReviewDecision::Approved {
                marks.push(Span::styled(" \u{2714}", review_style(pr.review)));
            } else if pr.review == ReviewDecision::ChangesRequested {
                marks.push(Span::styled(" \u{b1}", review_style(pr.review)));
            }
            let used = 2
                + digits
                + 2
                + marks
                    .iter()
                    .map(|s| s.content.chars().count())
                    .sum::<usize>();
            let mut title = String::new();
            if pr.draft {
                title.push_str("draft \u{b7} ");
            }
            title.push_str(&pr.title);
            let room = width.saturating_sub(used + 1);
            spans.push(Span::styled(
                format!("{:<room$}", truncate(&title, room)),
                if pr.draft {
                    Style::default().fg(palette::TEXT_IDLE)
                } else {
                    Style::default().fg(Color::White)
                },
            ));
            spans.extend(marks);
            ListItem::new(Line::from(spans))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(palette::selection())
        .highlight_symbol("\u{203a}");
    let visible = area.height as usize;
    let offset = gh.pr_idx.saturating_sub(visible.saturating_sub(1));
    let mut list_state = crate::panel::scroll::list_state(Some(gh.pr_idx), offset);
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_pr_detail(state: &AppState, area: Rect, frame: &mut Frame) {
    let Some(pr) = selected_pr(state) else {
        return;
    };
    let width = area.width.saturating_sub(1);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(format!("#{} ", pr.number), state_style(pr.state)),
        Span::styled(
            pr.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let head = match &pr.head_owner {
        Some(owner) => format!("{owner}:{}", pr.head),
        None => pr.head.clone(),
    };
    lines.push(Line::from(vec![
        Span::styled(pr.author.clone(), Style::default().fg(Color::Cyan)),
        Span::styled(" wants to merge ", LABEL),
        Span::styled(head, Style::default().fg(Color::Yellow)),
        Span::styled(" \u{2192} ", LABEL),
        Span::styled(pr.base.clone(), Style::default().fg(Color::Yellow)),
    ]));

    let mut status = vec![Span::styled(pr.state.label(), state_style(pr.state))];
    if pr.draft {
        status.push(Span::styled(" \u{b7} draft", LABEL));
    }
    if let Some(review) = pr.review.label() {
        status.push(Span::styled(" \u{b7} ", LABEL));
        status.push(Span::styled(review, review_style(pr.review)));
    }
    if pr.is_open() {
        match pr.mergeable {
            Mergeable::Yes => status.push(Span::styled(
                " \u{b7} no conflicts",
                Style::default().fg(Color::Green),
            )),
            Mergeable::Conflicting => status.push(Span::styled(
                " \u{b7} has conflicts",
                Style::default().fg(Color::Red),
            )),
            Mergeable::Unknown => {}
        }
    }
    lines.push(Line::from(status));

    lines.push(Line::from(vec![
        Span::styled(
            format!("+{}", pr.additions),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("\u{2212}{}", pr.deletions),
            Style::default().fg(Color::Red),
        ),
        Span::styled(
            format!(
                " in {} file{}",
                pr.changed_files,
                if pr.changed_files == 1 { "" } else { "s" }
            ),
            LABEL,
        ),
        Span::styled(
            pr.updated_at
                .split('T')
                .next()
                .filter(|day| !day.is_empty())
                .map(|day| format!(" \u{b7} updated {day}"))
                .unwrap_or_default(),
            LABEL,
        ),
    ]));

    let checks = &pr.checks;
    if !checks.is_empty() {
        let mut spans = vec![Span::styled("checks ", LABEL)];
        if checks.passed > 0 {
            spans.push(Span::styled(
                format!("{} passed ", checks.passed),
                Style::default().fg(Color::Green),
            ));
        }
        if !checks.failed.is_empty() {
            spans.push(Span::styled(
                format!("{} failed ", checks.failed.len()),
                Style::default().fg(Color::Red),
            ));
        }
        if checks.pending > 0 {
            spans.push(Span::styled(
                format!("{} running", checks.pending),
                Style::default().fg(Color::Yellow),
            ));
        }
        lines.push(Line::from(spans));
        for name in checks.failed.iter().take(5) {
            lines.push(Line::from(Span::styled(
                format!("  \u{2717} {name}"),
                Style::default().fg(Color::Red),
            )));
        }
    }
    if !pr.reviews.is_empty() {
        let mut spans = vec![Span::styled("reviews ", LABEL)];
        for (i, review) in pr.reviews.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(", ", LABEL));
            }
            let (word, style) = match review.state.as_str() {
                "APPROVED" => ("approved", Style::default().fg(Color::Green)),
                "CHANGES_REQUESTED" => ("wants changes", Style::default().fg(Color::Red)),
                "COMMENTED" => ("commented", LABEL),
                "DISMISSED" => ("dismissed", LABEL),
                _ => ("pending", LABEL),
            };
            spans.push(Span::styled(
                review.author.clone(),
                Style::default().fg(Color::Cyan),
            ));
            spans.push(Span::styled(format!(" {word}"), style));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    if pr.body.trim().is_empty() {
        lines.push(Line::from(Span::styled("No description.", LABEL)));
    } else {
        lines.extend(crate::panel::markdown::render(&pr.body, "", width));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((state.github.detail_scroll, 0)),
        Rect {
            x: area.x + 1,
            width,
            ..area
        },
    );
}

fn toggle(on: bool) -> &'static str {
    if on { "[x]" } else { "[ ]" }
}

fn draw_merge(state: &AppState, form: &MergeForm, area: Rect, frame: &mut Frame) {
    let title = selected_pr(state)
        .map(|pr| pr.title.clone())
        .unwrap_or_default();
    let row_style = |row: MergeRow| {
        if form.row == row {
            palette::selection()
        } else {
            Style::default()
        }
    };
    let methods = form
        .methods
        .iter()
        .map(|method| method.label())
        .collect::<Vec<_>>()
        .join(", ");
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("Merge #{} ", form.number),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(title, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Method         ", LABEL),
            Span::styled(
                format!("\u{2039} {} \u{203a}", form.method.label()),
                row_style(MergeRow::Method).fg(Color::Cyan),
            ),
        ]),
        Line::from(Span::styled(
            format!("               allowed here: {methods}"),
            LABEL,
        )),
        Line::from(vec![
            Span::styled("Delete branch  ", LABEL),
            Span::styled(
                format!(
                    "{} local and remote, once merged",
                    toggle(form.delete_branch)
                ),
                row_style(MergeRow::DeleteBranch),
            ),
        ]),
        Line::from(vec![
            Span::styled("Auto-merge     ", LABEL),
            Span::styled(
                format!("{} wait for required checks and reviews", toggle(form.auto)),
                row_style(MergeRow::Auto),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(1),
            ..area
        },
    );
}

fn draw_create(form: &CreateForm, area: Rect, frame: &mut Frame) {
    let label = |field: CreateField, text: &'static str| {
        if form.field == field {
            Span::styled(
                text,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(text, LABEL)
        }
    };
    let value = |field: CreateField| {
        if form.field == field {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        }
    };
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    };
    let value_width = inner.width.saturating_sub(8) as usize;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("From    ", LABEL),
            Span::styled(form.branch.clone(), Style::default().fg(Color::Yellow)),
            Span::styled("  pushed to the remote first if it has to be", LABEL),
        ]),
        Line::from(vec![
            label(CreateField::Title, "Title   "),
            Span::styled(tail(&form.title, value_width), value(CreateField::Title)),
        ]),
        Line::from(vec![
            label(CreateField::Base, "Base    "),
            Span::styled(tail(&form.base, value_width), value(CreateField::Base)),
        ]),
        Line::from(vec![
            label(CreateField::Draft, "Draft   "),
            Span::styled(
                format!("{} open as a draft", toggle(form.draft)),
                value(CreateField::Draft),
            ),
        ]),
        Line::from(label(CreateField::Body, "Body")),
    ];
    let body_top = lines.len() as u16;
    let body_width = inner.width.saturating_sub(2).max(1) as usize;
    let body_rows = wrap_body(&form.body, body_width);
    let room = inner.height.saturating_sub(body_top) as usize;
    // Keep the end of the body in sight: that is where the typing happens.
    let skip = body_rows.len().saturating_sub(room);
    for row in body_rows.iter().skip(skip) {
        lines.push(Line::from(Span::styled(
            format!("  {row}"),
            value(CreateField::Body),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    let cursor = match form.field {
        CreateField::Title => Some((8 + form.title.chars().count().min(value_width), 1)),
        CreateField::Base => Some((8 + form.base.chars().count().min(value_width), 2)),
        CreateField::Body => {
            let last = body_rows.last().map(|row| row.chars().count()).unwrap_or(0);
            let row = body_rows.len().saturating_sub(1).saturating_sub(skip);
            Some((2 + last, body_top as usize + row))
        }
        CreateField::Draft => None,
    };
    if let Some((x, y)) = cursor
        && (y as u16) < inner.height
    {
        frame.set_cursor_position(Position::new(inner.x + x as u16, inner.y + y as u16));
    }
}

/// The body cut to rows of `width`, breaking only where it has to: the text
/// is shown as written, line breaks included.
fn wrap_body(body: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in body.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            rows.push(String::new());
            continue;
        }
        for chunk in chars.chunks(width) {
            rows.push(chunk.iter().collect());
        }
    }
    rows
}

/// Keep the end of a value in view when it is too long: that is where the
/// cursor is.
fn tail(value: &str, width: usize) -> String {
    let len = value.chars().count();
    if len <= width {
        return value.to_string();
    }
    let skip = len - width.saturating_sub(1);
    format!("\u{2026}{}", value.chars().skip(skip).collect::<String>())
}

fn draw_repo_list(state: &AppState, area: Rect, frame: &mut Frame) {
    let gh = &state.github;
    let [search, list_area] =
        ratatui::layout::Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Search ", LABEL),
            Span::styled(gh.query.clone(), Style::default().fg(Color::White)),
        ])),
        search,
    );
    frame.set_cursor_position(Position::new(
        search.x + 8 + gh.query.chars().count() as u16,
        search.y,
    ));

    let visible = visible_repos(gh);
    if visible.is_empty() {
        let (text, error) = match (&gh.repos_error, gh.repos_loading.is_some()) {
            (Some(err), _) => (err.clone(), true),
            (None, true) => ("reading repositories\u{2026}".to_string(), false),
            (None, false) if gh.query.contains('/') => (
                format!("Enter clones {} from GitHub", gh.query.trim()),
                false,
            ),
            (None, false) if gh.query.trim().is_empty() => (
                "no repositories here you can see \u{2014} \u{2190}\u{2192} lists another owner's"
                    .to_string(),
                false,
            ),
            (None, false) => (
                "nothing matches \u{2014} type owner/name to clone any repository".to_string(),
                false,
            ),
        };
        draw_placeholder(&text, error, list_area, frame);
        return;
    }
    let width = list_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = visible
        .iter()
        .map(|repo| {
            let mut tags = Vec::new();
            if repo.private {
                tags.push("private");
            }
            if repo.fork {
                tags.push("fork");
            }
            if repo.archived {
                tags.push("archived");
            }
            let tags = if tags.is_empty() {
                String::new()
            } else {
                format!(" {}", tags.join(" "))
            };
            let room = width.saturating_sub(tags.chars().count() + 1);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {:<room$}", truncate(&repo.name, room)),
                    Style::default().fg(Color::White),
                ),
                Span::styled(tags, LABEL),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(palette::selection())
        .highlight_symbol("\u{203a}");
    let rows = list_area.height as usize;
    let offset = gh.repo_idx.saturating_sub(rows.saturating_sub(1));
    let mut list_state = crate::panel::scroll::list_state(Some(gh.repo_idx), offset);
    frame.render_stateful_widget(list, list_area, &mut list_state);
}

fn draw_repo_detail(state: &AppState, area: Rect, frame: &mut Frame) {
    let Some(repo) = clone_request(state) else {
        return;
    };
    let description = visible_repos(&state.github)
        .get(state.github.repo_idx)
        .map(|listed| listed.description.clone())
        .unwrap_or_default();
    let mut lines = vec![
        Line::from(Span::styled(
            repo.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    if !description.is_empty() {
        lines.push(Line::from(description));
        lines.push(Line::from(""));
    }
    match clone_destination(state) {
        Some(dir) if dir.exists() => {
            lines.push(Line::from(Span::styled(
                "Already cloned at",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(dir.display().to_string()));
            lines.push(Line::from(Span::styled("Enter opens it", LABEL)));
        }
        Some(dir) => {
            lines.push(Line::from(Span::styled("Enter clones it into", LABEL)));
            lines.push(Line::from(Span::styled(
                dir.display().to_string(),
                Style::default().fg(Color::Yellow),
            )));
            lines.push(Line::from(Span::styled("and switches lg to it", LABEL)));
        }
        None => lines.push(Line::from(Span::styled(
            "no folder to clone into",
            Style::default().fg(Color::Red),
        ))),
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(1),
            ..area
        },
    );
}

fn draw_bottom(state: &AppState, area: Rect, frame: &mut Frame) {
    let line = match (&state.github.mode, state.github.tab) {
        (Mode::Compose { kind, text }, _) => {
            let number = selected_pr(state).map(|pr| pr.number).unwrap_or(0);
            let prompt = format!("{} #{number}: ", kind.label());
            let hint = match kind {
                Compose::Approve => "  (optional)  Enter send  Esc cancel",
                _ => "  Enter send  Esc cancel",
            };
            let x = area.x + (prompt.chars().count() + text.chars().count()) as u16;
            if x < area.x + area.width {
                frame.set_cursor_position(Position::new(x, area.y));
            }
            Line::from(vec![
                Span::styled(prompt, Style::default().fg(Color::Cyan)),
                Span::styled(text.clone(), Style::default().fg(Color::White)),
                Span::styled(if text.is_empty() { hint } else { "" }, LABEL),
            ])
        }
        (Mode::ConfirmClose, _) => {
            let (number, title) = selected_pr(state)
                .map(|pr| (pr.number, pr.title.clone()))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(
                    format!("Close #{number} \u{201c}{title}\u{201d} without merging?  "),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled("y", Style::default().fg(Color::Red)),
                Span::raw(" close  "),
                Span::styled("n/Esc", HINT_KEY),
                Span::raw(" keep it"),
            ])
        }
        (Mode::Merge(_), _) => ui::key_hints(&[
            ("j/k", "row"),
            ("Space/\u{2190}\u{2192}", "change"),
            ("Enter", "merge"),
            ("Esc", "back"),
        ]),
        (Mode::Create(_), _) => ui::key_hints(&[
            ("Ctrl-S", "open pull request"),
            ("Tab", "field"),
            ("Enter", "next / newline in body"),
            ("Esc", "cancel"),
        ]),
        (Mode::Browse, Tab::PullRequests) => ui::key_hints(&[
            ("Enter", "checkout"),
            ("w", "worktree"),
            ("a", "approve"),
            ("x", "request changes"),
            ("c", "comment"),
            ("m", "merge"),
            ("D", "draft"),
            ("X", "close"),
            ("n", "new"),
            ("o", "browser"),
            ("Tab", "repos"),
        ]),
        (Mode::Browse, Tab::Repositories) => ui::key_hints(&[
            ("type", "search"),
            ("\u{2191}\u{2193}", "select"),
            ("\u{2190}\u{2192}", "owner"),
            ("Enter", "clone"),
            ("Ctrl-R", "reload"),
            ("Tab", "pull requests"),
            ("Esc", "close"),
        ]),
    };
    frame.render_widget(Paragraph::new(line), area);
}
