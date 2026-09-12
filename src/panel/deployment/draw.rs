//! The environments modal: the pipeline on the left, drawn as a tree with a
//! marker travelling the route a promotion would take, and on the right what
//! the selected environment is, how the branch stands against it, and what
//! Enter would do about it.

use ratatui::{
    Frame,
    layout::{Constraint, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    git::environments::PromotionPreview,
    preferences::Branches,
    state::AppState,
    ui::{self, palette},
};

use super::pipeline::{Inclusion, Pipeline, Stage, rule_label, strategy_sentence};

/// Milliseconds the marker spends on each cell of its route.
const STEP_MS: u64 = 110;
/// Milliseconds the marker rests on the destination before setting off again.
const REST_MS: u64 = 900;
/// Column of the root's dot; the columns before it hold the selection marker.
const ROOT_X: usize = 2;

pub(super) struct Regions {
    pub modal: Rect,
    pub header: Rect,
    pub pipeline: Rect,
    pub detail: Rect,
    pub footer: Rect,
    pub dividers: Vec<Rect>,
}

pub(super) fn regions(area: Rect) -> Regions {
    let modal = ui::centered(
        area,
        area.width.saturating_sub(4).min(124),
        area.height.saturating_sub(2),
    );
    let inner = ui::modal_inner(modal);
    let (bands, mut dividers) = ui::modal_row_areas(
        inner,
        &[
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
        ],
    );
    let (columns, gaps) =
        ui::modal_column_areas(bands[1], &[Constraint::Percentage(58), Constraint::Min(24)]);
    dividers.extend(gaps);
    Regions {
        modal,
        header: bands[0],
        pipeline: columns[0],
        detail: columns[1],
        footer: bands[2],
        dividers,
    }
}

/// Where each stage is drawn: its row, and the column of its dot. Stages are
/// two rows apart when the pane has room for a breathing row between them.
pub(super) struct Geometry {
    pub stride: usize,
    pub row: Vec<usize>,
    pub dot_x: Vec<usize>,
}

pub(super) fn geometry(pipeline: &Pipeline, pane: Rect) -> Geometry {
    let count = pipeline.stages.len();
    let stride = if count * 2 <= usize::from(pane.height) + 1 {
        2
    } else {
        1
    };
    let mut dot_x = Vec::with_capacity(count);
    for stage in &pipeline.stages {
        let x = match stage.parent {
            None => ROOT_X,
            Some(parent) => dot_x[parent] + 1 + connector_label(stage).chars().count(),
        };
        dot_x.push(x);
    }
    Geometry {
        stride,
        row: (0..count).map(|i| i * stride).collect(),
        dot_x,
    }
}

/// The text on the connector into `stage`, arrow included, without the
/// junction glyph that starts it.
fn connector_label(stage: &Stage) -> String {
    match &stage.rule {
        Some(rule) => format!(
            "\u{2500}\u{2500} {} \u{2500}\u{2500}\u{25b6} ",
            rule_label(rule)
        ),
        None => "\u{254c}\u{254c} no rule \u{254c}\u{254c}\u{25b6} ".to_string(),
    }
}

/// A grid of styled characters the tree is composed on, so lines, labels and
/// the travelling marker can be placed independently and read out as rows.
struct Canvas {
    width: usize,
    height: usize,
    cells: Vec<Option<(char, Style)>>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![None; width * height],
        }
    }
    fn put(&mut self, x: usize, y: usize, ch: char, style: Style) {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x] = Some((ch, style));
        }
    }
    fn at(&self, x: usize, y: usize) -> Option<(char, Style)> {
        (x < self.width && y < self.height).then(|| self.cells[y * self.width + x])?
    }
    /// Writes `text` from `x` and returns the column after it.
    fn text(&mut self, x: usize, y: usize, text: &str, style: Style) -> usize {
        let mut x = x;
        for ch in text.chars() {
            self.put(x, y, ch, style);
            x += 1;
        }
        x
    }
    fn lines(&self, highlighted: Option<usize>) -> Vec<Line<'static>> {
        (0..self.height)
            .map(|y| {
                let row_style = if highlighted == Some(y) {
                    palette::selection()
                } else {
                    Style::default()
                };
                let spans = (0..self.width)
                    .map(|x| match self.cells[y * self.width + x] {
                        Some((ch, style)) => Span::styled(ch.to_string(), row_style.patch(style)),
                        None => Span::styled(" ", row_style),
                    })
                    .collect::<Vec<_>>();
                Line::from(spans)
            })
            .collect()
    }
}

/// The cells of the connector into `stage`, in the order a promotion travels
/// them: down the parent's spine, then along to the stage's dot.
fn connector_cells(pipeline: &Pipeline, geometry: &Geometry, stage: usize) -> Vec<(usize, usize)> {
    let Some(parent) = pipeline.stages[stage].parent else {
        return Vec::new();
    };
    let px = geometry.dot_x[parent];
    let mut cells: Vec<(usize, usize)> = (geometry.row[parent] + 1..geometry.row[stage])
        .map(|y| (px, y))
        .collect();
    cells.extend((px..geometry.dot_x[stage]).map(|x| (x, geometry.row[stage])));
    cells
}

/// Where the marker is on its route, or `None` while it rests on the
/// destination.
fn marker_index(clock_ms: u64, route_len: usize) -> Option<usize> {
    if route_len == 0 {
        return None;
    }
    let period = route_len as u64 * STEP_MS + REST_MS;
    let step = (clock_ms % period) / STEP_MS;
    (step < route_len as u64).then_some(step as usize)
}

fn inclusion_span(inclusion: &Inclusion) -> Option<Span<'static>> {
    match inclusion {
        Inclusion::Source => Some(Span::styled(
            "\u{25c9} your branch",
            Style::default().fg(palette::ACCENT),
        )),
        Inclusion::Included { .. } => Some(Span::styled(
            "\u{2713} included",
            Style::default().fg(palette::OK),
        )),
        Inclusion::Behind(n) => Some(Span::styled(
            format!("{n} commit{} behind", if *n == 1 { "" } else { "s" }),
            Style::default().fg(palette::LANE_TEST),
        )),
        Inclusion::Unknown => None,
    }
}

fn muted(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(palette::TEXT_IDLE))
}

fn dim() -> Style {
    Style::default().fg(palette::FRAME_IDLE)
}

/// Draws the tree into `pane`. `selected` is the stage the route lights up
/// for, `clock_ms` where the marker is; `None` holds the picture still.
fn render_pipeline(
    pipeline: &Pipeline,
    selected: Option<usize>,
    clock_ms: Option<u64>,
    pane: Rect,
    frame: &mut Frame,
) {
    let geometry = geometry(pipeline, pane);
    let mut canvas = Canvas::new(usize::from(pane.width), usize::from(pane.height));
    let route = selected.map(|s| pipeline.route(s)).unwrap_or_default();

    for (i, stage) in pipeline.stages.iter().enumerate() {
        let y = geometry.row[i];
        let on_route = route.contains(&i);
        let line_style = if on_route {
            Style::default().fg(stage.color)
        } else {
            dim()
        };
        if let Some(parent) = stage.parent {
            let px = geometry.dot_x[parent];
            for spine_y in geometry.row[parent] + 1..y {
                // The spine runs past the junctions of the siblings above,
                // which keep their glyph; a spine a route has lit stays lit.
                match canvas.at(px, spine_y) {
                    None => canvas.put(px, spine_y, '\u{2502}', line_style),
                    Some(('\u{2502}', _)) if on_route => {
                        canvas.put(px, spine_y, '\u{2502}', line_style)
                    }
                    Some(_) => {}
                }
            }
            let junction = if pipeline.has_later_sibling(i) {
                '\u{251c}'
            } else {
                '\u{2514}'
            };
            canvas.put(px, y, junction, line_style);
            canvas.text(px + 1, y, &connector_label(stage), line_style);
        }
        let dot_style = Style::default().fg(stage.color);
        let mut x = canvas.text(geometry.dot_x[i], y, "\u{25cf} ", dot_style);
        let name_style = if Some(i) == selected || on_route || stage.parent.is_none() {
            dot_style.add_modifier(Modifier::BOLD)
        } else {
            dot_style
        };
        x = canvas.text(x, y, &stage.name, name_style);
        if !stage.target.is_empty() {
            x = canvas.text(x, y, &format!("  {}", stage.target), muted("").style);
        }
        if let Some(span) = inclusion_span(&stage.inclusion) {
            // On the breathing row under the name when there is one, so a
            // long row does not push it off the pane.
            if geometry.stride > 1 {
                canvas.text(geometry.dot_x[i] + 2, y + 1, &span.content, span.style);
            } else {
                canvas.text(x, y, &format!("  {}", span.content), span.style);
            }
        }
    }

    if let Some(sel) = selected {
        canvas.put(
            0,
            geometry.row[sel],
            '\u{203a}',
            Style::default()
                .fg(palette::ACCENT)
                .add_modifier(Modifier::BOLD),
        );
    }

    // The marker walks the whole route from the root to the selected stage
    // and rests on it, over and over, so watching it is reading the rule.
    if let (Some(clock), Some(&destination)) = (clock_ms, route.last()) {
        let path: Vec<(usize, usize)> = route
            .iter()
            .flat_map(|&stage| connector_cells(pipeline, &geometry, stage))
            .collect();
        match marker_index(clock, path.len()) {
            Some(index) => {
                let (x, y) = path[index];
                canvas.put(
                    x,
                    y,
                    '\u{25c6}',
                    Style::default()
                        .fg(palette::glow(0.9))
                        .add_modifier(Modifier::BOLD),
                );
            }
            None => canvas.put(
                geometry.dot_x[destination],
                geometry.row[destination],
                '\u{25cf}',
                Style::default()
                    .fg(palette::glow(1.0))
                    .add_modifier(Modifier::BOLD),
            ),
        }
    }

    let highlighted = selected.map(|s| geometry.row[s]);
    frame.render_widget(Paragraph::new(canvas.lines(highlighted)), pane);
}

fn render_header(pipeline: &Pipeline, config: &Branches, area: Rect, frame: &mut Frame) {
    let root = &pipeline.stages[0];
    let standing = match root.env {
        Some(_) => format!("the branch {} deploys", root.name),
        None => "a feature branch, promoted into environments by rule".to_string(),
    };
    let lines = vec![
        Line::from(vec![
            muted("Integration  "),
            Span::styled(
                config.base.clone(),
                Style::default()
                    .fg(palette::LANE_MAIN)
                    .add_modifier(Modifier::BOLD),
            ),
            muted("   Remote  "),
            Span::styled(config.remote.clone(), Style::default().fg(palette::ACCENT)),
            muted(format!(
                "   {} environment{}",
                config.environments.len(),
                if config.environments.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )),
        ]),
        Line::from(vec![
            muted("You are on   "),
            Span::styled(
                pipeline.source.clone(),
                Style::default().fg(root.color).add_modifier(Modifier::BOLD),
            ),
            muted(format!("   \u{b7} {standing}")),
        ]),
        Line::from(muted(
            "Each arrow is a promotion rule. Nothing moves until a preview is confirmed with Enter.",
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn heading(text: impl Into<String>, color: ratatui::style::Color) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

fn label(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(palette::ACCENT)
            .add_modifier(Modifier::BOLD),
    ))
}

/// The right pane while choosing: the selected environment explained.
fn detail_lines(
    state: &AppState,
    pipeline: &Pipeline,
    selected: Option<usize>,
    protected: bool,
) -> Vec<Line<'static>> {
    let Some(sel) = selected else {
        return vec![
            heading("No environments yet", palette::TEXT_IDLE),
            Line::default(),
            Line::from(muted(
                "An environment is a remote branch that gets deployed: origin/develop for Development, say.",
            )),
            Line::from(muted(
                "Press , to add one under Branches & Environments, then a promotion rule that leads to it.",
            )),
        ];
    };
    let stage = &pipeline.stages[sel];
    let root = &pipeline.stages[0];
    let mut lines = vec![heading(format!("\u{25cf} {}", stage.name), stage.color)];
    let mut where_ = Vec::new();
    if stage.target.is_empty() {
        where_.push(Span::styled(
            "no branch assigned",
            Style::default().fg(palette::BAD),
        ));
    } else {
        where_.push(muted(format!("deploys {}", stage.target)));
    }
    if !stage.url.is_empty() {
        where_.push(muted(format!("  {}", stage.url)));
    }
    lines.push(Line::from(where_));
    lines.push(Line::default());

    lines.push(label(&format!("Your branch \u{b7} {}", pipeline.source)));
    lines.push(match &stage.inclusion {
        Inclusion::Source => Line::from(muted(format!(
            "This is the branch {} deploys, so there is nothing to include.",
            stage.name
        ))),
        Inclusion::Included { released_at } if released_at.is_empty() => Line::from(Span::styled(
            format!("\u{2713} Every commit is already in {}.", stage.name),
            Style::default().fg(palette::OK),
        )),
        Inclusion::Included { released_at } => Line::from(Span::styled(
            format!(
                "\u{2713} Every commit is already in {}, released {released_at}.",
                stage.name
            ),
            Style::default().fg(palette::OK),
        )),
        Inclusion::Behind(n) => Line::from(Span::styled(
            format!(
                "{n} commit{} on {} {} not in {} yet.",
                if *n == 1 { "" } else { "s" },
                pipeline.source,
                if *n == 1 { "is" } else { "are" },
                stage.name
            ),
            Style::default().fg(palette::LANE_TEST),
        )),
        Inclusion::Unknown if protected => Line::from(muted(format!(
            "Not compared: {} is an integration branch, and lg measures feature branches against the environments.",
            pipeline.source
        ))),
        Inclusion::Unknown => Line::from(muted(
            "Not compared yet. The comparison runs in the background once the branch has been fetched.",
        )),
    });
    lines.push(Line::default());

    lines.push(label("Promotion"));
    let route = pipeline.route(sel);
    match (&stage.inclusion, route.as_slice()) {
        (Inclusion::Source, _) => lines.push(Line::from(muted(
            "Choose another environment to see how this branch is promoted into it.",
        ))),
        (_, []) => {
            lines.push(Line::from(Span::styled(
                format!("No rule promotes {} into {}.", root.role, stage.name),
                Style::default().fg(palette::BAD),
            )));
            lines.push(Line::from(muted(format!(
                "Press , and add a promotion with from = {} and to = {} under Branches & Environments.",
                root.role, stage.role
            ))));
        }
        (_, [_]) => {
            let rule = stage.rule.as_ref().expect("a one-hop route has a rule");
            let mut spans = vec![
                Span::styled(
                    root.name.clone(),
                    Style::default().fg(root.color).add_modifier(Modifier::BOLD),
                ),
                muted(" \u{2500}\u{2500}\u{25b6} "),
                Span::styled(
                    stage.name.clone(),
                    Style::default()
                        .fg(stage.color)
                        .add_modifier(Modifier::BOLD),
                ),
                muted(format!("   {}", rule_label(rule))),
            ];
            spans.truncate(4);
            lines.push(Line::from(spans));
            lines.push(Line::from(muted(format!(
                "{} is merged into {} {}, {}.",
                pipeline.source,
                stage.target,
                strategy_sentence(rule, &stage.target),
                if rule.push {
                    format!("then pushed to {}", stage.target)
                } else {
                    "and kept as a safety ref rather than pushed".to_string()
                }
            ))));
            lines.push(Line::from(vec![
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(palette::HINT_KEY)
                        .add_modifier(Modifier::BOLD),
                ),
                muted(" lists exactly which commits would move before anything happens."),
            ]));
        }
        (_, hops) => {
            let names: Vec<String> = std::iter::once(root.name.clone())
                .chain(hops.iter().map(|&h| pipeline.stages[h].name.clone()))
                .collect();
            lines.push(Line::from(muted(names.join(" \u{2500}\u{2500}\u{25b6} "))));
            let first = &pipeline.stages[hops[0]];
            lines.push(Line::from(muted(format!(
                "{} reaches {} through {}: promote it there first, then from {} onwards.",
                pipeline.source, stage.name, first.name, first.name
            ))));
        }
    }
    lines.push(Line::default());

    lines.push(label("Assign a branch"));
    let highlighted = state
        .branches
        .get(state.branches_idx)
        .map(|b| b.name.clone())
        .or_else(|| state.branch.clone())
        .unwrap_or_else(|| "the highlighted branch".into());
    lines.push(Line::from(vec![
        Span::styled(
            "a",
            Style::default()
                .fg(palette::HINT_KEY)
                .add_modifier(Modifier::BOLD),
        ),
        muted(format!(
            " makes {highlighted} the branch {} deploys. It is staged in Settings and saved with Ctrl-S; no branch moves.",
            stage.name
        )),
    ]));
    lines
}

/// The right pane while previewing: the commits a promotion would move.
fn preview_lines(preview: &PromotionPreview, stage: &Stage, height: usize) -> Vec<Line<'static>> {
    let target = format!(
        "{}/{}",
        preview.environment.remote, preview.environment.branch
    );
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                preview.source.clone(),
                Style::default()
                    .fg(palette::LANE_FEATURE)
                    .add_modifier(Modifier::BOLD),
            ),
            muted(format!("  {}", &preview.source_oid[..8])),
            muted("   \u{2500}\u{2500}\u{25b6}   "),
            Span::styled(
                stage.name.clone(),
                Style::default()
                    .fg(stage.color)
                    .add_modifier(Modifier::BOLD),
            ),
            muted(format!("  {target}  {}", &preview.target_oid[..8])),
        ]),
        Line::from(muted(format!(
            "Lands {}, {}.",
            strategy_sentence(&preview.rule, &target),
            if preview.rule.push {
                format!("then pushed to {target}")
            } else {
                "kept as a safety ref rather than pushed".to_string()
            }
        ))),
        Line::default(),
    ];
    let count = preview.commits.len();
    if count == 0 {
        lines.push(Line::from(Span::styled(
            format!(
                "Nothing to move: {target} already has every commit of {}.",
                preview.source
            ),
            Style::default().fg(palette::OK),
        )));
    } else {
        lines.push(label(&format!(
            "{count} commit{} would move",
            if count == 1 { "" } else { "s" }
        )));
        let room = height.saturating_sub(lines.len() + 3);
        for commit in preview.commits.iter().take(room) {
            let (hash, subject) = commit.split_once(' ').unwrap_or((commit, ""));
            lines.push(Line::from(vec![
                Span::styled(hash.to_string(), Style::default().fg(palette::ACCENT)),
                Span::raw(format!(" {subject}")),
            ]));
        }
        if count > room {
            lines.push(Line::from(muted(format!(
                "  \u{2026} and {} more",
                count - room
            ))));
        }
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled(
            "Enter",
            Style::default()
                .fg(palette::HINT_KEY)
                .add_modifier(Modifier::BOLD),
        ),
        muted(" promotes. The refs are checked again first; if either has moved, nothing happens and the preview reopens."),
    ]));
    lines
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let config = crate::preferences::load().config.branches;
    render_with(state, &config, area, frame);
}

/// Draws the modal for an explicit configuration, so a test can put the
/// environments it wants on screen.
pub fn render_with(state: &AppState, config: &Branches, area: Rect, frame: &mut Frame) {
    let view = &state.environment_view;
    let r = regions(area);
    let pipeline = Pipeline::build(
        config,
        state.branch.as_deref(),
        &state.current_branch_releases,
    );
    let selected_env =
        (!config.environments.is_empty()).then(|| view.selected.min(config.environments.len() - 1));
    let selected = selected_env.and_then(|env| pipeline.stage_of(env));
    let previewing = view.preview.is_some();

    let title = if previewing {
        "Environments \u{b7} promotion preview"
    } else {
        "Environments \u{b7} where branches deploy and how they are promoted"
    };
    ui::modal_frame(frame, r.modal, title);
    ui::draw_dividers(frame, &r.dividers);

    render_header(&pipeline, config, r.header, frame);
    ui::section_title(frame, r.pipeline, "Pipeline");
    let clock = state.decorative_animations.then_some(state.animation_ms);
    render_pipeline(&pipeline, selected, clock, r.pipeline, frame);

    let protected = state
        .branch
        .as_deref()
        .is_some_and(|b| config.protected.iter().any(|p| p == b) || b == config.base);
    let lines = match (&view.preview, selected) {
        (Some(preview), Some(sel)) => {
            ui::section_title(frame, r.detail, "What would move");
            preview_lines(preview, &pipeline.stages[sel], usize::from(r.detail.height))
        }
        _ => {
            ui::section_title(frame, r.detail, "Selected environment");
            detail_lines(state, &pipeline, selected, protected)
        }
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), r.detail);

    let mut footer = vec![if previewing {
        ui::key_hints(&[("Enter", "promote"), ("Esc", "back to the pipeline")])
    } else {
        ui::key_hints(&[
            ("j/k", "select"),
            ("Enter", "preview promotion"),
            ("a", "assign branch"),
            (",", "configure"),
            ("Esc", "close"),
        ])
    }];
    if !view.notice.is_empty() {
        let style = if view.notice_error {
            Style::default()
                .fg(palette::BAD)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette::TEXT_IDLE)
        };
        footer.push(Line::from(Span::styled(view.notice.clone(), style)));
    }
    frame.render_widget(Paragraph::new(footer).wrap(Wrap { trim: false }), r.footer);

    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, r.modal, &r.dividers, frame);
    }
}

/// The configured environment drawn on the pipeline row under `at`, if any.
pub(super) fn environment_at(
    config: &Branches,
    state: &AppState,
    area: Rect,
    at: Position,
) -> Option<usize> {
    let r = regions(area);
    if !r.pipeline.contains(at) {
        return None;
    }
    let pipeline = Pipeline::build(
        config,
        state.branch.as_deref(),
        &state.current_branch_releases,
    );
    let geometry = geometry(&pipeline, r.pipeline);
    let row = usize::from(at.y - r.pipeline.y);
    pipeline
        .stages
        .iter()
        .zip(&geometry.row)
        .find(|(_, y)| **y == row)
        .and_then(|(stage, _)| stage.env)
}
