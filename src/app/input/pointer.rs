//! Routing one mouse event: modals first, then the panes.

use super::*;

pub(super) fn handle_modal_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) -> bool {
    match state.modal {
        Modal::None => false,
        Modal::Help => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            match m.kind {
                MouseEventKind::ScrollDown => panel::help::scroll(state, area, true, 3),
                MouseEventKind::ScrollUp => panel::help::scroll(state, area, false, 3),
                _ => {}
            }
            true
        }
        Modal::StatusDetails => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            match m.kind {
                MouseEventKind::ScrollDown => panel::details::scroll(state, area, true, 3),
                MouseEventKind::ScrollUp => panel::details::scroll(state, area, false, 3),
                // A click outside the popup puts it away.
                MouseEventKind::Down(MouseButton::Left)
                    if !rect_contains(panel::details::popup_area(area), m.column, m.row) =>
                {
                    state.modal = Modal::None;
                }
                _ => {}
            }
            true
        }
        Modal::Conflict => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            panel::conflict::handle_mouse(state, area, m);
            true
        }
        Modal::Settings => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            panel::settings::handle_mouse(state, area, m);
            true
        }
        Modal::Environments => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            panel::deployment::handle_mouse(state, area, m);
            true
        }
        Modal::GitHub => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            panel::github::handle_mouse(state, m);
            true
        }
        Modal::Commit => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            let over_files = rect_contains(panel::commit::staged_area(area), m.column, m.row);
            let inside = rect_contains(panel::commit::modal_area(area), m.column, m.row);
            match m.kind {
                // A click beside the modal puts it away; a generation in
                // flight carries on and is listed under its checkout.
                MouseEventKind::Down(MouseButton::Left) if !inside => {
                    if state.generating() {
                        state.set_status(panel::commit::BACKGROUND_NOTICE, false);
                    }
                    state.modal = Modal::None;
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    let _ = panel::commit::place_cursor_at(state, area, m.column, m.row);
                }
                MouseEventKind::ScrollDown if over_files => {
                    panel::commit::scroll_files(state, area, true, 3);
                }
                MouseEventKind::ScrollUp if over_files => {
                    panel::commit::scroll_files(state, area, false, 3);
                }
                _ => {}
            }
            true
        }
        modal => {
            state.column_drag_active = false;
            state.row_drag_active = None;
            // Each of these hit-tests against the layout it draws; a modal
            // with nothing to click or scroll takes the event and does nothing.
            let handled = match modal {
                Modal::GuidedReview => {
                    panel::guided::handle_mouse(state, area, m);
                    Ok(())
                }
                Modal::Commands => {
                    panel::commands::handle_mouse(state, area, m);
                    Ok(())
                }
                Modal::Flow => panel::flow::handle_mouse(state, area, m),
                Modal::Agent => panel::agent::handle_mouse(state, area, m),
                Modal::DeleteBranch => panel::delete_branch::handle_mouse(state, area, m),
                Modal::ConfirmDestructive => panel::confirm::handle_mouse(state, area, m),
                Modal::Push => panel::push::handle_mouse(state, area, m),
                Modal::Stash => panel::stash::handle_mouse(state, area, m),
                _ => Ok(()),
            };
            if let Err(err) = handled {
                state.set_status(format!("{err:#}"), true);
            }
            true
        }
    }
}

pub(super) fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

/// Route one mouse event. Modals get it first, then the pane dividers, then
/// whichever pane it landed in.
pub(super) fn dispatch_mouse<H: AppHost>(host: &mut H, m: MouseEvent) -> Result<()> {
    let area = host.area()?;
    if handle_modal_mouse(host.state_mut(), area, &m) {
        return Ok(());
    }

    let rects = crate::app::render::layout_for(host.state(), area);
    let divider_col = rects.main.x.saturating_sub(1);
    let on_divider = m.row >= rects.status.y
        && m.row < rects.footer.y
        && (m.column == divider_col || m.column == rects.main.x);

    match m.kind {
        MouseEventKind::Down(MouseButton::Left)
            if on_divider && !m.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            let width = ui::clamp_left_column_width(
                rects.status.width.saturating_add(rects.main.width),
                m.column.saturating_sub(area.x).saturating_add(1),
            );
            let state = host.state_mut();
            state.column_drag_active = true;
            state.row_drag_active = None;
            state.left_column_width = Some(width);
            return Ok(());
        }
        MouseEventKind::Drag(MouseButton::Left) if host.state().column_drag_active => {
            let width = ui::clamp_left_column_width(
                rects.status.width.saturating_add(rects.main.width),
                m.column.saturating_sub(area.x).saturating_add(1),
            );
            host.state_mut().left_column_width = Some(width);
            return Ok(());
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let state = host.state_mut();
            state.column_drag_active = false;
            state.row_drag_active = None;
            state.selection = state.selection.take().and_then(ui::TextSelection::release);
            return Ok(());
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            host.state_mut().selection = None;
        }
        _ => {}
    }

    let show_environments = host.state().environments_visible();
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if !m.modifiers.contains(KeyModifiers::SHIFT) => {
            if let Some(pair) =
                mouse::row_divider_pair_at(&rects, show_environments, m.column, m.row)
            {
                let heights = mouse::current_left_panel_heights(&rects);
                let state = host.state_mut();
                state.column_drag_active = false;
                state.row_drag_active = Some(pair);
                state.left_panel_heights = Some(heights);
                mouse::resize_left_panel_pair(
                    host.state_mut(),
                    &rects,
                    pair,
                    m.row,
                    show_environments,
                );
                return Ok(());
            }

            if let Some(pane) = mouse::pane_at(&rects, m.column, m.row) {
                let commit_ref_before = selected_commit_ref(host.state());
                host.state_mut().selection = mouse::pane_rect_at(&rects, m.column, m.row)
                    .and_then(|pane| ui::TextSelection::start(pane, m.column, m.row));
                host.state_mut().focus = pane;
                mouse::select_mouse_row(host.state_mut(), pane, &rects, m.row);
                if !matches!(pane, Pane::Main) {
                    host.diff_for_focus();
                }
                if selected_commit_ref(host.state()) != commit_ref_before {
                    host.sync_commit_log();
                }
                return Ok(());
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(pair) = host.state().row_drag_active {
                mouse::resize_left_panel_pair(
                    host.state_mut(),
                    &rects,
                    pair,
                    m.row,
                    show_environments,
                );
                return Ok(());
            }
            if let Some(selection) = host
                .state_mut()
                .selection
                .as_mut()
                .filter(|selection| selection.dragging)
            {
                selection.extend(m.column, m.row);
                return Ok(());
            }
        }
        _ => {}
    }

    if matches!(
        m.kind,
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
    ) && let Some(pane @ (Pane::Status | Pane::Files | Pane::Branches | Pane::Commits)) =
        mouse::pane_at(&rects, m.column, m.row)
    {
        let focus_before = host.state().focus;
        let commit_ref_before = selected_commit_ref(host.state());
        host.state_mut().focus = pane;
        let changed = mouse::scroll_list(
            host.state_mut(),
            pane,
            matches!(m.kind, MouseEventKind::ScrollDown),
            3,
        );
        if changed || focus_before != pane {
            host.diff_for_focus();
        }
        if selected_commit_ref(host.state()) != commit_ref_before {
            host.sync_commit_log();
        }
        return Ok(());
    }

    let in_main = m.column >= rects.main.x
        && m.column < rects.main.x + rects.main.width
        && m.row >= rects.main.y
        && m.row < rects.main.y + rects.main.height;
    if !in_main {
        return Ok(());
    }
    // Cells inside the pane's border, 1-based, which is how a program counts
    // its own screen.
    let column = m.column.saturating_sub(rects.main.x);
    let row = m.row.saturating_sub(rects.main.y);
    match m.kind {
        MouseEventKind::ScrollDown => {
            panel::main::wheel_at(host.state_mut(), true, 3, column, row);
        }
        MouseEventKind::ScrollUp => {
            panel::main::wheel_at(host.state_mut(), false, 3, column, row);
        }
        _ => {}
    }
    Ok(())
}
