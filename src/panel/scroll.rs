use ratatui::widgets::ListState;

pub(crate) const EDGE_MARGIN: usize = 3;

pub(crate) fn list_viewport_height(area_height: u16) -> usize {
    area_height.saturating_sub(2) as usize
}

pub(crate) fn selection_scroll_offset(
    selected: Option<usize>,
    len: usize,
    viewport_height: usize,
    current_offset: usize,
) -> usize {
    if len == 0 || viewport_height == 0 {
        return 0;
    }

    let max_offset = len.saturating_sub(viewport_height);
    let Some(selected) = selected.map(|idx| idx.min(len - 1)) else {
        return current_offset.min(max_offset);
    };

    let margin = EDGE_MARGIN.min(viewport_height.saturating_sub(1) / 2);
    let offset = current_offset.min(max_offset);
    let top_edge = offset.saturating_add(margin);
    let bottom_edge = offset
        .saturating_add(viewport_height)
        .saturating_sub(1)
        .saturating_sub(margin);

    if selected < top_edge {
        selected.saturating_sub(margin).min(max_offset)
    } else if selected > bottom_edge {
        selected
            .saturating_add(margin)
            .saturating_add(1)
            .saturating_sub(viewport_height)
            .min(max_offset)
    } else {
        offset
    }
}

/// The rows of a `len`-row list that fit in a pane `area_height` tall when it
/// is scrolled to `offset`: the only ones worth building items for.
pub(crate) fn visible_window(
    len: usize,
    offset: usize,
    area_height: u16,
) -> std::ops::Range<usize> {
    let start = offset.min(len);
    start
        ..start
            .saturating_add(list_viewport_height(area_height))
            .min(len)
}

/// [`list_state`] for a list built from `window` alone: the selection
/// counted from the window's first row, and no offset left to apply.
pub(crate) fn window_list_state(
    selected: Option<usize>,
    window: &std::ops::Range<usize>,
) -> ListState {
    list_state(
        selected
            .filter(|idx| window.contains(idx))
            .map(|idx| idx - window.start),
        0,
    )
}

pub(crate) fn list_state(selected: Option<usize>, offset: usize) -> ListState {
    let mut state = ListState::default();
    state.select(selected);
    *state.offset_mut() = offset;
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_holds_what_fits_from_the_offset() {
        assert_eq!(visible_window(100, 10, 12), 10..20);
        assert_eq!(visible_window(15, 10, 12), 10..15);
        assert_eq!(visible_window(5, 10, 12), 5..5);
        let window = visible_window(100, 10, 12);
        assert_eq!(window_list_state(Some(12), &window).selected(), Some(2));
        assert_eq!(window_list_state(Some(3), &window).selected(), None);
    }

    #[test]
    fn selection_moves_freely_inside_three_row_edges() {
        let mut offset = 0;

        for selected in 0..=6 {
            offset = selection_scroll_offset(Some(selected), 30, 10, offset);
        }
        assert_eq!(offset, 0);

        offset = selection_scroll_offset(Some(7), 30, 10, offset);
        assert_eq!(offset, 1);

        offset = selection_scroll_offset(Some(8), 30, 10, offset);
        assert_eq!(offset, 2);

        offset = selection_scroll_offset(Some(7), 30, 10, offset);
        assert_eq!(offset, 2);

        offset = selection_scroll_offset(Some(4), 30, 10, offset);
        assert_eq!(offset, 1);
    }

    #[test]
    fn selection_scroll_offset_clamps_to_content() {
        assert_eq!(selection_scroll_offset(Some(99), 5, 10, 99), 0);
        assert_eq!(selection_scroll_offset(Some(20), 30, 10, 99), 17);
        assert_eq!(selection_scroll_offset(None, 30, 10, 99), 20);
    }
}
