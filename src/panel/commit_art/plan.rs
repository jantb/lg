//! Fitting the mascot, pipeline and tree into the box the scene was given.

use super::*;

/// Where the parts go in a box of one size. The backdrop fills the top down
/// to the ground line; under it is a black band with the mascot at the left,
/// a stream of tokens running from it across the middle into the root of the
/// network at the right, and the message the network is writing under the
/// network; the caption sits at the very bottom.
pub(super) struct Plan {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) mascot_w: usize,
    pub(super) mascot_h: usize,
    /// The row the backdrop stops at; the band starts on the row below.
    pub(super) ground: usize,
    /// The stream and network, if there is room for them beside the mascot.
    pub(super) pipeline: Option<Pipeline>,
    pub(super) caption: bool,
}

#[derive(Debug, Clone)]
pub(super) struct Pipeline {
    /// Where the stream starts and how wide it is.
    pub(super) stream_x: usize,
    pub(super) stream_w: usize,
    pub(super) tree: Tree,
    /// Top row of the tree.
    pub(super) tree_y: usize,
    /// The row the network writes its output on, when there is one to spare.
    pub(super) output_row: Option<usize>,
}

/// One node of the network.
#[derive(Debug, Clone)]
pub(super) struct Node {
    pub(super) level: usize,
    /// Row relative to the top of the tree.
    pub(super) row: usize,
    pub(super) children: Vec<usize>,
}

/// The network's shape: an irregular tree, grown from a seed. Each node has
/// one to three branches, some stop short, and no two waits grow the same
/// one. Leaves take the rows in order; a node sits in the middle of the rows
/// its leaves take.
#[derive(Debug, Clone)]
pub(super) struct Tree {
    pub(super) nodes: Vec<Node>,
    pub(super) levels: usize,
    /// Column of the root, and cells between levels.
    pub(super) x: usize,
    pub(super) level_step: usize,
    pub(super) height: usize,
}

impl Tree {
    /// Grow a tree with at most `levels` levels whose leaves, `leaf_step`
    /// rows apart, fit in `max_height` rows. `None` if not even a root and
    /// two leaves fit.
    pub(super) fn grow(
        seed: usize,
        levels: usize,
        leaf_step: usize,
        max_height: usize,
    ) -> Option<Self> {
        let max_leaves = max_height.checked_sub(1)? / leaf_step + 1;
        if max_leaves < 2 || levels < 2 {
            return None;
        }
        let mut nodes = vec![Node {
            level: 0,
            row: 0,
            children: Vec::new(),
        }];
        // Leaves still allowed beyond the one every node already is.
        let mut spare = max_leaves - 1;
        Self::branch(&mut nodes, 0, seed, levels, &mut spare);
        let mut next_row = 0;
        Self::place(&mut nodes, 0, leaf_step, &mut next_row);
        let height = nodes.iter().map(|n| n.row).max().unwrap_or(0) + 1;
        Some(Self {
            nodes,
            levels,
            x: 0,
            level_step: LEVEL_STEP,
            height,
        })
    }

    /// Give node `id` its children and grow them in turn, within the leaf
    /// budget.
    pub(super) fn branch(
        nodes: &mut Vec<Node>,
        id: usize,
        seed: usize,
        levels: usize,
        spare: &mut usize,
    ) {
        let level = nodes[id].level;
        if level + 1 >= levels || *spare == 0 {
            return;
        }
        let h = hash(seed.wrapping_mul(7919).wrapping_add(id), 5);
        let mut want = match h % 10 {
            0..=1 => 1,
            2..=7 => 2,
            _ => 3,
        };
        // Some branches stop short of the last level; the root never does,
        // and always forks.
        if level == 0 {
            want = want.max(2);
        } else if level >= 2 && (h >> 8).is_multiple_of(5) {
            return;
        }
        let want = want.min(*spare + 1);
        *spare -= want - 1;
        let first = nodes.len();
        for _ in 0..want {
            nodes.push(Node {
                level: level + 1,
                row: 0,
                children: Vec::new(),
            });
        }
        nodes[id].children = (first..first + want).collect();
        for child in first..first + want {
            Self::branch(nodes, child, seed, levels, spare);
        }
    }

    /// Rows: leaves in order, each node in the middle of its leaves.
    pub(super) fn place(nodes: &mut [Node], id: usize, leaf_step: usize, next_row: &mut usize) {
        let children = nodes[id].children.clone();
        if children.is_empty() {
            nodes[id].row = *next_row;
            *next_row += leaf_step;
            return;
        }
        for &c in &children {
            Self::place(nodes, c, leaf_step, next_row);
        }
        let first = nodes[children[0]].row;
        let last = nodes[*children.last().unwrap_or(&children[0])].row;
        nodes[id].row = (first + last) / 2;
    }

    pub(super) fn width(&self) -> usize {
        (self.levels - 1) * self.level_step + 1
    }

    pub(super) fn col(&self, level: usize) -> usize {
        self.x + level * self.level_step
    }
}

/// Cells between the end of the stream and the root.
pub(super) const WIRE_GAP: usize = 3;
/// Rows of backdrop the band leaves above itself when the box allows.
pub(super) const MIN_SKY: usize = 5;

impl Plan {
    /// The figure's box is a little over twice as wide as tall in cells, as
    /// cells are twice as tall as they are wide: square on screen.
    pub(super) fn mascot_width(rows: usize) -> usize {
        rows * 2 + 4
    }

    pub(super) fn fit(lang: Language, seed: usize, width: usize, height: usize) -> Option<Self> {
        let caption_w = CAPTIONS
            .iter()
            .chain(std::iter::once(&lang.quip()))
            .map(|text| text.chars().count() + 3)
            .max()
            .unwrap_or(0);
        let caption = caption_w <= width;
        // Ground line above the band, blank and caption below it.
        let below = 1 + if caption { 2 } else { 0 };
        let usable = height.checked_sub(below)?;
        if usable < FIGURE_MIN_ROWS {
            return None;
        }
        // The band is as tall as the figure, which takes what it can while
        // leaving some sky, and shrinks to the width if it must.
        let mut mascot_h = usable
            .saturating_sub(MIN_SKY)
            .clamp(FIGURE_MIN_ROWS, FIGURE_MAX_ROWS.min(usable));
        while Self::mascot_width(mascot_h) > width {
            mascot_h -= 1;
            if mascot_h < FIGURE_MIN_ROWS {
                return None;
            }
        }
        let mascot_w = Self::mascot_width(mascot_h);
        let ground = height - below - mascot_h;
        let band_y = ground + 1;
        // The biggest tree the band holds, spread out if it can be; ideally
        // with a row left under it for the output. The stream takes only
        // what it needs of the width; the tree gets the rest, level by level.
        let stream_x = mascot_w + 1;
        // A box too narrow for the pipeline keeps the mascot on its own.
        let for_tree = width
            .checked_sub(stream_x + MIN_STREAM_WIDTH + WIRE_GAP + NETWORK_MARGIN)
            .and_then(|w| w.checked_sub(1));
        let pipeline = [2, 0].into_iter().find_map(|spare| {
            let for_tree = for_tree?;
            let band = mascot_h.checked_sub(spare)?;
            // Levels the width allows, and no more than the leaves can
            // fill: a tall thin tree with one leaf a level is a stick.
            let max_leaves = (band - 1) / LEAF_STEP + 1;
            let most_levels = (for_tree / LEVEL_STEP + 1)
                .min(TREE_LEVELS)
                .min((max_leaves / 2).max(TREE_MIN_LEVELS));
            (TREE_MIN_LEVELS..=most_levels)
                .rev()
                .flat_map(|levels| [(levels, LEAF_STEP), (levels, 1)])
                .find_map(|(levels, leaf_step)| {
                    let mut tree = Tree::grow(seed, levels, leaf_step, band)?;
                    // A tree that could not fork enough to use its levels
                    // is no better than a smaller one; let that be tried.
                    if tree.nodes.iter().map(|n| n.level).max().unwrap_or(0) + 1 < levels {
                        return None;
                    }
                    // Spread the levels to fill what the stream does not
                    // need, then give the stream the rest.
                    let widest = width
                        .checked_sub(stream_x + MAX_STREAM_WIDTH + WIRE_GAP + NETWORK_MARGIN + 1)?;
                    tree.level_step = (widest / (levels - 1)).clamp(LEVEL_STEP, MAX_LEVEL_STEP);
                    tree.x = width.checked_sub(tree.width() + NETWORK_MARGIN)?;
                    let stream_w = tree.x.checked_sub(stream_x + WIRE_GAP)?;
                    if stream_w < MIN_STREAM_WIDTH {
                        return None;
                    }
                    // The stream leaves the figure a little below its
                    // middle, where its body is, so the root sits there and
                    // the tree hangs off it as far as the band allows.
                    let root_target = band_y + mascot_h * 3 / 5;
                    let lowest = band_y + mascot_h - spare - tree.height;
                    let tree_y = root_target
                        .saturating_sub(tree.nodes[0].row)
                        .clamp(band_y, lowest);
                    Some(Pipeline {
                        stream_x,
                        stream_w,
                        tree,
                        tree_y,
                        output_row: (spare > 0).then_some(band_y + mascot_h - 1),
                    })
                })
        });
        Some(Self {
            width,
            height,
            mascot_w,
            mascot_h,
            ground,
            pipeline,
            caption,
        })
    }
}
