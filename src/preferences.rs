//! Versioned local configuration, shared by linked worktrees with explicit overrides.
use serde::{Deserialize, Serialize};

mod cache;
mod layers;
mod save;
mod validate;

pub use cache::{
    ai_enabled, animations_enabled, animations_enabled_for, base_branch, configured_category,
    invalidate, load, remote,
};
pub(crate) use layers::merge;
pub use layers::{Loaded, Scope, base_dir, default_folder, scope_path};
pub use save::{
    atomic_write, clear_fields, import_document, reset_category, reset_folder, save_category,
    save_fields, save_folder,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub writing: Writing,
    pub models: Models,
    pub agents: Vec<Agent>,
    pub sandbox: Sandbox,
    pub branches: Branches,
    pub tools: Tools,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Writing {
    pub language: String,
    pub comment_style: String,
    pub commit_prompt: String,
    pub review_style: String,
}
/// The defaults are the per-checkout settings' own, so the two cannot drift.
impl Default for Writing {
    fn default() -> Self {
        crate::settings::RepoSettings::default().into()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Models {
    /// Whether lg talks to the model at all. Off, the commit message is typed,
    /// conflicts are left to the person, and the review assists stay quiet,
    /// so nothing waits on a server that is not running.
    pub enabled: bool,
    /// Who answers: `local` for the chat endpoint below, or `claude` for the
    /// `claude` CLI, which then writes every commit message, review assist,
    /// chat reply and conflict resolution.
    pub provider: crate::llm::LlmProvider,
    pub model: String,
    pub endpoint: String,
    /// The model the `claude` provider asks for (`sonnet`, `opus`, `haiku` or
    /// a full id). Empty uses the Claude agent profile's model, or the CLI's
    /// default.
    pub claude_model: String,
}
impl Default for Models {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: crate::llm::LlmProvider::Mtplx,
            model: crate::config::LLM_MODEL.into(),
            endpoint: crate::config::MTPLX_CHAT_ENDPOINT.into(),
            claude_model: String::new(),
        }
    }
}
/// The providers `models.provider` may name.
pub const PROVIDERS: &[&str] = &crate::llm::LlmProvider::CONFIG_VALUES;
/// A setting stored as one word out of a fixed few. The file holds the word;
/// anything else is refused with `$refusal` when the file is read, which keeps
/// that file out of the effective configuration as any invalid file is.
macro_rules! word_setting {
    (
        $(#[$meta:meta])*
        $name:ident, $refusal:expr, { $($(#[$vmeta:meta])* $variant:ident = $word:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "&'static str")]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }
        impl $name {
            /// Every word, in the order the settings picker offers them.
            pub const NAMES: &'static [&'static str] = &[$($word),+];
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $word),+
                }
            }
        }
        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(word: String) -> std::result::Result<Self, String> {
                match word.as_str() {
                    $($word => Ok(Self::$variant),)+
                    _ => Err(($refusal)(word.as_str())),
                }
            }
        }
        impl From<$name> for &'static str {
            fn from(value: $name) -> Self {
                value.as_str()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}
word_setting!(
    /// The integration an agent is driven through.
    Adapter,
    |word: &str| format!("unknown agent adapter {word}"),
    {
        Claude = "claude",
        Codex = "codex",
        Pi = "pi",
        Terminal = "terminal",
    }
);
word_setting!(
    /// How tightly an agent is confined.
    Confinement,
    |_: &str| "confinement must be terrarium, agent or direct".to_string(),
    {
        /// The terrarium sandbox.
        Terrarium = "terrarium",
        /// The agent's own permission harness.
        Agent = "agent",
        /// Nothing.
        Direct = "direct",
    }
);
word_setting!(
    /// How a promotion lands on an environment branch.
    Strategy,
    |_: &str| "promotion strategy must be merge, squash or ff-only".to_string(),
    {
        Merge = "merge",
        Squash = "squash",
        FfOnly = "ff-only",
    }
);
impl Adapter {
    /// Whether the agent brings a permission harness of its own that lg can
    /// run it under.
    pub fn has_own_permissions(self) -> bool {
        matches!(self, Self::Claude | Self::Codex)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    pub name: String,
    pub adapter: Adapter,
    pub executable: String,
    pub args: Vec<String>,
    pub model: String,
    pub confinement: Confinement,
    pub default: bool,
}
impl Default for Agent {
    fn default() -> Self {
        Self {
            name: "Custom agent".into(),
            adapter: Adapter::Terminal,
            executable: String::new(),
            args: vec![],
            model: String::new(),
            confinement: default_confinement(Adapter::Terminal),
            default: false,
        }
    }
}
impl Agent {
    /// Whether lg wraps the process in the Terrarium sandbox. A terminal is
    /// never wrapped: it is the user's own shell, and confining it only gets
    /// in the way of the work done there by hand.
    pub fn sandboxed(&self) -> bool {
        self.confinement == Confinement::Terrarium && self.adapter != Adapter::Terminal
    }
}
/// The integrations an agent may be driven through. The settings picker offers
/// exactly these, so a typo cannot reach the file.
pub const ADAPTERS: &[&str] = Adapter::NAMES;
/// The languages generated prose may be written in.
pub const LANGUAGES: &[&str] = &["English", "Norwegian"];
/// How tightly an agent is confined: the terrarium sandbox, the agent's own
/// permission harness, or nothing.
pub const CONFINEMENTS: &[&str] = Confinement::NAMES;
/// The ways a promotion may land on an environment branch.
pub const STRATEGIES: &[&str] = Strategy::NAMES;
/// The confinement an adapter starts out with: coding agents bring their own
/// permission harness and run under it, anything else runs unconfined.
pub fn default_confinement(adapter: Adapter) -> Confinement {
    if adapter.has_own_permissions() {
        Confinement::Agent
    } else {
        Confinement::Direct
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Sandbox {
    pub preset: String,
}
impl Default for Sandbox {
    fn default() -> Self {
        Self {
            preset: "rust".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Branches {
    pub base: String,
    pub remote: String,
    pub protected: Vec<String>,
    pub environments: Vec<Environment>,
    pub promotions: Vec<Promotion>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub remote: String,
    pub branch: String,
    pub url: String,
}
impl Default for Environment {
    fn default() -> Self {
        Self {
            id: "new".into(),
            name: "New environment".into(),
            remote: "origin".into(),
            branch: String::new(),
            url: String::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Promotion {
    pub from: String,
    pub to: String,
    pub strategy: Strategy,
    pub push: bool,
}
impl Default for Promotion {
    fn default() -> Self {
        Self {
            from: "feature".into(),
            to: "dev".into(),
            strategy: Strategy::Merge,
            push: true,
        }
    }
}
/// A checkout with nothing saved has no environments of its own; what it
/// deploys is read off its branches by [`detect_branches`].
impl Default for Branches {
    fn default() -> Self {
        Self {
            base: crate::config::BRANCH_MAIN.into(),
            remote: crate::config::DEFAULT_PUSH_REMOTE.into(),
            protected: vec![crate::config::BRANCH_MAIN.into()],
            environments: vec![],
            promotions: vec![],
        }
    }
}
/// The id of the environment the integration branch deploys.
pub const PROD_ENV_ID: &str = "prod";
/// Where the detected configuration is said to come from.
pub const DETECTED_SOURCE: &str = "Detected from local branches";
/// The branch configuration a checkout implies. `develop` or `dev` is the
/// development environment and `test` the test environment, when those
/// branches exist; the integration branch is production unless a `prod`
/// branch exists to take that place.
pub fn detect_branches(local: &[String]) -> Branches {
    let has = |name: &str| local.iter().any(|b| b == name);
    let base = if !has(crate::config::BRANCH_MAIN) && has("master") {
        "master".to_string()
    } else {
        crate::config::BRANCH_MAIN.to_string()
    };
    let remote = crate::config::DEFAULT_PUSH_REMOTE.to_string();
    let environment = |id: &str, name: &str, branch: &str| Environment {
        id: id.into(),
        name: name.into(),
        remote: remote.clone(),
        branch: branch.into(),
        url: String::new(),
    };
    let mut environments = Vec::new();
    if let Some(dev) = crate::config::DEV_BRANCH_NAMES
        .into_iter()
        .find(|name| has(name))
    {
        environments.push(environment("dev", "Development", dev));
    }
    if has(crate::config::BRANCH_TEST) {
        environments.push(environment("test", "Test", crate::config::BRANCH_TEST));
    }
    let prod = if has(PROD_ENV_ID) { PROD_ENV_ID } else { &base };
    environments.push(environment(PROD_ENV_ID, "Production", prod));
    let promotions = environments
        .iter()
        .filter(|e| e.id != PROD_ENV_ID)
        .map(|e| Promotion {
            to: e.id.clone(),
            ..Promotion::default()
        })
        .collect();
    let mut protected = vec![base.clone()];
    for e in &environments {
        if !protected.contains(&e.branch) {
            protected.push(e.branch.clone());
        }
    }
    Branches {
        base,
        remote,
        protected,
        environments,
        promotions,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Tools {
    pub decorative_animations: bool,
    pub quiet_author_emails: Vec<String>,
    pub editor: String,
    pub editor_args: Vec<String>,
    pub terminal: String,
    pub terminal_args: Vec<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            writing: Writing::default(),
            models: Models::default(),
            agents: [Adapter::Claude, Adapter::Codex, Adapter::Pi]
                .into_iter()
                .map(|adapter| Agent {
                    name: adapter.as_str().into(),
                    adapter,
                    executable: adapter.as_str().into(),
                    default: adapter == Adapter::Claude,
                    confinement: default_confinement(adapter),
                    ..Agent::default()
                })
                .collect(),
            sandbox: Sandbox::default(),
            branches: Branches::default(),
            tools: Tools::default(),
        }
    }
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            decorative_animations: true,
            quiet_author_emails: vec![],
            editor: String::new(),
            editor_args: vec![],
            terminal: String::new(),
            terminal_args: vec![],
        }
    }
}

#[cfg(test)]
mod detect_tests;

#[cfg(test)]
mod word_setting_tests;
