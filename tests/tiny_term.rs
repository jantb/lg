use lg::app::HeadlessApp;
use ratatui::backend::TestBackend;

#[test]
fn renders_at_absurdly_small_sizes() {
    for (w, h) in [(1u16, 1u16), (4, 3), (20, 6), (40, 10), (80, 3)] {
        let mut app = HeadlessApp::new(TestBackend::new(w, h))
            .unwrap_or_else(|e| panic!("{w}x{h}: construct failed: {e}"));
        app.render()
            .unwrap_or_else(|e| panic!("{w}x{h}: render failed: {e}"));
    }
}

#[test]
fn renders_every_modal_at_small_sizes() {
    use lg::state::Modal;
    let modals = [
        Modal::Commit,
        Modal::StageAllBeforeCommit,
        Modal::Push,
        Modal::Author,
        Modal::Model,
        Modal::Help,
        Modal::Flow,
        Modal::Conflict,
        Modal::DeleteBranch,
        Modal::ConfirmDestructive,
    ];
    for (w, h) in [(4u16, 3u16), (20, 6), (40, 10)] {
        for modal in modals {
            let mut app = HeadlessApp::new(TestBackend::new(w, h)).unwrap();
            app.state.modal = modal;
            app.render()
                .unwrap_or_else(|e| panic!("{w}x{h} {modal:?}: render failed: {e}"));
        }
    }
}
