use ai::skills::{SkillProvider, SkillScope};

use crate::ai::skills::SkillDescriptor;
use crate::terminal::CLIAgent;

/// Which agent's accessible skill set the panel is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillsSubtab {
    All,
    Claude,
    Codex,
}

impl SkillsSubtab {
    pub fn label(self) -> &'static str {
        match self {
            SkillsSubtab::All => "All",
            SkillsSubtab::Claude => "Claude",
            SkillsSubtab::Codex => "Codex",
        }
    }
}

/// The provider set to query for a subtab. `None` means "All" (no provider filter — use
/// `get_skills_for_working_directory`). `Some(providers)` means query
/// `skills_for_providers(providers)`.
pub fn providers_for_subtab(subtab: SkillsSubtab) -> Option<&'static [SkillProvider]> {
    match subtab {
        SkillsSubtab::All => None,
        SkillsSubtab::Claude => Some(CLIAgent::Claude.supported_skill_providers()),
        SkillsSubtab::Codex => Some(CLIAgent::Codex.supported_skill_providers()),
    }
}

/// Ordered display grouping: Home, then Project, then Bundled. Empty groups are omitted.
/// Within a group, skills are sorted case-insensitively by name.
pub fn group_skills_by_scope(
    skills: Vec<SkillDescriptor>,
) -> Vec<(SkillScope, Vec<SkillDescriptor>)> {
    const ORDER: [SkillScope; 3] = [SkillScope::Home, SkillScope::Project, SkillScope::Bundled];
    let mut out: Vec<(SkillScope, Vec<SkillDescriptor>)> = Vec::new();
    for scope in ORDER {
        let mut group: Vec<SkillDescriptor> = skills
            .iter()
            .filter(|s| s.scope == scope)
            .cloned()
            .collect();
        if group.is_empty() {
            continue;
        }
        group.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out.push((scope, group));
    }
    out
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn all_subtab_has_no_provider_filter() {
        assert!(providers_for_subtab(SkillsSubtab::All).is_none());
    }

    #[test]
    fn codex_reads_more_providers_than_claude() {
        let claude = providers_for_subtab(SkillsSubtab::Claude).unwrap();
        let codex = providers_for_subtab(SkillsSubtab::Codex).unwrap();
        assert!(claude.contains(&SkillProvider::Claude));
        assert!(codex.len() > claude.len());
    }
}
