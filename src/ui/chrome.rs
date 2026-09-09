use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear},
};

use super::palette;
use crate::{
    config::{ANIMATION_STEP_MS, BORDER_COLOR},
    state::SPINNER_FRAMES,
};

/// A block with the default border color and the given title.
pub fn bordered(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(title)
}

/// A modal is one box. The panes inside it are told apart by single lines
/// that join that box, rather than by each pane carrying a frame of its own:
/// stacked frames read as several dialogs sharing a screen, and they spend two
/// rows and two columns on every seam.
///
/// Clears `area`, draws the frame with `title` on it, and hands back the room
/// inside it for the panes.
pub fn modal_frame(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    modal_frame_with(frame, area, bordered(title))
}

/// As [`modal_frame`], for a modal whose frame carries more than a title —
/// a scroll position along the bottom edge, say.
pub fn modal_frame_with(frame: &mut Frame, area: Rect, block: Block<'_>) -> Rect {
    frame.render_widget(Clear, area);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// The room inside a modal's frame, measured without drawing it: the layout
/// the panes are cut from.
pub fn modal_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Splits a modal's inside into columns, drawing a divider between each pair
/// and returning the panes and the dividers. The dividers join the frame above
/// and below them, so the result is one box with rooms in it; they are handed
/// back because the travelling light has to reach along them too.
pub fn modal_columns(
    frame: &mut Frame,
    inner: Rect,
    constraints: &[Constraint],
) -> (Vec<Rect>, Vec<Rect>) {
    let (panes, gaps) = modal_column_areas(inner, constraints);
    for gap in &gaps {
        divider_v(frame, *gap);
    }
    (panes, gaps)
}

/// [`modal_columns`] laid the other way: rows separated by a full-width
/// divider.
pub fn modal_rows(
    frame: &mut Frame,
    inner: Rect,
    constraints: &[Constraint],
) -> (Vec<Rect>, Vec<Rect>) {
    let (panes, gaps) = modal_row_areas(inner, constraints);
    for gap in &gaps {
        divider_h(frame, *gap);
    }
    (panes, gaps)
}

/// The panes and the one-cell gaps between them, measured but not drawn: the
/// same split as [`modal_columns`], for the code that needs to know where a
/// pane landed without a frame in hand.
pub fn modal_column_areas(inner: Rect, constraints: &[Constraint]) -> (Vec<Rect>, Vec<Rect>) {
    divided_areas(inner, constraints, Direction::Horizontal)
}

/// [`modal_column_areas`] laid the other way.
pub fn modal_row_areas(inner: Rect, constraints: &[Constraint]) -> (Vec<Rect>, Vec<Rect>) {
    divided_areas(inner, constraints, Direction::Vertical)
}

/// Draws the dividers a split measured out, for a caller that took the panes
/// from [`modal_column_areas`] or [`modal_row_areas`] first.
pub fn draw_dividers(frame: &mut Frame, gaps: &[Rect]) {
    for gap in gaps {
        if gap.width == 1 {
            divider_v(frame, *gap);
        } else if gap.height == 1 {
            divider_h(frame, *gap);
        }
    }
}

fn divided_areas(
    inner: Rect,
    constraints: &[Constraint],
    direction: Direction,
) -> (Vec<Rect>, Vec<Rect>) {
    let mut interleaved: Vec<Constraint> = Vec::with_capacity(constraints.len() * 2);
    for (idx, constraint) in constraints.iter().enumerate() {
        if idx > 0 {
            interleaved.push(Constraint::Length(1));
        }
        interleaved.push(*constraint);
    }
    let chunks = Layout::default()
        .direction(direction)
        .constraints(interleaved)
        .split(inner);
    (
        chunks.iter().step_by(2).copied().collect(),
        chunks.iter().skip(1).step_by(2).copied().collect(),
    )
}

/// A divider down a one-cell-wide gap, joined to whatever it runs into at
/// either end.
fn divider_v(frame: &mut Frame, gap: Rect) {
    for y in gap.y..gap.y.saturating_add(gap.height) {
        draw_line(frame, gap.x, y, ARM_UP | ARM_DOWN);
    }
    join(frame, gap.x, gap.y.saturating_sub(1), ARM_DOWN);
    join(frame, gap.x, gap.y.saturating_add(gap.height), ARM_UP);
}

/// A divider across a one-cell-tall gap, joined at either end.
fn divider_h(frame: &mut Frame, gap: Rect) {
    for x in gap.x..gap.x.saturating_add(gap.width) {
        draw_line(frame, x, gap.y, ARM_LEFT | ARM_RIGHT);
    }
    join(frame, gap.x.saturating_sub(1), gap.y, ARM_RIGHT);
    join(frame, gap.x.saturating_add(gap.width), gap.y, ARM_LEFT);
}

/// Names a pane, written onto the line directly above it. A pane inside a
/// modal has no frame of its own to hang a title on, so it borrows the one
/// the modal already draws there.
pub fn section_title(frame: &mut Frame, pane: Rect, title: &str) {
    if pane.y == 0 || pane.width == 0 {
        return;
    }
    let width = pane.width as usize;
    let text: String = title.chars().take(width).collect();
    frame
        .buffer_mut()
        .set_string(pane.x, pane.y - 1, text, Style::default());
}

const ARM_UP: u8 = 1;
const ARM_DOWN: u8 = 2;
const ARM_LEFT: u8 = 4;
const ARM_RIGHT: u8 = 8;

/// The box drawing characters a modal is built from, and the arms each reaches
/// out with.
const JOINTS: [(char, u8); 11] = [
    ('\u{2500}', ARM_LEFT | ARM_RIGHT),
    ('\u{2502}', ARM_UP | ARM_DOWN),
    ('\u{250c}', ARM_DOWN | ARM_RIGHT),
    ('\u{2510}', ARM_DOWN | ARM_LEFT),
    ('\u{2514}', ARM_UP | ARM_RIGHT),
    ('\u{2518}', ARM_UP | ARM_LEFT),
    ('\u{251c}', ARM_UP | ARM_DOWN | ARM_RIGHT),
    ('\u{2524}', ARM_UP | ARM_DOWN | ARM_LEFT),
    ('\u{252c}', ARM_DOWN | ARM_LEFT | ARM_RIGHT),
    ('\u{2534}', ARM_UP | ARM_LEFT | ARM_RIGHT),
    ('\u{253c}', ARM_UP | ARM_DOWN | ARM_LEFT | ARM_RIGHT),
];

fn arms_of(symbol: &str) -> Option<u8> {
    let mut chars = symbol.chars();
    let (ch, rest) = (chars.next()?, chars.next());
    if rest.is_some() {
        return None;
    }
    JOINTS
        .iter()
        .find_map(|(joint, arms)| (*joint == ch).then_some(*arms))
}

/// Lays a divider's own cell down, joined to a line already crossing it so a
/// divider meeting another divider reads as a crossing rather than a break.
fn draw_line(frame: &mut Frame, x: u16, y: u16, arms: u8) {
    let existing = frame
        .buffer_mut()
        .cell(Position::new(x, y))
        .and_then(|cell| arms_of(cell.symbol()))
        .unwrap_or(0);
    write_joint(frame, x, y, existing | arms);
}

/// Adds arms to the character in a cell, so a divider meeting the frame reads
/// as a junction rather than one line laid across another. A cell holding
/// anything but box drawing — a title, a word that reaches the edge — keeps
/// what it has.
fn join(frame: &mut Frame, x: u16, y: u16, arms: u8) {
    let Some(existing) = frame
        .buffer_mut()
        .cell(Position::new(x, y))
        .and_then(|cell| arms_of(cell.symbol()))
    else {
        return;
    };
    write_joint(frame, x, y, existing | arms);
}

fn write_joint(frame: &mut Frame, x: u16, y: u16, arms: u8) {
    let Some(joint) = JOINTS
        .iter()
        .find_map(|(joint, candidate)| (*candidate == arms).then_some(*joint))
    else {
        return;
    };
    let Some(cell) = frame.buffer_mut().cell_mut(Position::new(x, y)) else {
        return;
    };
    cell.set_symbol(joint.encode_utf8(&mut [0u8; 4]));
    cell.set_fg(BORDER_COLOR);
}

/// Two soft light trails orbit a modal's frame, and reach along the dividers
/// inside it. Only cells that are frame already get tinted, so titles, counts
/// and any text sitting on an edge keep their own colour and the form beneath
/// stays still. Continuous colour interpolation avoids cell jumps.
///
/// `area` is the modal's whole footprint and `dividers` the one-cell gaps its
/// panes are split by, so a modal built from several rooms is lit as one
/// object: a divider meets the border in the colour the border already has
/// there, leaving no seam at the junction.
pub fn animate_modal_border(clock_ms: u64, area: Rect, dividers: &[Rect], frame: &mut Frame) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let light = Light::new(clock_ms, area);
    let w = u32::from(area.width - 1);
    let h = u32::from(area.height - 1);
    for step in 0..light.perimeter {
        let (x, y) = if step < w {
            (step, 0)
        } else if step < w + h {
            (w, step - w)
        } else if step < 2 * w + h {
            (2 * w + h - step, h)
        } else {
            (0, light.perimeter - step)
        };
        tint(
            frame,
            area.x + x as u16,
            area.y + y as u16,
            color_of(light.on_edge(step)),
        );
    }
    for gap in dividers {
        for y in gap.y..gap.y.saturating_add(gap.height) {
            for x in gap.x..gap.x.saturating_add(gap.width) {
                tint(frame, x, y, color_of(light.inside(x, y)));
            }
        }
    }
}

/// Paints a cell that is frame and leaves any other alone.
fn tint(frame: &mut Frame, x: u16, y: u16, color: Color) {
    let Some(cell) = frame.buffer_mut().cell_mut(Position::new(x, y)) else {
        return;
    };
    if is_frame_cell(cell.symbol()) {
        cell.set_fg(color);
    }
}

/// How brightly the two trails fall on a cell: cyan first, then violet.
type Glow = (f64, f64);

/// The travelling light, as a value every cell of a modal's frame can be
/// asked for. Around the edge it is the position of the two trails; inside,
/// it is the same field carried in from the four edges, which is what keeps a
/// divider and the border it meets the same colour at their junction.
struct Light {
    area: Rect,
    perimeter: u32,
    phase: f64,
}

impl Light {
    /// Only ever built for a modal at least two cells each way, which
    /// `animate_modal_border` checks before it asks for one.
    fn new(clock_ms: u64, area: Rect) -> Self {
        let perimeter = 2 * (u32::from(area.width - 1) + u32::from(area.height - 1));
        Self {
            area,
            perimeter,
            phase: (clock_ms % 4_800) as f64 / 4_800.0,
        }
    }

    /// The light a step around the edge, counted from the top-left corner and
    /// running clockwise.
    fn on_edge(&self, step: u32) -> Glow {
        let position = f64::from(step % self.perimeter) / f64::from(self.perimeter);
        let trail = |offset: f64| {
            let distance = (position - self.phase - offset + 0.5).rem_euclid(1.0) - 0.5;
            (-((distance / 0.085).powi(2))).exp()
        };
        (trail(0.0), trail(0.5))
    }

    /// The light inside the frame, blended from the four edges so that it
    /// meets each of them at exactly the value that edge already has. Two
    /// dividers crossing therefore agree on the cell they share, and a
    /// divider running into the border matches it cell for cell.
    fn inside(&self, x: u16, y: u16) -> Glow {
        let w = u32::from(self.area.width - 1);
        let h = u32::from(self.area.height - 1);
        let (col, row) = (
            u32::from(x.saturating_sub(self.area.x)).min(w),
            u32::from(y.saturating_sub(self.area.y)).min(h),
        );
        let u = f64::from(col) / f64::from(w);
        let v = f64::from(row) / f64::from(h);
        let top = self.on_edge(col);
        let bottom = self.on_edge(2 * w + h - col);
        let left = self.on_edge(self.perimeter - row);
        let right = self.on_edge(w + row);
        let corners = [
            self.on_edge(0),
            self.on_edge(w),
            self.on_edge(2 * w + h),
            self.on_edge(w + h),
        ];
        let blend = |part: fn(Glow) -> f64| {
            let edges =
                (1.0 - v) * part(top) + v * part(bottom) + (1.0 - u) * part(left) + u * part(right);
            let corner = (1.0 - u) * (1.0 - v) * part(corners[0])
                + u * (1.0 - v) * part(corners[1])
                + (1.0 - u) * v * part(corners[2])
                + u * v * part(corners[3]);
            edges - corner
        };
        (blend(|glow| glow.0), blend(|glow| glow.1))
    }
}

fn color_of((cyan, violet): Glow) -> Color {
    Color::Rgb(
        (85.0 + 35.0 * cyan + 150.0 * violet).clamp(0.0, 255.0) as u8,
        (75.0 + 170.0 * cyan + 45.0 * violet).clamp(0.0, 255.0) as u8,
        (135.0 + 115.0 * cyan + 120.0 * violet).clamp(0.0, 255.0) as u8,
    )
}

/// Whether a cell holds box drawing, which is what tells a frame apart from a
/// title or a line of text that reaches the modal's edge.
fn is_frame_cell(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => ('\u{2500}'..='\u{257f}').contains(&ch),
        _ => false,
    }
}

/// Framed block for numbered panels.
/// `n` = panel number shown in title, `focused` controls border colour,
/// `count` = optional `(current, total)` shown bottom-right.
pub fn framed<'a>(
    n: u8,
    title: &'a str,
    focused: bool,
    count: Option<(usize, usize)>,
) -> Block<'a> {
    framed_with_activity(n, title, focused, count, 0, false)
}

pub fn framed_with_activity<'a>(
    n: u8,
    title: &'a str,
    focused: bool,
    count: Option<(usize, usize)>,
    clock_ms: u64,
    active: bool,
) -> Block<'a> {
    // A running job pulses the whole frame, not only the marker in the title:
    // the border is the biggest thing a panel has, so it is what the eye reads
    // as "busy" from across the screen. Idle, the frame holds one accent shade.
    let (border_color, title_style) = if focused {
        let border = if active {
            palette::pulse(clock_ms)
        } else {
            palette::ACCENT
        };
        (
            Style::default().fg(border).add_modifier(Modifier::BOLD),
            Style::default()
                .fg(palette::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            Style::default().fg(palette::FRAME_IDLE),
            Style::default().fg(palette::TEXT_IDLE),
        )
    };

    let title_text = if focused {
        // Focus is already carried by the border colour, so the marker only
        // animates while there is work to report.
        let pulse = if active {
            let tick = (clock_ms / ANIMATION_STEP_MS) as usize;
            SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
        } else {
            "\u{25cf}"
        };
        format!("{pulse} [{n}] {title}")
    } else {
        format!("[{n}] {title}")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_color)
        .title(Span::styled(title_text, title_style));

    if let Some((cur, total)) = count {
        let count_text = format!("{cur} of {total}");
        block.title_bottom(
            Line::from(Span::styled(
                count_text,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ))
            .alignment(Alignment::Right),
        )
    } else {
        block
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::widgets::Widget;

    /// The animation clock reading for a given step of a spinner.
    fn step(tick: u64) -> u64 {
        tick * ANIMATION_STEP_MS
    }

    /// The block's top border, which is where the title and its marker sit.
    fn title_row(tick: u64, active: bool) -> String {
        let area = Rect::new(0, 0, 24, 3);
        let mut buf = Buffer::empty(area);
        framed_with_activity(1, "Status", true, None, step(tick), active).render(area, &mut buf);
        (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect()
    }

    #[test]
    fn the_focus_marker_holds_still_while_nothing_is_running() {
        let first = title_row(0, false);
        assert!(first.contains("[1] Status"), "{first}");
        for tick in 1..8 {
            assert_eq!(
                first,
                title_row(tick, false),
                "an idle panel must not blink"
            );
        }
    }

    #[test]
    fn the_marker_animates_only_while_work_is_running() {
        assert_ne!(
            title_row(0, true),
            title_row(1, true),
            "a running job still shows progress"
        );
    }

    /// The colour of the top-left corner, which is border and nothing else.
    fn corner_color(tick: u64, focused: bool, active: bool) -> Option<Color> {
        let area = Rect::new(0, 0, 24, 3);
        let mut buf = Buffer::empty(area);
        framed_with_activity(1, "Status", focused, None, step(tick), active).render(area, &mut buf);
        buf[(0, 0)].style().fg
    }

    #[test]
    fn a_busy_focused_frame_pulses_and_an_idle_one_holds() {
        let idle = corner_color(0, true, false);
        for tick in 1..16 {
            assert_eq!(idle, corner_color(tick, true, false), "idle frame moved");
        }
        assert!(
            (1..16).any(|tick| corner_color(tick, true, true) != corner_color(0, true, true)),
            "a frame with work in it must visibly pulse"
        );
    }

    #[test]
    fn focus_is_told_apart_from_idle_by_colour() {
        assert_ne!(corner_color(0, true, false), corner_color(0, false, false));
        // Work in an unfocused panel does not make it move.
        assert_eq!(corner_color(0, false, true), corner_color(3, false, true));
    }

    /// Draws into a buffer the way the app does, through a real frame.
    fn drawn(width: u16, height: u16, draw: impl FnOnce(&mut Frame)) -> Buffer {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    /// However many rooms a modal is divided into, it reads as one box: the
    /// dividers meet the frame at a junction rather than laying a second
    /// outline inside the first.
    #[test]
    fn a_modal_with_rooms_is_still_one_box() {
        let area = Rect::new(0, 0, 20, 8);
        let buf = drawn(20, 8, |frame| {
            let inner = modal_frame(frame, area, "Title");
            let (panes, _) = modal_columns(
                frame,
                inner,
                &[Constraint::Percentage(50), Constraint::Percentage(50)],
            );
            modal_rows(
                frame,
                panes[0],
                &[Constraint::Min(1), Constraint::Length(1)],
            );
        });

        assert!(row(&buf, 0).starts_with('\u{250c}'), "{}", row(&buf, 0));
        assert!(row(&buf, 0).ends_with('\u{2510}'), "{}", row(&buf, 0));
        assert!(
            row(&buf, 0).contains('\u{252c}'),
            "the column divider joins the top edge: {}",
            row(&buf, 0)
        );
        // The row divider runs across the left pane only, so it starts on the
        // frame and stops on the column divider.
        let split = (1..7)
            .map(|y| row(&buf, y))
            .find(|line| line.starts_with('\u{251c}'))
            .expect("the row divider joins the left edge");
        assert!(
            split.contains('\u{2524}'),
            "it stops on the column divider: {split}"
        );
        assert!(split.ends_with('\u{2502}'), "{split}");
        assert!(row(&buf, 7).starts_with('\u{2514}'), "{}", row(&buf, 7));
        assert!(
            row(&buf, 7).contains('\u{2534}'),
            "the column divider joins the bottom edge: {}",
            row(&buf, 7)
        );
    }

    /// A pane inside a modal has no box to hang a title on, so its name goes
    /// on the line above it — where the reader looks for a title anyway.
    #[test]
    fn a_pane_is_named_on_the_line_above_it() {
        let area = Rect::new(0, 0, 20, 6);
        let buf = drawn(20, 6, |frame| {
            let inner = modal_frame(frame, area, "Title");
            let (panes, _) = modal_rows(frame, inner, &[Constraint::Length(1), Constraint::Min(1)]);
            section_title(frame, panes[1], "Files");
        });

        assert!(row(&buf, 2).contains("Files"), "{}", row(&buf, 2));
        assert!(row(&buf, 0).contains("Title"), "{}", row(&buf, 0));
    }

    fn channels(buf: &Buffer, x: u16, y: u16) -> [i32; 3] {
        match buf[(x, y)].style().fg {
            Some(Color::Rgb(r, g, b)) => [i32::from(r), i32::from(g), i32::from(b)],
            other => panic!("({x},{y}) was left uncoloured: {other:?}"),
        }
    }

    fn step_between(buf: &Buffer, a: (u16, u16), b: (u16, u16)) -> i32 {
        let (first, second) = (channels(buf, a.0, a.1), channels(buf, b.0, b.1));
        (0..3).map(|i| (first[i] - second[i]).abs()).max().unwrap()
    }

    /// The light runs on into the dividers instead of stopping at them, so
    /// there is no seam where a divider meets the frame: the step in colour
    /// across a junction is no bigger than the step between two neighbouring
    /// cells of the border itself.
    #[test]
    fn the_light_crosses_a_junction_without_a_seam() {
        let area = Rect::new(0, 0, 30, 11);
        let buf = drawn(30, 11, |frame| {
            let inner = modal_frame(frame, area, "Title");
            let (panes, mut dividers) = modal_columns(
                frame,
                inner,
                &[Constraint::Percentage(50), Constraint::Percentage(50)],
            );
            let (_, gaps) = modal_rows(
                frame,
                panes[0],
                &[Constraint::Min(1), Constraint::Length(1)],
            );
            dividers.extend(gaps);
            animate_modal_border(1_500, area, &dividers, frame);
        });

        // The bottom edge is border and nothing else; the top one carries the
        // title, whose cells keep their own colour.
        let bottom = area.height - 1;
        let along_the_border = (0..area.width - 1)
            .map(|x| step_between(&buf, (x, bottom), (x + 1, bottom)))
            .max()
            .unwrap();
        let column = (1..area.width - 1)
            .find(|x| buf[(*x, bottom)].symbol() == "\u{2534}")
            .expect("a column divider stands on the bottom border");
        for y in 1..bottom {
            assert!(
                step_between(&buf, (column, y), (column, y + 1)) <= along_the_border,
                "the light breaks at ({column},{y}) on the way down the divider"
            );
        }
        let crossing = (1..area.height - 1)
            .find(|y| buf[(column, *y)].symbol() == "\u{2524}")
            .expect("the row divider stops on the column divider");
        assert!(
            step_between(&buf, (column, crossing), (column - 1, crossing)) <= along_the_border,
            "the light breaks where the two dividers meet"
        );
        assert!(
            (0..area.width)
                .map(|x| step_between(&buf, (0, bottom), (x, bottom)))
                .max()
                .unwrap()
                > along_the_border,
            "the border is lit unevenly enough for a seam to have shown"
        );
    }

    /// A modal-sized box with a word written across its bottom row, which is
    /// what a modal whose last line is text rather than frame looks like.
    fn modal_buffer(clock_ms: u64) -> Buffer {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        bordered("Title").render(area, &mut buf);
        buf.set_string(0, area.height - 1, "note", Style::default().fg(Color::Red));
        let mut frame_buf = buf.clone();
        {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 5)).unwrap();
            terminal
                .draw(|frame| {
                    *frame.buffer_mut() = frame_buf.clone();
                    animate_modal_border(clock_ms, area, &[], frame);
                    frame_buf = frame.buffer_mut().clone();
                })
                .unwrap();
        }
        frame_buf
    }

    #[test]
    fn the_modal_frame_moves_without_touching_what_is_written_on_it() {
        let first = modal_buffer(0);
        assert!(
            (1..16).any(|tick| modal_buffer(tick * 300)[(0, 2)] != first[(0, 2)]),
            "the frame must travel with elapsed time"
        );
        for tick in 0..16 {
            let buf = modal_buffer(tick * 300);
            for x in 0..20 {
                for y in 0..5 {
                    assert_eq!(buf[(x, y)].symbol(), first[(x, y)].symbol());
                }
            }
            // The title and a line of text reaching the edge keep their colour.
            assert_eq!(buf[(1, 0)].style().fg, first[(1, 0)].style().fg);
            assert_eq!(buf[(0, 4)].style().fg, Some(Color::Red));
        }
    }

    #[test]
    fn an_unfocused_panel_has_no_marker() {
        let area = Rect::new(0, 0, 24, 3);
        let mut buf = Buffer::empty(area);
        framed_with_activity(1, "Status", false, None, 0, false).render(area, &mut buf);
        let row: String = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(row.contains("[1] Status"), "{row}");
        assert!(!row.contains('\u{25cf}'), "{row}");
    }
}
