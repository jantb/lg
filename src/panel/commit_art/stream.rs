//! The token stream flowing through the network into the output.

use super::*;

/// The token in slot `slot` of the stream at time `t`, or none for a gap:
/// the next character of the diff, coloured for the side of the diff it is
/// on, or an abstract glyph when there is no diff to show. Each slot is
/// hashed for its own hue, so the stream is a jumble rather than a pattern,
/// and every token glitters at its own pace. The hue also drifts with time
/// and along the stream, so the whole thing slowly cycles through the
/// rainbow.
pub(super) fn token(feed: &Feed, slot: usize, along: f32, t: f32) -> Option<(char, Style)> {
    let seed = hash(slot, 11);
    if !feed.occupied(slot) {
        return None;
    }
    let (glyph, side) = feed.at(slot).unwrap_or_else(|| {
        let glyph = TOKEN_GLYPHS[((seed >> 8) % TOKEN_GLYPHS.len() as u64) as usize];
        (glyph, Side::Context)
    });
    let own_hue = ((seed >> 16) % 1_000) as f32 / 1_000.0;
    let color = match side {
        Side::Added => mix(Color::Rgb(120, 255, 140), Color::Rgb(40, 200, 90), own_hue),
        Side::Removed => mix(Color::Rgb(255, 120, 130), Color::Rgb(230, 60, 80), own_hue),
        Side::Context => hue(own_hue * 0.35 + along * 0.5 + t * 0.12),
    };
    // Each token twinkles on its own period; the brightest go white-hot.
    let rate = 3.0 + ((seed >> 24) % 50) as f32 / 10.0;
    let phase = ((seed >> 32) % 628) as f32 / 100.0;
    let sparkle = 0.5 + 0.5 * (t * rate + phase).sin();
    let dense = token_fires(feed, slot);
    let color = if sparkle > 0.85 {
        mix(color, Color::Rgb(255, 255, 255), (sparkle - 0.85) * 5.0)
    } else {
        dim(color, 0.55 + 0.45 * sparkle)
    };
    let mut style = Style::default().fg(color);
    if dense || sparkle > 0.85 {
        style = style.add_modifier(Modifier::BOLD);
    }
    Some((glyph, style))
}

/// When slot `slot` reaches the arrowhead at the end of the stream: the slot
/// there at time `t` is `STREAM_ORIGIN + t * STREAM_SPEED + 1`.
pub(super) fn arrival(slot: usize) -> f32 {
    (slot as f32 - STREAM_ORIGIN as f32 - 1.0) / STREAM_SPEED
}

/// Whether the token in `slot` fires a pulse when it reaches the network:
/// only a few do, so the paths through the tree stay distinct.
pub(super) fn token_fires(feed: &Feed, slot: usize) -> bool {
    let seed = hash(slot, 11);
    feed.occupied(slot) && (seed >> 8) % (TOKEN_GLYPHS.len() as u64) < 2
}

/// The stream of tokens flowing right into the root of the network, one
/// line of them ending in an arrowhead at the wire.
pub(super) fn draw_stream(
    canvas: &mut Canvas,
    x: usize,
    y: usize,
    width: usize,
    t: f32,
    feed: &Feed,
) {
    let travelled = (t * STREAM_SPEED) as usize;
    for col in 0..width.saturating_sub(1) {
        let slot = STREAM_ORIGIN + travelled + width - col;
        let along = col as f32 / width.max(1) as f32;
        if let Some((c, style)) = token(feed, slot, along, t) {
            canvas.put(x + col, y, c, style);
        }
    }
    canvas.put(
        x + width - 1,
        y,
        '\u{25b8}',
        Style::default()
            .fg(hue(t * 0.12 + 0.5))
            .add_modifier(Modifier::BOLD),
    );
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
