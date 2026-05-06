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
