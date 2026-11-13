//! Stops the agent running in a pane's foreground when its tab is put to sleep, and makes sure
//! nothing it started is left behind.
//!
//! Claude Code handles SIGTERM: it restores the terminal and exits within a second. Codex installs
//! no signal handlers, so any signal ends it at once, which is also what quitting Clinch does to
//! it. Both write their transcript as they go, and Clinch only stops an agent that is idle, so
//! neither loses anything. What a signal does not guarantee is that helper processes (MCP servers
//! and the like) go with the agent, so those are checked for afterwards and ended.
//!
//! Everything here blocks while it waits, so it runs on its own thread.
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// How long the agent gets to exit after being asked. The pane gives up on the sleep after this.
const EXIT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long helpers get to notice their agent has gone before they are told to stop.
const HELPER_SETTLE: Duration = Duration::from_secs(1);
/// How long a helper gets to stop when asked, before it is killed.
const HELPER_STOP_TIMEOUT: Duration = Duration::from_secs(3);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// What Clinch needs to know about a process to follow it: who started it and what it is. The
/// start time tells a process apart from a later one that was given the same id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessInfo {
    pub(crate) pid: u32,
    pub(crate) parent: Option<u32>,
    pub(crate) name: String,
    pub(crate) started: u64,
}

/// How stopping a foreground job went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StopOutcome {
    /// The job was gone within the time allowed.
    pub(crate) exited: bool,
    /// Helpers that outlived the agent and were told to stop.
    pub(crate) helpers_stopped: usize,
    /// Helpers that ignored that and were killed.
    pub(crate) helpers_killed: usize,
}

/// The process group in the foreground of the terminal behind `pty`, when that is a job rather
/// than the shell itself waiting at its prompt.
pub(crate) fn foreground_job_group(pty: std::os::fd::RawFd, shell_pid: u32) -> Option<u32> {
    // Safe to call with any descriptor: it fails with ENOTTY if `pty` is no longer a terminal.
    let group = u32::try_from(unsafe { libc::tcgetpgrp(pty) }).ok()?;
    (group > 1 && group != shell_pid).then_some(group)
}

fn running_processes() -> Vec<ProcessInfo> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    system
        .processes()
        .values()
        .map(|process| ProcessInfo {
            pid: process.pid().as_u32(),
            parent: process.parent().map(|parent| parent.as_u32()),
            name: process.name().to_string_lossy().into_owned(),
            started: process.start_time(),
        })
        .collect()
}

/// `root` and everything started under it, parents before their children.
pub(crate) fn process_tree(processes: &[ProcessInfo], root: u32) -> Vec<ProcessInfo> {
    let mut children: HashMap<u32, Vec<&ProcessInfo>> = HashMap::new();
    for process in processes {
        if let Some(parent) = process.parent {
            children.entry(parent).or_default().push(process);
        }
    }
    let Some(root) = processes.iter().find(|process| process.pid == root) else {
        return Vec::new();
    };
    let mut tree = vec![root.clone()];
    let mut seen = HashSet::from([root.pid]);
    let mut next = 0;
    while next < tree.len() {
        let parent = tree[next].pid;
        next += 1;
        let mut below = children.get(&parent).cloned().unwrap_or_default();
        below.sort_by_key(|process| process.pid);
        for child in below {
            if seen.insert(child.pid) {
                tree.push(child.clone());
            }
        }
    }
    tree
}

/// The agent itself among a foreground job's processes: the outermost one named like an agent. A
/// launcher script usually sits above it, and signalling that would end the script at once and
/// hand the terminal back while the agent was still shutting down.
pub(crate) fn agent_process<'a>(
    tree: &'a [ProcessInfo],
    is_agent: &dyn Fn(&str) -> bool,
) -> Option<&'a ProcessInfo> {
    tree.iter().find(|process| is_agent(&process.name))
}

fn is_cli_agent(name: &str) -> bool {
    matches!(name, "claude" | "codex")
}

fn signal(pid: u32, signal: Signal) {
    let _ = kill(Pid::from_raw(pid as i32), signal);
}

/// The processes of `tree` that are still running, ignoring any id that has been reused since.
/// Looks only at those processes, so polling it stays cheap when many tabs go to sleep at once.
fn survivors(tree: &[ProcessInfo]) -> Vec<ProcessInfo> {
    let pids: Vec<sysinfo::Pid> = tree
        .iter()
        .map(|process| sysinfo::Pid::from_u32(process.pid))
        .collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing(),
    );
    tree.iter()
        .filter(|process| {
            system
                .process(sysinfo::Pid::from_u32(process.pid))
                .is_some_and(|now| now.start_time() == process.started)
        })
        .cloned()
        .collect()
}

fn wait_until(deadline: Instant, mut done: impl FnMut() -> bool) -> bool {
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Asks the job whose process group is `foreground_group` to stop, waits for it, then ends
/// whatever it started that is still running. Never kills the agent itself: one that won't stop
/// is left running, and the caller reports that the tab could not sleep.
pub(crate) fn stop_foreground_job(foreground_group: u32) -> StopOutcome {
    stop_foreground_job_with(foreground_group, &is_cli_agent, EXIT_TIMEOUT)
}

pub(crate) fn stop_foreground_job_with(
    foreground_group: u32,
    is_agent: &dyn Fn(&str) -> bool,
    exit_timeout: Duration,
) -> StopOutcome {
    let tree = process_tree(&running_processes(), foreground_group);
    match agent_process(&tree, is_agent) {
        Some(agent) => signal(agent.pid, Signal::SIGTERM),
        // No recognizable agent (a wrapper under another name, or the group leader has already
        // gone): ask the whole job.
        None => {
            let _ = killpg(Pid::from_raw(foreground_group as i32), Signal::SIGTERM);
        }
    }
    let Some(leader) = tree.first().cloned() else {
        return StopOutcome {
            exited: true,
            ..Default::default()
        };
    };
    let exited = wait_until(Instant::now() + exit_timeout, || {
        survivors(std::slice::from_ref(&leader)).is_empty()
    });
    if !exited {
        return StopOutcome::default();
    }

    std::thread::sleep(HELPER_SETTLE);
    let left = survivors(&tree);
    for helper in &left {
        signal(helper.pid, Signal::SIGTERM);
    }
    let mut killed = 0;
    if !left.is_empty()
        && !wait_until(Instant::now() + HELPER_STOP_TIMEOUT, || {
            survivors(&left).is_empty()
        })
    {
        for helper in survivors(&left) {
            signal(helper.pid, Signal::SIGKILL);
            killed += 1;
        }
    }
    StopOutcome {
        exited: true,
        helpers_stopped: left.len(),
        helpers_killed: killed,
    }
}

#[cfg(test)]
#[path = "agent_stop_tests.rs"]
mod tests;
