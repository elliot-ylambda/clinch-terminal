use ai::skills::SkillScope;

use crate::ai::skills::SkillDescriptor;

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
#[path = "grouping_tests.rs"]
mod tests;
