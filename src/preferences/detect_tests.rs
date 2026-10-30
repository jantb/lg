use super::*;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}
fn valid(b: &Branches) {
    Preferences {
        branches: b.clone(),
        ..Preferences::default()
    }
    .validate()
    .unwrap();
}
fn branch_of<'a>(b: &'a Branches, id: &str) -> Option<&'a str> {
    b.environments
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.branch.as_str())
}

#[test]
fn a_checkout_without_deploy_branches_only_has_production_on_main() {
    let b = detect_branches(&names(&["main", "feature/x"]));
    assert_eq!(branch_of(&b, "prod"), Some("main"));
    assert_eq!(branch_of(&b, "dev"), None);
    assert_eq!(branch_of(&b, "test"), None);
    assert!(b.promotions.is_empty());
    valid(&b);
}

#[test]
fn dev_and_test_branches_become_environments_with_promotions() {
    let b = detect_branches(&names(&["main", "dev", "test"]));
    assert_eq!(branch_of(&b, "dev"), Some("dev"));
    assert_eq!(branch_of(&b, "test"), Some("test"));
    assert_eq!(branch_of(&b, "prod"), Some("main"));
    let targets: Vec<_> = b.promotions.iter().map(|p| p.to.as_str()).collect();
    assert!(targets.contains(&"dev") && targets.contains(&"test"));
    assert!(!targets.contains(&"prod"));
    for e in &b.environments {
        assert!(b.protected.contains(&e.branch), "{} unprotected", e.branch);
    }
    valid(&b);
}

#[test]
fn a_local_prod_branch_is_production_instead_of_main() {
    let b = detect_branches(&names(&["main", "prod", "develop"]));
    assert_eq!(branch_of(&b, "prod"), Some("prod"));
    assert_eq!(branch_of(&b, "dev"), Some("develop"));
    assert_eq!(b.base, "main");
}

#[test]
fn master_is_the_integration_branch_when_there_is_no_main() {
    let b = detect_branches(&names(&["master", "test"]));
    assert_eq!(b.base, "master");
    assert_eq!(branch_of(&b, "prod"), Some("master"));
}
