use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::Stdio;

use command::blocking::Command;

use super::*;

/// The test jobs run under `/bin/sh`, which macOS may report under the name of the shell behind it.
fn is_test_shell(name: &str) -> bool {
    matches!(name, "sh" | "bash" | "dash" | "zsh")
}

fn process(pid: u32, parent: u32, name: &str) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent: Some(parent),
        name: name.to_owned(),
        started: 0,
    }
}

#[test]
fn a_process_tree_is_the_job_and_everything_under_it() {
    let processes = vec![
        process(1, 0, "launchd"),
        process(100, 1, "zsh"),
        process(200, 100, "zsh"),
        process(300, 200, "claude"),
        process(400, 300, "node"),
        process(401, 300, "npm"),
        process(500, 401, "node"),
        // Another tab's agent, and a process that only shares a name.
        process(600, 1, "zsh"),
        process(700, 600, "claude"),
    ];
    let pids: Vec<u32> = process_tree(&processes, 200)
        .into_iter()
        .map(|process| process.pid)
        .collect();
    assert_eq!(pids, vec![200, 300, 400, 401, 500]);
    assert!(
        process_tree(&processes, 999).is_empty(),
        "a job that is gone"
    );
}

#[test]
fn the_agent_is_the_outermost_one_below_its_launcher() {
    let tree = vec![
        process(200, 100, "zsh"),
        process(300, 200, "claude"),
        // A nested agent the outer one started as a tool.
        process(400, 300, "codex"),
    ];
    assert_eq!(
        agent_process(&tree, &is_cli_agent).map(|p| p.pid),
        Some(300)
    );

    let wrapped = vec![process(200, 100, "zsh"), process(300, 200, "node")];
    assert_eq!(
        agent_process(&wrapped, &is_cli_agent),
        None,
        "an agent under another name is stopped through its whole job instead"
    );
}

/// Starts `script` under `/bin/sh` in its own process group, the way a shell starts a job, and
/// reaps it on another thread the way that shell would.
fn spawn_job(script: &str) -> u32 {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().expect("spawn test job");
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    pid
}

fn wait_for_children(parent: u32, count: usize) -> Vec<ProcessInfo> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let tree = process_tree(&running_processes(), parent);
        if tree.len() > count || Instant::now() >= deadline {
            return tree;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Codex leaves its helpers to notice on their own that it has gone. Whatever is still running
/// once the agent has exited is told to stop, so an asleep tab holds no processes.
#[test]
fn helpers_that_outlive_the_agent_are_stopped() {
    // The "agent" is the shell itself; two helpers run under it and ignore its exit.
    let job = spawn_job("sleep 300 & sleep 301 & wait");
    let tree = wait_for_children(job, 2);
    assert_eq!(tree.len(), 3, "the job and its two helpers: {tree:?}");

    let outcome = stop_foreground_job_with(job, &is_test_shell, Duration::from_secs(10));
    assert!(outcome.exited);
    assert_eq!(outcome.helpers_stopped, 2);
    assert_eq!(outcome.helpers_killed, 0);
    assert!(survivors(&tree).is_empty(), "nothing is left running");
}

/// A helper that ignores being asked is killed, because nothing else will ever end it.
#[test]
fn a_helper_that_ignores_the_request_is_killed() {
    let job = spawn_job("(trap '' TERM; sleep 300) & wait");
    let tree = wait_for_children(job, 1);
    assert!(tree.len() >= 2, "the job and its helper: {tree:?}");

    let outcome = stop_foreground_job_with(job, &is_test_shell, Duration::from_secs(10));
    assert!(outcome.exited);
    assert!(outcome.helpers_killed >= 1, "{outcome:?}");
    assert!(survivors(&tree).is_empty(), "nothing is left running");
}

/// An agent that will not stop is left alone, and the caller hears that it is still running.
#[test]
fn an_agent_that_will_not_stop_is_left_running() {
    let job = spawn_job("trap '' TERM; sleep 300 & wait");
    let tree = wait_for_children(job, 1);
    let outcome = stop_foreground_job_with(job, &is_test_shell, Duration::from_millis(600));
    assert_eq!(outcome, StopOutcome::default());
    assert!(
        !survivors(&tree[..1]).is_empty(),
        "the job itself is never killed"
    );
    // Clean up what this test started.
    for process in &tree {
        signal(process.pid, Signal::SIGKILL);
    }
}

/// The real thing a pane has: a shell with job control on a PTY. The foreground group is the job
/// the shell started, found from the PTY's controlling end, and never the shell itself.
#[test]
fn the_foreground_job_of_a_pty_is_what_the_shell_is_running() {
    let (mut leader, mut follower) = (0, 0);
    let opened = unsafe {
        libc::openpty(
            &mut leader,
            &mut follower,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(opened, 0, "openpty");
    let leader = unsafe { OwnedFd::from_raw_fd(leader) };
    let follower = unsafe { OwnedFd::from_raw_fd(follower) };
    // The shell must not inherit the controlling end: a pane's shell never holds it.
    unsafe { libc::fcntl(leader.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    let terminal = follower.as_raw_fd();
    let mut command = Command::new("/bin/zsh");
    command
        // An interactive shell without the user's startup files: job control, nothing else.
        .args(["-f", "-i", "-c", "sleep 300; exit 0"])
        .stdin(Stdio::from(follower.try_clone().unwrap()))
        .stdout(Stdio::from(follower.try_clone().unwrap()))
        .stderr(Stdio::from(follower.try_clone().unwrap()));
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 || libc::ioctl(terminal, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut shell = command.spawn().expect("spawn shell on a pty");
    drop(follower);
    let shell_pid = shell.id();
    // Read what the shell prints, as a terminal does. A shell whose output nobody reads can't
    // finish closing its terminal, and never exits.
    let mut screen = std::fs::File::from(leader.try_clone().unwrap());
    std::thread::spawn(move || {
        let mut buffer = [0; 4096];
        while matches!(std::io::Read::read(&mut screen, &mut buffer), Ok(read) if read > 0) {}
    });

    let deadline = Instant::now() + Duration::from_secs(15);
    let job = loop {
        if let Some(job) = foreground_job_group(leader.as_raw_fd(), shell_pid) {
            break job;
        }
        assert!(Instant::now() < deadline, "the shell never started its job");
        std::thread::sleep(POLL_INTERVAL);
    };
    let tree = process_tree(&running_processes(), job);
    assert_eq!(
        tree.first().map(|process| process.name.as_str()),
        Some("sleep"),
        "the foreground job is the command, not the shell"
    );

    // Stopping the job hands the terminal back to the shell, which then exits by itself.
    let outcome = stop_foreground_job_with(job, &|name| name == "sleep", Duration::from_secs(10));
    assert!(outcome.exited, "{outcome:?}");
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = shell.try_wait().expect("wait for the shell") {
            break status;
        }
        if Instant::now() >= deadline {
            let left = process_tree(&running_processes(), shell_pid);
            let _ = shell.kill();
            panic!("the shell did not carry on after its job stopped; still running: {left:?}");
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    assert!(
        status.success(),
        "the shell carried on after its job: {status:?}"
    );
}
