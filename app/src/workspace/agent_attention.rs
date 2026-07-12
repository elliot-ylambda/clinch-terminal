//! Pure ordering logic for cycling focus through tabs whose agent is waiting on the user.
//!
//! The per-tab "needs attention" predicate lives on `Workspace`
//! (`Workspace::tab_needs_agent_attention`), which reuses the agent-icon badge derivation so the
//! cycling action, the "N waiting" header chip, and the yellow tab badges can never drift apart.
//! This module owns only the order-of-cycling math, kept free of `AppContext` so it is directly
//! unit-testable.

/// Indices (in stable tab order) of tabs flagged as needing attention.
///
/// The input is one bool per tab, in tab order; the output preserves that order, so cycling
/// visits waiting tabs top-to-bottom as they appear in the vertical tabs panel.
pub(crate) fn attention_tab_indices(
    needs_attention_by_tab: impl Iterator<Item = bool>,
) -> Vec<usize> {
    needs_attention_by_tab
        .enumerate()
        .filter_map(|(index, needs_attention)| needs_attention.then_some(index))
        .collect()
}

/// The next tab to focus when cycling through tabs that need attention: the first waiting tab
/// strictly after `active_index`, wrapping around to the first waiting tab overall. `None` when
/// no tab is waiting.
///
/// Because the comparison is strict, invoking the cycle while a waiting tab is focused advances
/// past it to the next waiting tab. When the active tab is the only waiting tab, its own index is
/// returned (re-activating the already-active tab is a no-op).
pub(crate) fn next_attention_tab_index(
    waiting_indices: &[usize],
    active_index: usize,
) -> Option<usize> {
    waiting_indices
        .iter()
        .copied()
        .find(|&index| index > active_index)
        .or_else(|| waiting_indices.first().copied())
}

#[cfg(test)]
#[path = "agent_attention_tests.rs"]
mod tests;
