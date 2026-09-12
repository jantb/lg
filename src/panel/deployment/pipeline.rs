//! The environments as the tree a change travels: the branch being looked at
//! at the root, every environment a promotion rule leads to hanging off the
//! stage it is promoted from, and the ones no rule reaches listed last.
//!
//! This is the model the picture and the explanation are both drawn from, so
//! what the diagram shows and what the words say cannot disagree.

use ratatui::style::Color;

use crate::{
    git::BranchReleaseStatus,
    preferences::{Branches, Environment, Promotion},
    ui::palette,
};

/// The word a branch with no environment of its own plays in the promotion
/// rules.
pub const FEATURE_ROLE: &str = "feature";

/// How the branch being looked at stands against an environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inclusion {
    /// The branch is the one this environment deploys.
    Source,
    /// Every commit of the branch is already in the environment.
    Included { released_at: String },
    /// This many commits of the branch have not reached the environment.
    Behind(usize),
    /// lg has not compared them.
    Unknown,
}

/// One stop on the pipeline.
#[derive(Debug, Clone)]
pub struct Stage {
    /// The environment's name, or the branch name for a feature at the root.
    pub name: String,
    /// `remote/branch` the environment deploys; empty for a feature branch.
    pub target: String,
    /// What the promotion rules call this stage: an environment id or
    /// [`FEATURE_ROLE`].
    pub role: String,
    /// Index into the configured environments, when this is one.
    pub env: Option<usize>,
    pub color: Color,
    pub url: String,
    pub inclusion: Inclusion,
    /// The stage this one is promoted from. `None` at the root.
    pub parent: Option<usize>,
    /// The rule that promotes the parent into this stage. `None` at the root
    /// and for an environment no rule reaches.
    pub rule: Option<Promotion>,
}

/// The stages in the order they are drawn: each stage directly after the
/// stages promoted from it, unreachable environments at the end.
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub stages: Vec<Stage>,
    /// The branch at the root.
    pub source: String,
}

/// Colours for environments beyond the three the palette names.
const EXTRA_COLORS: [Color; 3] = [
    Color::Rgb(255, 160, 90),
    Color::Rgb(232, 122, 204),
    Color::Rgb(120, 205, 220),
];

/// The colour an environment is drawn in everywhere: the integration branch's
/// magenta for whatever deploys it, the deploy-branch colours for dev and test,
/// and a spare hue for anything else.
pub fn environment_color(config: &Branches, env: &Environment, index: usize) -> Color {
    if env.branch == config.base {
        palette::LANE_MAIN
    } else {
        match env.id.as_str() {
            "dev" => palette::LANE_DEV,
            "test" => palette::LANE_TEST,
            "prod" => palette::LANE_MAIN,
            _ => EXTRA_COLORS[index % EXTRA_COLORS.len()],
        }
    }
}

impl Pipeline {
    pub fn build(config: &Branches, source: Option<&str>, releases: &BranchReleaseStatus) -> Self {
        let source = source.unwrap_or("detached HEAD").to_string();
        let envs = &config.environments;
        let source_env = envs
            .iter()
            .position(|e| !e.branch.is_empty() && e.branch == source);
        let stage_for = |index: usize, parent: Option<usize>, rule: Option<Promotion>| {
            let e = &envs[index];
            let inclusion = if Some(index) == source_env {
                Inclusion::Source
            } else {
                match releases.environments.get(&e.id) {
                    Some(s) if s.missing_commits == 0 => Inclusion::Included {
                        released_at: s.released_at.clone(),
                    },
                    Some(s) => Inclusion::Behind(s.missing_commits),
                    None => Inclusion::Unknown,
                }
            };
            Stage {
                name: e.name.clone(),
                target: if e.branch.is_empty() {
                    String::new()
                } else {
                    format!("{}/{}", e.remote, e.branch)
                },
                role: e.id.clone(),
                env: Some(index),
                color: environment_color(config, e, index),
                url: e.url.clone(),
                inclusion,
                parent,
                rule,
            }
        };
        let root = match source_env {
            Some(index) => stage_for(index, None, None),
            None => Stage {
                name: source.clone(),
                target: String::new(),
                role: FEATURE_ROLE.into(),
                env: None,
                color: palette::LANE_FEATURE,
                url: String::new(),
                inclusion: Inclusion::Source,
                parent: None,
                rule: None,
            },
        };
        // Breadth first from the root, each environment placed under the
        // first stage a rule promotes into it, so a stage reached two ways is
        // drawn once, on its shortest route.
        let mut nodes = vec![root];
        let mut placed: Vec<usize> = source_env.into_iter().collect();
        let mut cursor = 0;
        while cursor < nodes.len() {
            let role = nodes[cursor].role.clone();
            for (index, e) in envs.iter().enumerate() {
                if placed.contains(&index) {
                    continue;
                }
                if let Some(rule) = config
                    .promotions
                    .iter()
                    .find(|p| p.from == role && p.to == e.id)
                {
                    placed.push(index);
                    nodes.push(stage_for(index, Some(cursor), Some(rule.clone())));
                }
            }
            cursor += 1;
        }
        for index in 0..envs.len() {
            if !placed.contains(&index) {
                nodes.push(stage_for(index, Some(0), None));
            }
        }
        // Then laid out depth first, so a stage's own promotions follow it.
        let mut order = Vec::with_capacity(nodes.len());
        visit(&nodes, 0, &mut order);
        let stages = order
            .iter()
            .map(|&i| {
                let mut stage = nodes[i].clone();
                stage.parent = stage
                    .parent
                    .and_then(|p| order.iter().position(|&o| o == p));
                stage
            })
            .collect();
        Self { stages, source }
    }

    /// The stage drawn for the configured environment `env`.
    pub fn stage_of(&self, env: usize) -> Option<usize> {
        self.stages.iter().position(|s| s.env == Some(env))
    }

    /// The stages a promotion passes through to reach `stage`, from the first
    /// hop after the root to `stage` itself. Empty for the root and for a
    /// stage no rule reaches.
    pub fn route(&self, stage: usize) -> Vec<usize> {
        let mut route = Vec::new();
        let mut at = stage;
        while let Some(parent) = self.stages[at].parent {
            if self.stages[at].rule.is_none() {
                return Vec::new();
            }
            route.push(at);
            at = parent;
        }
        route.reverse();
        route
    }

    /// Whether `stage` has stages promoted from it drawn after `stage` that
    /// are not the last under the same parent — that is, whether its parent's
    /// spine has to run on past it.
    pub fn has_later_sibling(&self, stage: usize) -> bool {
        let parent = self.stages[stage].parent;
        parent.is_some()
            && self
                .stages
                .iter()
                .skip(stage + 1)
                .any(|s| s.parent == parent)
    }
}

fn visit(nodes: &[Stage], index: usize, order: &mut Vec<usize>) {
    order.push(index);
    for (child, node) in nodes.iter().enumerate() {
        if node.parent == Some(index) {
            visit(nodes, child, order);
        }
    }
}

/// How a rule lands, in words a person reads once: the method and whether the
/// result is pushed.
pub fn rule_label(rule: &Promotion) -> String {
    format!(
        "{}, {}",
        rule.strategy,
        if rule.push { "push" } else { "no push" }
    )
}

/// The method spelled out for the explanation pane.
pub fn strategy_sentence(rule: &Promotion, target: &str) -> String {
    match rule.strategy.as_str() {
        "squash" => "squashed into a single commit".to_string(),
        "ff-only" => format!("fast-forward only, refused if {target} has commits of its own"),
        _ => "with a merge commit that keeps the branch history".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::detect_branches;

    fn detected() -> Branches {
        detect_branches(&["main".into(), "develop".into(), "test".into()])
    }

    #[test]
    fn a_feature_branch_is_the_root_and_reaches_the_environments_a_rule_leads_to() {
        let config = detected();
        let pipeline = Pipeline::build(&config, Some("feature/x"), &Default::default());
        assert_eq!(pipeline.stages[0].name, "feature/x");
        assert_eq!(pipeline.stages[0].role, FEATURE_ROLE);
        let dev = pipeline
            .stage_of(
                config
                    .environments
                    .iter()
                    .position(|e| e.id == "dev")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(pipeline.route(dev), vec![dev]);
        let prod = pipeline
            .stage_of(
                config
                    .environments
                    .iter()
                    .position(|e| e.id == "prod")
                    .unwrap(),
            )
            .unwrap();
        assert!(pipeline.route(prod).is_empty());
    }

    #[test]
    fn the_branch_an_environment_deploys_sits_at_the_root() {
        let config = detected();
        let pipeline = Pipeline::build(&config, Some("main"), &Default::default());
        assert_eq!(pipeline.stages[0].name, "Production");
        assert_eq!(pipeline.stages[0].inclusion, Inclusion::Source);
        assert_eq!(pipeline.stages.len(), config.environments.len());
    }

    #[test]
    fn a_two_hop_route_lists_both_hops_in_travel_order() {
        let mut config = detected();
        config.promotions.retain(|p| p.to != "test");
        config.promotions.push(Promotion {
            from: "dev".into(),
            to: "test".into(),
            ..Promotion::default()
        });
        let pipeline = Pipeline::build(&config, Some("feature/x"), &Default::default());
        let test = pipeline
            .stage_of(
                config
                    .environments
                    .iter()
                    .position(|e| e.id == "test")
                    .unwrap(),
            )
            .unwrap();
        let route = pipeline.route(test);
        assert_eq!(route.len(), 2);
        assert_eq!(pipeline.stages[route[0]].role, "dev");
        assert_eq!(route[1], test);
        assert_eq!(pipeline.stages[test].parent, Some(route[0]));
    }
}
