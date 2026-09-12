//! Routing one key press to the modal, the session or the focused pane.

use super::*;

/// A jump through a list pane: to either end, half a page, or a whole page.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Jump {
    Top,
    Bottom,
    HalfDown,
    HalfUp,
    PageDown,
    PageUp,
}

pub(super) fn jump_for(k: KeyEvent) -> Option<Jump> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Char('g') if !ctrl => Some(Jump::Top),
        KeyCode::Char('G') if !ctrl => Some(Jump::Bottom),
        KeyCode::Char('d') if ctrl => Some(Jump::HalfDown),
        KeyCode::Char('u') if ctrl => Some(Jump::HalfUp),
        KeyCode::PageDown => Some(Jump::PageDown),
        KeyCode::PageUp => Some(Jump::PageUp),
        _ => None,
    }
}

/// Rows the list in `pane` can show at once.
pub(super) fn list_page<H: AppHost>(host: &H, pane: Pane) -> Result<usize> {
    let area = host.area()?;
    let rects = crate::app::render::layout_for(host.state(), area);
    let rect = match pane {
        // The repository tree is the list the Status pane scrolls.
        Pane::Status => rects.environments,
        Pane::Files => rects.files,
        Pane::Branches => rects.branches,
        Pane::Commits => rects.commits,
        Pane::Main => rects.main,
    };
    Ok(crate::panel::scroll::list_viewport_height(rect.height).max(1))
}

/// Move the selection in a list pane by a jump key, and say whether one was
/// pressed.
///
/// This sits in front of the pane handlers rather than inside each of them so
/// that all four lists jump the same way, and so Ctrl-d and Ctrl-u cannot fall
/// through to the plain `d` and `u` beneath them — which, in Files, are delete
/// and unstage.
pub(super) fn list_jump<H: AppHost>(host: &mut H, pane: Pane, k: KeyEvent) -> Result<bool> {
    // The diff pane scrolls text, not a list, and handles these keys itself.
    if pane == Pane::Main {
        return Ok(false);
    }
    let Some(jump) = jump_for(k) else {
        return Ok(false);
    };
    let (down, amount) = match jump {
        Jump::Top => (false, usize::MAX),
        Jump::Bottom => (true, usize::MAX),
        Jump::HalfDown => (true, (list_page(host, pane)? / 2).max(1)),
        Jump::HalfUp => (false, (list_page(host, pane)? / 2).max(1)),
        Jump::PageDown => (true, list_page(host, pane)?),
        Jump::PageUp => (false, list_page(host, pane)?),
    };
    crate::app::mouse::scroll_list(host.state_mut(), pane, down, amount);
    Ok(true)
}

/// Say so when a key does nothing here. lg has enough per-pane keys that a
/// silent no-op reads as a bug; naming the pane and pointing at the help turns
/// a dead end into the answer.
pub(super) fn report_unbound(state: &mut AppState, pane: Pane, k: KeyEvent) {
    let KeyCode::Char(c) = k.code else {
        return;
    };
    // Modified keys are the terminal's business as often as lg's, and a bare
    // space reads as nothing at all when quoted back.
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        || c.is_whitespace()
    {
        return;
    }
    let where_it_is = panel::keys::active_section(pane, state.main_keys())
        .map_or("this pane", panel::keys::footer_label);
    state.set_status(
        format!("no binding for '{c}' in {where_it_is} \u{2014} ? for help"),
        false,
    );
}

/// Route one key press. Modals get it first, then lg's global bindings, then the
/// focused pane.
pub(super) fn dispatch_key<H: AppHost>(host: &mut H, k: KeyEvent) -> Result<()> {
    // A terminal spelling keys out can also report them being let go. Acting on
    // both halves would type everything twice.
    if k.kind == KeyEventKind::Release {
        return Ok(());
    }
    // A selection is screen cells, and a key is about to move what is under
    // them; it has either been copied by now or was not wanted.
    host.state_mut().selection = None;
    // A session holding the keyboard sees everything, Ctrl-C included —
    // interrupting the program inside it matters more than quitting lg, which
    // Ctrl-] then q still does.
    if host.state().session_input_active() {
        if crate::app::session::is_release_key(&k) {
            host.release_session_keyboard();
            return Ok(());
        }
        crate::app::session::forward_key(host.state_mut(), k);
        return Ok(());
    }
    if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
        host.state_mut().request_quit();
        return Ok(());
    }

    match host.state().modal {
        Modal::Help => {
            let area = host.area()?;
            panel::help::handle_key(host.state_mut(), k, area)?;
            return Ok(());
        }
        Modal::Commit => {
            panel::commit::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::StageAllBeforeCommit => {
            panel::stage_all::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Push => {
            panel::push::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Author => {
            panel::author::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Commands => {
            if let Some(key) = panel::commands::handle_key(host.state_mut(), k) {
                return dispatch_key(host, key);
            }
            return Ok(());
        }
        Modal::Environments => {
            panel::deployment::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Settings => {
            panel::settings::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Model => {
            panel::model::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Flow => {
            panel::flow::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::RepoActions => {
            panel::environments::menu::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Agent => {
            panel::agent::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Conflict => {
            panel::conflict::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::DeleteBranch => {
            panel::delete_branch::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::Worktree => {
            panel::worktree::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::ConfirmDestructive => {
            panel::confirm::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::ReviewChat => {
            panel::review_chat::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        Modal::None => {}
    }

    match k.code {
        KeyCode::Char('w') => {
            host.state_mut().toggle_mode();
            return Ok(());
        }
        KeyCode::Char('n') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            host.cycle_session(true);
            return Ok(());
        }
        KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            host.cycle_session(false);
            return Ok(());
        }
        KeyCode::Char('?') => {
            let focus = host.state().focus;
            let area = host.area()?;
            let state = host.state_mut();
            state.prev_focus = focus;
            state.modal = Modal::Help;
            // The table is longer than any terminal, and Global sits at the
            // top of it. Open where the keys the user just pressed ? about are.
            state.help_offset = panel::help::open_offset(state, area);
            return Ok(());
        }
        KeyCode::Char('F') => {
            // A flow stopped on a conflict is still the flow: while the
            // checkout has unmerged files, the flow key goes back to them
            // rather than offering to start another on top.
            if crate::app::reopen_conflicts(host.state_mut(), None) {
                return Ok(());
            }
            if host.state().focus == Pane::Branches && host.state().branch_actions_available() {
                host.before_flow_modal();
                host.state_mut().modal = Modal::Flow;
            } else {
                let reason = flow_unavailable_reason(host.state());
                host.state_mut().set_status(reason, false);
            }
            return Ok(());
        }
        KeyCode::Char('q') => {
            host.state_mut().request_quit();
            return Ok(());
        }
        KeyCode::Esc if host.state().status.as_ref().is_some_and(|s| s.is_error) => {
            host.state_mut().status = None;
            return Ok(());
        }
        KeyCode::Esc if host.state().llm_job_running() => {
            if let Some(message) = host.state_mut().cancel_llm_jobs() {
                host.state_mut().set_status(message, false);
            }
            return Ok(());
        }
        KeyCode::Esc if host.state().focus == Pane::Status => {
            panel::environments::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        KeyCode::Esc => {
            return Ok(());
        }
        KeyCode::Char('1') => {
            host.state_mut().focus_pane(Pane::Status);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('2') => {
            host.state_mut().focus_pane(Pane::Files);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('3') => {
            host.state_mut().focus_pane(Pane::Branches);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('4') => {
            host.state_mut().focus_pane(Pane::Commits);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('0') => {
            host.state_mut().focus_pane(Pane::Main);
            return Ok(());
        }
        KeyCode::Tab => {
            host.state_mut().focus = cycle_pane(host.state(), true);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::BackTab => {
            host.state_mut().focus = cycle_pane(host.state(), false);
            host.diff_for_focus();
            host.sync_commit_log();
            return Ok(());
        }
        KeyCode::Char('c') => {
            host.state_mut().open_commit_or_stage_all_prompt();
            return Ok(());
        }
        KeyCode::Char('a') => {
            open_author_modal(host.state_mut());
            return Ok(());
        }
        KeyCode::Char(':') => {
            panel::commands::open(host.state_mut());
            return Ok(());
        }
        KeyCode::Char('E') => {
            panel::deployment::open(host.state_mut());
            return Ok(());
        }
        KeyCode::Char(',') => {
            panel::settings::open(host.state_mut(), 0);
            return Ok(());
        }
        KeyCode::Char('L') => {
            open_model_modal(host.state_mut());
            return Ok(());
        }
        KeyCode::Char('p') => {
            host.start_pull();
            return Ok(());
        }
        KeyCode::Char('f') if focused_review_panel(host.state()) => {
            panel::main::handle_key(host.state_mut(), k)?;
            return Ok(());
        }
        KeyCode::Char('f') => {
            host.start_fetch();
            return Ok(());
        }
        KeyCode::Char('P') => {
            if !host.state().has_unpushed_commits() {
                host.state_mut().set_status("nothing to push", false);
                return Ok(());
            }
            spawn_push(host.state_mut());
            return Ok(());
        }
        KeyCode::Char('R') => {
            spawn_assisted_review(host.state_mut());
            return Ok(());
        }
        _ => {}
    }

    let focus_before = host.state().focus;
    let commit_ref_before = selected_commit_ref(host.state());

    let handled = if list_jump(host, focus_before, k)? {
        true
    } else {
        match focus_before {
            Pane::Status => panel::environments::handle_key(host.state_mut(), k)?,
            Pane::Files => panel::files::handle_key(host.state_mut(), k)?,
            Pane::Branches => panel::branches::handle_key(host.state_mut(), k)?,
            Pane::Commits => panel::commits::handle_key(host.state_mut(), k)?,
            Pane::Main => panel::main::handle_key(host.state_mut(), k)?,
        }
    };
    if !handled {
        report_unbound(host.state_mut(), focus_before, k);
    }

    if host.state().pending_action.is_none()
        && (matches!(focus_before, Pane::Files | Pane::Branches | Pane::Commits)
            || reveals_diff(host.state()))
    {
        host.diff_for_focus();
    }
    if selected_commit_ref(host.state()) != commit_ref_before {
        host.sync_commit_log();
    }
    Ok(())
}
