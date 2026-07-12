use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum PaletteMode {
    Command,
    Navigation,
    LaunchConfig,
    WarpDrive,
    Files,
    Conversations,
    /// Recent CLI-agent (Claude/Codex) conversations from the agent-resume journal,
    /// reopenable in a new tab ("Reopen agent conversation").
    AgentConversations,
}
