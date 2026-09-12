use crate::config::project::ProjectProfile;
use crate::config::workspace::WorkspaceConfig;
use crate::error::Result;
use crate::proxy::log::{self as blocked_log, BlockedEntry};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(PartialEq)]
pub enum Panel {
    Allowed,
    Blocked,
}

pub enum InputMode {
    Normal,
    AddDomain(String),
}

#[derive(Debug, PartialEq, Eq)]
enum ConfigKind {
    Project,
    Workspace,
}

pub struct TuiApp {
    pub allowed: Vec<String>,
    pub blocked: Vec<BlockedEntry>,
    pub panel: Panel,
    pub allowed_cursor: usize,
    pub blocked_cursor: usize,
    pub input_mode: InputMode,
    pub project_root: PathBuf,
    pub dirty: bool,
}

impl TuiApp {
    pub fn new(project_root: &Path) -> Result<Self> {
        let (allowed, _kind) = load_allowed_domains(project_root)?;
        let blocked = blocked_log::load(project_root).unwrap_or_default();
        Ok(Self {
            allowed,
            blocked,
            panel: Panel::Allowed,
            allowed_cursor: 0,
            blocked_cursor: 0,
            input_mode: InputMode::Normal,
            project_root: project_root.to_path_buf(),
            dirty: false,
        })
    }

    pub fn run(project_root: &Path) -> Result<()> {
        let mut app = TuiApp::new(project_root)?;
        let mut terminal = ratatui::init();
        let result = app.run_loop(&mut terminal);
        ratatui::restore();
        result
    }

    fn refresh_blocked(&mut self) {
        self.blocked = blocked_log::load(&self.project_root).unwrap_or_default();
        if !self.blocked.is_empty() {
            self.blocked_cursor = self.blocked_cursor.min(self.blocked.len() - 1);
        } else {
            self.blocked_cursor = 0;
        }
    }

    fn run_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        const REFRESH: Duration = Duration::from_secs(2);
        let mut last_refresh = Instant::now();
        loop {
            terminal.draw(|f| super::ui::draw(f, self))?;
            if event::poll(REFRESH.saturating_sub(last_refresh.elapsed()))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                match &self.input_mode {
                    InputMode::Normal => {
                        if self.handle_normal_key(key.code)? {
                            return Ok(());
                        }
                    }
                    InputMode::AddDomain(_) => self.handle_add_key(key.code),
                }
                if self.dirty {
                    self.save()?;
                    self.dirty = false;
                }
            }
            if last_refresh.elapsed() >= REFRESH {
                self.refresh_blocked();
                last_refresh = Instant::now();
            }
        }
    }

    /// Returns true if the loop should exit.
    fn handle_normal_key(&mut self, code: KeyCode) -> Result<bool> {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                return Ok(true);
            }
            KeyCode::Tab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.panel = if self.panel == Panel::Allowed {
                    Panel::Blocked
                } else {
                    Panel::Allowed
                };
            }
            KeyCode::Up | KeyCode::Char('k') => match self.panel {
                Panel::Allowed => {
                    if self.allowed_cursor > 0 {
                        self.allowed_cursor -= 1;
                    }
                }
                Panel::Blocked => {
                    if self.blocked_cursor > 0 {
                        self.blocked_cursor -= 1;
                    }
                }
            },
            KeyCode::Down | KeyCode::Char('j') => match self.panel {
                Panel::Allowed => {
                    if !self.allowed.is_empty() && self.allowed_cursor < self.allowed.len() - 1 {
                        self.allowed_cursor += 1;
                    }
                }
                Panel::Blocked => {
                    if !self.blocked.is_empty() && self.blocked_cursor < self.blocked.len() - 1 {
                        self.blocked_cursor += 1;
                    }
                }
            },
            KeyCode::Enter => match self.panel {
                Panel::Allowed => {
                    if let Some(domain) = self.allowed.get(self.allowed_cursor).cloned() {
                        self.allowed.remove(self.allowed_cursor);
                        if self.allowed_cursor > 0 && self.allowed_cursor >= self.allowed.len() {
                            self.allowed_cursor = self.allowed.len().saturating_sub(1);
                        }
                        self.dirty = true;
                        // Remove from blocked log too (cleanup) — no-op if not present
                        let _ = blocked_log::remove(&self.project_root, &domain);
                    }
                }
                Panel::Blocked => {
                    if let Some(entry) = self.blocked.get(self.blocked_cursor).cloned() {
                        if !self.allowed.contains(&entry.domain) {
                            self.allowed.push(entry.domain.clone());
                        }
                        let _ = blocked_log::remove(&self.project_root, &entry.domain);
                        self.blocked.remove(self.blocked_cursor);
                        if self.blocked_cursor > 0 && self.blocked_cursor >= self.blocked.len() {
                            self.blocked_cursor = self.blocked.len().saturating_sub(1);
                        }
                        self.dirty = true;
                    }
                }
            },
            KeyCode::Char('a') => {
                self.input_mode = InputMode::AddDomain(String::new());
            }
            KeyCode::Char('d') | KeyCode::Char('x') => match self.panel {
                Panel::Allowed => {
                    if !self.allowed.is_empty() {
                        self.allowed.remove(self.allowed_cursor);
                        if self.allowed_cursor > 0 && self.allowed_cursor >= self.allowed.len() {
                            self.allowed_cursor = self.allowed.len().saturating_sub(1);
                        }
                        self.dirty = true;
                    }
                }
                Panel::Blocked => {
                    if let Some(entry) = self.blocked.get(self.blocked_cursor).cloned() {
                        let _ = blocked_log::remove(&self.project_root, &entry.domain);
                        self.blocked.remove(self.blocked_cursor);
                        if self.blocked_cursor > 0 && self.blocked_cursor >= self.blocked.len() {
                            self.blocked_cursor = self.blocked.len().saturating_sub(1);
                        }
                    }
                }
            },
            _ => {}
        }
        Ok(false)
    }

    fn handle_add_key(&mut self, code: KeyCode) {
        let InputMode::AddDomain(ref mut input) = self.input_mode else {
            return;
        };
        match code {
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => {
                let domain = input.trim().to_string();
                if !domain.is_empty() && !self.allowed.contains(&domain) {
                    self.allowed.push(domain);
                    self.dirty = true;
                }
                self.input_mode = InputMode::Normal;
            }
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
            }
            _ => {}
        }
    }

    fn save(&self) -> Result<()> {
        match config_kind(&self.project_root)? {
            ConfigKind::Workspace => {
                let mut config = WorkspaceConfig::load(&self.project_root)?;
                config.network.allowed_domains = self.allowed.clone();
                config.save(&self.project_root)?;
            }
            ConfigKind::Project => {
                let mut profile = ProjectProfile::load(&self.project_root)?;
                profile.network.allowed_domains = self.allowed.clone();
                profile.save(&self.project_root)?;
            }
        }
        // Remove any allowed domain from the blocked log
        for domain in &self.allowed {
            let _ = blocked_log::remove(&self.project_root, domain);
        }
        Ok(())
    }
}

fn config_kind(project_root: &Path) -> Result<ConfigKind> {
    if WorkspaceConfig::load(project_root).is_ok() {
        Ok(ConfigKind::Workspace)
    } else {
        ProjectProfile::load(project_root)?;
        Ok(ConfigKind::Project)
    }
}

fn load_allowed_domains(project_root: &Path) -> Result<(Vec<String>, ConfigKind)> {
    if let Ok(config) = WorkspaceConfig::load(project_root) {
        let mut allowed = config.network.allowed_domains;
        for project in &config.projects {
            crate::profile::presets::ensure_defaults(&mut allowed, &project.preset);
        }
        return Ok((allowed, ConfigKind::Workspace));
    }

    let profile = ProjectProfile::load(project_root)?;
    let mut allowed = profile.network.allowed_domains;
    let preset = profile.preset.as_deref().unwrap_or("none");
    crate::profile::presets::ensure_defaults(&mut allowed, preset);
    Ok((allowed, ConfigKind::Project))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::global::override_home_for_tests;

    fn temp_root(label: &str) -> (PathBuf, PathBuf, crate::config::global::TestHomeGuard) {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("{}_{}", label, uuid::Uuid::new_v4()));
        let home = base.join("home");
        let root = base.join("project");
        std::fs::create_dir_all(&root).unwrap();
        let guard = override_home_for_tests(home);
        (base, root, guard)
    }

    #[test]
    fn tui_loads_workspace_allowed_domains() {
        let (base, root, _guard) = temp_root("tui_workspace_allowed");
        let config = WorkspaceConfig {
            projects: vec![crate::config::workspace::WorkspaceProject {
                path: "api".to_string(),
                preset: "rust".to_string(),
            }],
            network: crate::config::project::NetworkConfig {
                proxy_enabled: true,
                allowed_domains: vec!["custom.example.com".to_string()],
            },
            allow_commit: false,
            command: None,
        };
        config.save(&root).unwrap();

        let (allowed, kind) = load_allowed_domains(&root).unwrap();

        assert_eq!(kind, ConfigKind::Workspace);
        assert!(allowed.iter().any(|domain| domain == "custom.example.com"));
        assert!(allowed.iter().any(|domain| domain == "crates.io"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn tui_save_updates_workspace_config() {
        let (base, root, _guard) = temp_root("tui_workspace_save");
        let config = WorkspaceConfig {
            projects: vec![crate::config::workspace::WorkspaceProject {
                path: "api".to_string(),
                preset: "none".to_string(),
            }],
            network: crate::config::project::NetworkConfig {
                proxy_enabled: true,
                allowed_domains: vec![],
            },
            allow_commit: false,
            command: None,
        };
        config.save(&root).unwrap();

        let mut app = TuiApp::new(&root).unwrap();
        app.allowed = vec!["saved.example.com".to_string()];
        app.save().unwrap();

        let loaded = WorkspaceConfig::load(&root).unwrap();
        assert_eq!(loaded.network.allowed_domains, vec!["saved.example.com"]);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
