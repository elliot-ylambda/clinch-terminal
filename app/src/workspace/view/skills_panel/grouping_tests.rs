use ai::skills::SkillProvider;

use super::*;

fn desc(name: &str, scope: SkillScope, provider: SkillProvider) -> SkillDescriptor {
    SkillDescriptor {
        reference: ai::skills::SkillReference::BundledSkillId(name.to_string()),
        name: name.to_string(),
        description: String::new(),
        scope,
        provider,
        icon_override: None,
    }
}

#[test]
fn groups_are_ordered_home_project_bundled_and_sorted() {
    let skills = vec![
        desc("zebra", SkillScope::Project, SkillProvider::Agents),
        desc("alpha", SkillScope::Project, SkillProvider::Agents),
        desc("home-one", SkillScope::Home, SkillProvider::Claude),
    ];
    let grouped = group_skills_by_scope(skills);
    assert_eq!(grouped[0].0, SkillScope::Home);
    assert_eq!(grouped[1].0, SkillScope::Project);
    assert_eq!(
        grouped[1]
            .1
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zebra"]
    );
}
