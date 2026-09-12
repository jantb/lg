mod app;
mod ui;

use crate::error::Result;
use std::path::Path;

pub fn run(project_root: &Path) -> Result<()> {
    app::TuiApp::run(project_root)
}
