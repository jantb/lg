//! A file's diff as the walk draws it, with its notes and the findings under
//! their lines.

use super::*;

pub(super) struct DiffLines {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) cursor: std::ops::Range<usize>,
    pub(super) findings: Vec<(std::ops::Range<usize>, TickedFinding)>,
}

impl DiffLines {
    /// Keeps the cursor line and what is pinned under it in view.
    pub(super) fn scroll(&self, height: u16) -> usize {
        (self.cursor.end + 2)
            .saturating_sub(usize::from(height))
            .min(self.cursor.start)
    }
}

pub(super) fn diff_lines(
    guided: &Guided,
    step: &GuidedFile,
    mode: DiffViewMode,
    width: u16,
) -> DiffLines {
    let mut out = DiffLines {
        lines: vec![file_title(guided, step)],
        cursor: 0..0,
        findings: Vec::new(),
    };
    let lead = inline_lead(mode, width);
    let flagged = inline_findings(guided, step, &lead, width);
    push_diff(
        guided,
        step,
        mode,
        width,
        &flagged,
        Some(guided.cursor),
        &mut out,
    );
    out
}

/// A file's rows, with its notes and `flagged` findings under their lines,
/// added to `out`; the rows of line `cursor` become `out.cursor`.
pub(super) fn push_diff(
    guided: &Guided,
    step: &GuidedFile,
    mode: DiffViewMode,
    width: u16,
    flagged: &[Inline],
    cursor: Option<usize>,
    out: &mut DiffLines,
) {
    let lines = &mut out.lines;
    let notes = guided.notes_for(step);
    let lead = inline_lead(mode, width);
    for row in layout::rows(step, mode) {
        let start = lines.len();
        let worst = flagged
            .iter()
            .filter(|inline| !inline.ticked && row.lines().any(|line| line == inline.at))
            .map(|inline| inline.finding.severity)
            .max();
        let bar = match worst {
            Some(severity) => {
                Span::styled("\u{258c}", Style::default().fg(severity_color(severity)))
            }
            None => Span::raw(" "),
        };
        let body = match row {
            Row::Whole(at) => vec![unified_line(step, at, Some(at) == cursor)],
            Row::Pair(left, right) => {
                paired_lines(step, left, right, cursor, width.saturating_sub(1))
            }
        };
        lines.extend(body.into_iter().enumerate().map(|(i, line)| {
            let lead = if i == 0 { bar.clone() } else { Span::raw(" ") };
            Line::from(std::iter::once(lead).chain(line.spans).collect::<Vec<_>>())
                .style(line.style)
        }));
        for at in row.lines() {
            let here = Some(at) == cursor;
            if let Some(anchor) = step.lines[at].anchor() {
                for (_, note) in notes
                    .iter()
                    .filter(|(_, note)| (note.line, note.side) == anchor)
                {
                    lines.extend(note_lines(&note.body, &lead));
                }
            }
            for inline in flagged.iter().filter(|inline| inline.at == at) {
                let first = lines.len();
                lines.extend(inline.lines.iter().cloned().map(|line| {
                    if here {
                        line.style(Style::default().bg(CURSOR_BG).add_modifier(Modifier::BOLD))
                    } else {
                        line
                    }
                }));
                out.findings
                    .push((first..lines.len(), inline.ticket.clone()));
            }
        }
        if row.lines().any(|at| Some(at) == cursor) {
            out.cursor = start..lines.len();
        }
    }
    // Notes pinned to lines this diff no longer shows, said once at the end.
    let shown: Vec<(usize, Side)> = step.lines.iter().filter_map(|l| l.anchor()).collect();
    for (_, note) in notes
        .iter()
        .filter(|(_, n)| !shown.contains(&(n.line, n.side)))
    {
        lines.push(Line::from(vec![
            Span::styled(format!("  \u{270e} L{} (moved) ", note.line), NOTE),
            Span::styled(note.body.clone(), NOTE),
        ]));
    }
    if step.lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no text diff to show for this file",
            LABEL,
        )));
    }
}

pub(super) fn file_title(guided: &Guided, step: &GuidedFile) -> Line<'static> {
    let mut title = vec![Span::styled(
        step.path.clone(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    title.push(Span::styled(format!("  {}", file_label(step)), LABEL));
    if let Some(note) = step.file_note.as_ref().filter(|_| !step.lines.is_empty()) {
        title.push(Span::styled(
            format!("  [{note}]"),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(verdict) = guided.verdict(step) {
        let (word, color) = verdict_look(verdict);
        title.push(Span::raw("  "));
        title.push(Span::styled(
            format!(" {word} "),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if guided.is_reviewed(step) {
        title.push(Span::styled(
            "  reviewed",
            Style::default().fg(Color::Green),
        ));
    }
    Line::from(title)
}

fn unified_line(step: &GuidedFile, at: usize, here: bool) -> Line<'static> {
    let line = &step.lines[at];
    if line.kind == LineKind::Header {
        return Line::from(vec![
            Span::styled(format!("{:>12}", "\u{22ef} "), LABEL),
            Span::styled(line.text.clone(), Style::default().fg(Color::Cyan)),
        ]);
    }
    let number = |n: Option<usize>| n.map(|n| n.to_string()).unwrap_or_default();
    let gutter = format!("{:>5} {:>5} ", number(line.old), number(line.new));
    let spans: Vec<Span<'static>> = std::iter::once(Span::styled(gutter, LABEL))
        .chain(
            ui::highlight_diff_line_for_path(&line.text, &step.path)
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), span.style)),
        )
        .collect();
    let rendered = Line::from(spans);
    if here {
        rendered.style(Style::default().bg(CURSOR_BG))
    } else {
        rendered
    }
}

fn paired_lines(
    step: &GuidedFile,
    left: Option<usize>,
    right: Option<usize>,
    cursor: Option<usize>,
    width: u16,
) -> Vec<Line<'static>> {
    let cell = |at: Option<usize>, number: fn(&HunkLine) -> Option<usize>| {
        let line = &step.lines[at?];
        let kind = match line.kind {
            LineKind::Context => DiffContentKind::Context,
            LineKind::Added => DiffContentKind::Added,
            LineKind::Removed => DiffContentKind::Removed,
            LineKind::Meta | LineKind::Header => return None,
        };
        Some((number(line)?, line.text.get(1..).unwrap_or_default(), kind))
    };
    let rows = ui::side_by_side_pair_for_path(
        cell(left, |line| line.old),
        cell(right, |line| line.new),
        width,
        &step.path,
    );
    let (left_width, right_start) = ui::side_by_side_columns(width);
    let at_cursor = |at: Option<usize>| at.is_some() && at == cursor;
    let lit = match (at_cursor(left), at_cursor(right)) {
        (true, true) => Some(0..width),
        (true, false) => Some(0..left_width),
        (false, true) => Some(right_start..width),
        (false, false) => None,
    };
    match lit {
        Some(columns) => rows.into_iter().map(|row| light(row, &columns)).collect(),
        None => rows,
    }
}

fn light(line: Line<'static>, columns: &std::ops::Range<u16>) -> Line<'static> {
    let mut column = 0u16;
    let spans = line
        .spans
        .into_iter()
        .map(|span| {
            let start = column;
            column = column.saturating_add(u16::try_from(span.width()).unwrap_or(u16::MAX));
            if columns.contains(&start) {
                let lit = span.style.bg(CURSOR_BG);
                span.style(lit)
            } else {
                span
            }
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

fn note_lines(body: &str, lead: &str) -> Vec<Line<'static>> {
    body.lines()
        .enumerate()
        .map(|(i, text)| {
            let mark = if i == 0 { "\u{270e} " } else { "  " };
            Line::from(vec![
                Span::styled(format!("{lead}{mark}"), NOTE),
                Span::styled(text.to_string(), NOTE),
            ])
        })
        .collect()
}

pub(super) struct Inline {
    at: usize,
    finding: Finding,
    ticket: TickedFinding,
    ticked: bool,
    lines: Vec<Line<'static>>,
}

/// Side by side, right of the middle: under the new code it is nearly always about.
pub(super) fn inline_lead(mode: DiffViewMode, width: u16) -> String {
    let column = match mode {
        DiffViewMode::Unified => 7,
        DiffViewMode::SideBySide => {
            1 + usize::from(ui::side_by_side_columns(width.saturating_sub(1)).1)
        }
    };
    " ".repeat(column)
}

fn inline_findings(guided: &Guided, step: &GuidedFile, lead: &str, width: u16) -> Vec<Inline> {
    placed_inline(guided, step, &guided.findings_in(step), lead, width)
}

/// The final review's findings that cite `step`'s file, under their lines.
pub(super) fn final_inline(
    guided: &Guided,
    step: &GuidedFile,
    lead: &str,
    width: u16,
) -> Vec<Inline> {
    let Some(commentary) = guided.commentary.get(&guided.final_key()) else {
        return Vec::new();
    };
    let cited: Vec<Finding> = findings::read(&commentary.text)
        .cited
        .into_iter()
        .filter(|(path, _)| *path == step.path)
        .map(|(_, finding)| finding)
        .collect();
    placed_inline(guided, step, &cited, lead, width)
}

fn placed_inline(
    guided: &Guided,
    step: &GuidedFile,
    found: &[Finding],
    lead: &str,
    width: u16,
) -> Vec<Inline> {
    findings::place(step, found)
        .0
        .into_iter()
        .map(|(at, finding)| {
            let ticket = super::ticket_in(&step.path, finding);
            let ticked = guided.is_ticked(&ticket);
            Inline {
                at,
                finding: finding.clone(),
                ticket,
                ticked,
                lines: finding_lines(finding, ticked, lead, width),
            }
        })
        .collect()
}

pub(super) fn finding_lines(
    finding: &Finding,
    ticked: bool,
    lead: &str,
    width: u16,
) -> Vec<Line<'static>> {
    let (mark, style) = match (ticked, finding.severity) {
        (true, _) => (
            "\u{2713} ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::CROSSED_OUT),
        ),
        (false, severity) => (
            severity_mark(severity),
            Style::default().fg(severity_color(severity)),
        ),
    };
    let indent = lead.chars().count() + 2;
    let available = usize::from(width).saturating_sub(indent).max(10);
    crate::panel::markdown::word_wrap(&finding.text, available)
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let head = if i == 0 {
                format!("{lead}{mark}")
            } else {
                " ".repeat(indent)
            };
            Line::from(vec![Span::styled(head, style), Span::styled(chunk, style)])
        })
        .collect()
}
