mod chrome;
mod diff;
mod layout;
mod log;

pub use chrome::{bordered, framed, framed_with_activity};
pub use diff::{
    highlight_diff_line, highlight_diff_line_for_path, highlight_diff_text,
    highlight_source_line_for_path,
};
pub use layout::{
    LEFT_PANEL_COUNT, LayoutRects, LeftPanelHeights, centered, clamp_left_column_width,
    default_left_panel_heights, left_panel_min_height, normalize_left_panel_heights, split_layout,
    split_layout_with_environments, split_layout_with_sizes, split_layout_with_width, split_main,
};
pub use log::highlight_log_line;
