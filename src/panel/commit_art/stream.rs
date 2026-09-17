//! The token stream flowing through the network into the output.

use super::*;

/// One row of the stream: how far into the diff the code on it is taken
/// from, and what keeps its twinkling apart from the rest. The lane at the
/// wire is the one the network reads, and takes the diff from where it
/// stands; the lanes above and below it are shifted a line back and on from
/// there, so the block of them reads as the code it is.
pub(super) struct Lane {
    pub(super) y: usize,
    pub(super) shift: isize,
    pub(super) salt: usize,
    /// How far this lane is from the wire, 0 at it: the ones out at the
    /// edge of the stream are held back a little.
    pub(super) depth: usize,
}

/// The lanes of a stream `rows` rows tall starting at `y`, with the wire
/// leaving it on row `wire`. Rows above the wire carry the lines before the
/// one at it and rows below the lines after, so the stream reads down the
/// diff the way the file does.
pub(super) fn lanes(feed: &Feed, y: usize, rows: usize, wire: usize) -> Vec<Lane> {
    (y..y + rows)
        .map(|row| {
            let away = row as isize - wire as isize;
            let shift = feed.lane_shift(away.unsigned_abs()) as isize;
            Lane {
                y: row,
                shift: if away < 0 { -shift } else { shift },
                salt: if away == 0 { 0 } else { row.wrapping_mul(977) },
                depth: away.unsigned_abs(),
            }
        })
        .collect()
}

impl Lane {
    /// Where on the tape this lane reads the token in slot `slot` from.
    pub(super) fn pos(&self, slot: usize) -> usize {
        tape_pos(slot, self.shift)
    }
}

/// Where on the tape a lane shifted by `shift` reads slot `slot` from.
///
/// Slots rise towards the network, which is on the right, so a run of text
/// laid out slot by slot would come out mirrored. Taking the tape backwards
/// puts it the right way round for a reader: the code drifts rightwards into
/// the network and reads left to right on the way.
fn tape_pos(slot: usize, shift: isize) -> usize {
    (FEED_ORIGIN as isize - slot as isize + shift).max(0) as usize
}

/// The token in slot `slot` of `lane` at time `t`, or none for a gap: the
/// next character of the diff in the colour the code in it is read in, or an
/// abstract glyph when there is no diff to show. Every token glitters at its
/// own pace, and the lanes away from the wire sit back behind the one on it.
pub(super) fn token(
    feed: &Feed,
    lane: &Lane,
    slot: usize,
    along: f32,
    t: f32,
) -> Option<(char, Style)> {
    let seed = hash(slot.wrapping_add(lane.salt), 11);
    let pos = lane.pos(slot);
    if !feed.occupied(pos, seed) {
        return None;
    }
    let own_hue = ((seed >> 16) % 1_000) as f32 / 1_000.0;
    let (glyph, color) = feed.at(pos).unwrap_or_else(|| {
        let glyph = TOKEN_GLYPHS[((seed >> 8) % TOKEN_GLYPHS.len() as u64) as usize];
        // With no diff to show the stream cycles through the rainbow, the
        // hue drifting with time and along the stream.
        (glyph, hue(own_hue * 0.35 + along * 0.5 + t * 0.12))
    });
    // Each token twinkles on its own period; the brightest go white-hot.
    // The code has to stay legible through it, so the twinkle lifts the
    // colour rather than taking it away.
    let rate = 3.0 + ((seed >> 24) % 50) as f32 / 10.0;
    let phase = ((seed >> 32) % 628) as f32 / 100.0;
    let sparkle = 0.5 + 0.5 * (t * rate + phase).sin();
    let dense = lane.depth == 0 && token_fires(feed, slot);
    let color = if sparkle > 0.9 {
        mix(color, Color::Rgb(255, 255, 255), (sparkle - 0.9) * 10.0)
    } else {
        dim(color, 0.78 + 0.22 * sparkle)
    };
    let mut style = Style::default().fg(dim(color, 1.0 - 0.16 * lane.depth as f32));
    if dense || sparkle > 0.9 {
        style = style.add_modifier(Modifier::BOLD);
    }
    Some((glyph, style))
}

/// When slot `slot` reaches the end of the stream, where the lanes are
/// gathered: the slot there at time `t` is
/// `STREAM_ORIGIN + t * STREAM_SPEED + 1`.
pub(super) fn arrival(slot: usize) -> f32 {
    (slot as f32 - STREAM_ORIGIN as f32 - 1.0) / STREAM_SPEED
}

/// Whether the token in `slot` fires a pulse when it reaches the network:
/// only a few do, so the paths through the tree stay distinct. The network
/// reads the lane the wire leaves on, which is the unshifted one.
pub(super) fn token_fires(feed: &Feed, slot: usize) -> bool {
    let seed = hash(slot, 11);
    feed.occupied(tape_pos(slot, 0), seed) && (seed >> 8) % (TOKEN_GLYPHS.len() as u64) < 2
}

/// The stream of code flowing right into the root of the network: several
/// lines of the diff at once, one to a lane, the one at the wire running on
/// into the tree. A bracket at the end gathers the lanes onto that row,
/// which is where the tokens leave the stream for the network.
pub(super) fn draw_stream(
    canvas: &mut Canvas,
    x: usize,
    width: usize,
    lanes: &[Lane],
    wire: usize,
    t: f32,
    feed: &Feed,
) {
    let travelled = (t * STREAM_SPEED) as usize;
    let text_w = width.saturating_sub(1);
    for lane in lanes {
        for col in 0..text_w {
            let slot = STREAM_ORIGIN + travelled + width - col;
            let along = col as f32 / width.max(1) as f32;
            if let Some((c, style)) = token(feed, lane, slot, along, t) {
                canvas.put(x + col, lane.y, c, style);
            }
        }
    }
    draw_gather(canvas, x + text_w, lanes, wire, t);
}

/// The bracket the lanes are gathered into at the end of the stream, an
/// arrowhead when the stream is a single lane.
fn draw_gather(canvas: &mut Canvas, x: usize, lanes: &[Lane], wire: usize, t: f32) {
    let style = Style::default()
        .fg(hue(t * 0.12 + 0.5))
        .add_modifier(Modifier::BOLD);
    let (Some(first), Some(last)) = (lanes.first(), lanes.last()) else {
        return;
    };
    let (top, bottom) = (first.y, last.y);
    if top == bottom {
        canvas.put(x, top, '\u{25b8}', style);
        return;
    }
    for y in top..=bottom {
        // Only the wire's own row reaches on towards the network; the rest
        // of the bracket is the column that gathers the lanes onto it.
        let glyph = match (y == wire, y == top, y == bottom) {
            (true, true, _) => '\u{256d}',
            (true, _, true) => '\u{2570}',
            (true, ..) => '\u{251c}',
            _ => '\u{2502}',
        };
        canvas.put(x, y, glyph, style);
    }
}

/// Slots are counted from here so the arithmetic never goes below zero.
pub(super) const STREAM_ORIGIN: usize = 1_000_000;
/// Seconds a pulse takes to cross one level of the tree.
pub(super) const PULSE_LEVEL_TIME: f32 = 0.28;

/// The network at one moment: the tree with the pulses running through it.
/// Every dense token that has reached the end of the stream lately is a
/// pulse somewhere in the tree, following a path of its own from the root to
/// one leaf, so different branches light up as the tokens go in.
pub(super) struct Network<'a> {
    pub(super) tree: &'a Tree,
    pub(super) y: usize,
    /// How hot each node is, 0..=1.
    pub(super) heat: Vec<f32>,
    /// How hot the branch into each node is.
    pub(super) branches: Vec<f32>,
    /// Tokens that have come all the way through so far.
    pub(super) landed: usize,
    /// How recently the last one landed, 1 at the moment it does, fading.
    pub(super) landing: f32,
}

impl<'a> Network<'a> {
    pub(super) fn new(tree: &'a Tree, y: usize, t: f32, feed: &Feed) -> Self {
        let mut heat = vec![0.0f32; tree.nodes.len()];
        let mut branches = vec![0.0f32; tree.nodes.len()];
        let travel = tree.levels as f32 * PULSE_LEVEL_TIME;
        let travelled = (t * STREAM_SPEED) as usize;
        // The slot at the arrowhead right now, and the ones just before it
        // that may still be crossing the tree.
        let newest = STREAM_ORIGIN + travelled + 1;
        let in_flight = (travel * STREAM_SPEED) as usize + 2;
        let mut landed = 0;
        let mut landing = 0.0f32;
        for slot in newest.saturating_sub(in_flight)..=newest {
            if !token_fires(feed, slot) {
                continue;
            }
            let age = t - arrival(slot);
            if age < 0.0 {
                continue;
            }
            if age > travel {
                landed += 1;
                landing = landing.max(1.0 - (age - travel) / 0.5);
                continue;
            }
            let position = age / PULSE_LEVEL_TIME;
            let mut node = 0;
            loop {
                let level = tree.nodes[node].level as f32;
                let glow = (1.0 - (position - level).abs()).clamp(0.0, 1.0);
                heat[node] = heat[node].max(glow);
                let children = &tree.nodes[node].children;
                if children.is_empty() {
                    break;
                }
                // Which branch, decided by the token itself.
                let pick = (hash(slot, tree.nodes[node].level + 40) >> 8) as usize % children.len();
                node = children[pick];
                let crossing = (1.0 - (position - level - 0.5).abs() / 0.7).clamp(0.0, 1.0);
                branches[node] = branches[node].max(crossing);
            }
        }
        // Everything that landed before the window counts too.
        landed += newest
            .saturating_sub(in_flight)
            .saturating_sub(STREAM_ORIGIN)
            / 6;
        Self {
            tree,
            y,
            heat,
            branches,
            landed,
            landing,
        }
    }

    pub(super) fn root_heat(&self) -> f32 {
        self.heat[0]
    }

    pub(super) fn draw(&self, canvas: &mut Canvas) {
        let cold = Color::Rgb(100, 100, 135);
        let ember = Color::Rgb(255, 120, 70);
        let hot = Color::Rgb(255, 250, 170);
        let branch_cold = Color::Rgb(70, 70, 100);
        let tree = self.tree;
        let style = |k: f32| {
            let mut st = Style::default().fg(mix(branch_cold, hot, k));
            if k > 0.3 {
                st = st.add_modifier(Modifier::BOLD);
            }
            st
        };
        // Branches first, nodes over them. Each parent runs a stub right to
        // an elbow column, a bar there spans its children's rows, and each
        // child gets a stub from the bar; the bar's cells are joined with
        // whichever corner or tee the lines through them call for.
        for node in &tree.nodes {
            if node.children.is_empty() {
                continue;
            }
            let px = tree.col(node.level);
            let elbow = px + tree.level_step / 2;
            let cx = tree.col(node.level + 1);
            let py = self.y + node.row;
            let rows: Vec<(usize, f32)> = node
                .children
                .iter()
                .map(|&c| (self.y + tree.nodes[c].row, self.branches[c]))
                .collect();
            let stub = rows.iter().map(|r| r.1).fold(0.0, f32::max);
            for x in px + 1..elbow {
                canvas.put(x, py, '\u{2500}', style(stub));
            }
            let lo = rows.iter().map(|r| r.0).min().unwrap_or(py).min(py);
            let hi = rows.iter().map(|r| r.0).max().unwrap_or(py).max(py);
            for yy in lo..=hi {
                let up = yy > lo;
                let down = yy < hi;
                let left = yy == py;
                let child = rows.iter().find(|r| r.0 == yy);
                let right = child.is_some();
                let glyph = match (up, down, left, right) {
                    (false, true, false, true) => '\u{256d}',
                    (true, false, false, true) => '\u{2570}',
                    (true, true, false, true) => '\u{251c}',
                    (true, true, true, false) => '\u{2524}',
                    (true, true, true, true) => '\u{253c}',
                    (false, true, true, false) => '\u{256e}',
                    (true, false, true, false) => '\u{256f}',
                    (false, true, true, true) => '\u{252c}',
                    (true, false, true, true) => '\u{2534}',
                    (true, true, false, false) => '\u{2502}',
                    _ => '\u{2500}',
                };
                // A cell of the bar is as hot as the hottest branch running
                // through it: one whose child row is on the far side of it.
                let k = rows
                    .iter()
                    .filter(|r| (r.0.min(py)..=r.0.max(py)).contains(&yy))
                    .map(|r| r.1)
                    .fold(0.0, f32::max);
                canvas.put(elbow, yy, glyph, style(k));
                if let Some(&(_, kh)) = child {
                    for x in elbow + 1..cx {
                        canvas.put(x, yy, '\u{2500}', style(kh));
                    }
                }
            }
        }
        for (id, node) in tree.nodes.iter().enumerate() {
            let heat = self.heat[id];
            let leaf = node.children.is_empty();
            let (glyph, color) = if heat > 0.75 {
                ('\u{25c9}', mix(ember, hot, (heat - 0.75) * 4.0))
            } else if heat > 0.3 {
                ('\u{25cf}', mix(cold, ember, (heat - 0.3) / 0.45))
            } else if leaf {
                ('\u{25c7}', cold)
            } else {
                ('\u{25cb}', cold)
            };
            let mut style = Style::default().fg(color);
            if heat > 0.75 {
                style = style.add_modifier(Modifier::BOLD);
            }
            canvas.put(tree.col(node.level), self.y + node.row, glyph, style);
        }
    }
}

/// Tokens through the network per character of output.
pub(super) const TOKENS_PER_CHAR: usize = 2;

/// The start of the commit message the network is writing, one character
/// per couple of tokens through it, with a cursor blinking at the end. The
/// newest character flares as the pulse that wrote it lands. Only the
/// opening is guessable before the model has spoken; the real message takes
/// this line's place the moment it does.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_output(
    canvas: &mut Canvas,
    lang: Language,
    x: usize,
    y: usize,
    room: usize,
    landed: usize,
    landing: f32,
    t: f32,
) {
    let text = lang.opening();
    let typed = (landed / TOKENS_PER_CHAR).min(text.chars().count());
    let shown: String = text.chars().take(typed).collect();
    let plain = Color::Rgb(220, 220, 240);
    let style = Style::default().fg(plain).add_modifier(Modifier::BOLD);
    for (i, c) in shown.chars().enumerate().take(room.saturating_sub(1)) {
        let st = if i + 1 == typed {
            Style::default()
                .fg(mix(plain, Color::Rgb(255, 200, 120), landing))
                .add_modifier(Modifier::BOLD)
        } else {
            style
        };
        canvas.put(x + i, y, c, st);
    }
    if ((t * 3.0).sin() > 0.0 || landing > 0.5) && typed < room {
        canvas.put(x + typed, y, '\u{258c}', style);
    }
}
