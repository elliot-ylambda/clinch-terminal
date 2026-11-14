use std::borrow::Cow;

use itertools::Itertools;
use lazy_static::lazy_static;
use memo_map::MemoMap;
use rand::Rng;
use warp_core::session_id::SessionId;
use warpui::{AppContext, AssetProvider, SingletonEntity};

#[cfg(feature = "local_fs")]
use super::{
    model::session::{BootstrapSessionType, SessionInfo},
    warpify::settings::{PIPENV_SUBSHELL_COMMAND_REGEX, POETRY_SUBSHELL_COMMAND_REGEX},
};
use crate::env_vars::{EnvVar, EnvVarExt};
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::shell::ShellType;

lazy_static! {
    /// A memoized cache of the fully-interpolated bootstrap script for each
    /// shell.  We store the full version here as an optimization so that we
    /// don't have to regenerate it every time we spawn a shell.
    static ref BOOTSTRAP_CACHE: MemoMap<ShellType, Vec<u8>> = Default::default();
}

/// This can sometimes appear in the beginning of files. If it gets written into the PTY, it causes
/// errors
const BYTE_ORDER_MARK: &str = "\u{FEFF}";

#[cfg(feature = "local_fs")]
pub fn is_container_subshell(session_info: &SessionInfo) -> bool {
    session_info.subshell_info.as_ref().is_some_and(|info| {
        let first_token = info
            .spawning_command
            .split_ascii_whitespace()
            .next()
            .unwrap_or("");
        first_token == "docker" || first_token == "podman"
    })
}

/// Returns `true` if Warp should use an RC-file based bootstrap (e.g. dump the bootstrap script to
/// a temp file and `source` it) for a newly spawned session with the given `shell_type`, and
/// associated `session_type` and `subshell_initialization_info`.
///
/// This returns `true` for local Fish/Pwsh shells and local subshells spawned via `poetry shell`.
///
/// We use RC-file based bootstrap for local Fish shells because there is a long-standing bug which
/// causes an explosion of formatting output when a command is longer than a screen height. (See
/// https://github.com/fish-shell/fish-shell/issues/7296 for more) This multiplication of output
/// makes our bootstrap take a long time, as we need to process all of that output (even though
/// most of it is irrelevant). To avoid the impact on bootstrap time, we write the script to a
/// temporary file and then source that file (which avoids writing the long script to the shell
/// itself).
///
/// We use RC-file based bootstrap for PowerShell because chars written to the PTY get randomly
/// ignored. See PLAT-757 in Linear.
///
/// We use RC-file based bootstrap for `poetry shell` subshells because the underlying library used
/// to spawn a subshell by `poetry shell` uses blocking PTY reads and writes, which results in a
/// deadlock when attempting to write the whole bootstrap script to the PTY; RC file-based
/// bootstrap is the only known way to bootstrap such subshells successfully.
///
/// We use RC-file based bootstrap for MSYS2 because it has slow PTY throughput.
#[cfg(feature = "local_fs")]
pub fn should_use_rc_file_bootstrap_method(
    shell_type: ShellType,
    session_info: &SessionInfo,
) -> bool {
    use super::ShellLaunchData;

    // Container subshells cannot access host temp files, so the RC-file
    // method is never viable for them.
    if is_container_subshell(session_info) {
        return false;
    }

    let session_type = &session_info.session_type;
    match session_type {
        BootstrapSessionType::Local => {
            let subshell_initialization_info = session_info.subshell_info.as_ref();
            let is_poetry_subshell = subshell_initialization_info
                .as_ref()
                .map(|info| POETRY_SUBSHELL_COMMAND_REGEX.is_match(info.spawning_command.as_str()))
                .unwrap_or(false);
            let is_pipenv_subshell = subshell_initialization_info
                .as_ref()
                .map(|info| PIPENV_SUBSHELL_COMMAND_REGEX.is_match(info.spawning_command.as_str()))
                .unwrap_or(false);
            let is_msys2 = session_info
                .launch_data
                .as_ref()
                .is_some_and(|data| matches!(data, ShellLaunchData::MSYS2 { .. }));
            shell_type == ShellType::Fish
                || shell_type == ShellType::PowerShell
                || is_poetry_subshell
                || ((is_pipenv_subshell
                    || (subshell_initialization_info.is_some() && cfg!(windows)))
                    && shell_type == ShellType::Zsh)
                || is_msys2
        }
        BootstrapSessionType::WarpifiedRemote => false,
    }
}

/// Returns the bootstrap script that should be used when initializing a shell
/// of the given type.
///
/// This supports a very basic form of interpolation:
///
/// ```shell
/// #include bundled/bootstrap/zsh_body.sh
/// ```
///
/// The directive above instructs this function to replace that line with the
/// contents of the file in our asset cache with the path `bundled/bootstrap/zsh_body.sh`.
///
/// Included files may include further files. `zsh.sh` and `bash.sh` include their
/// `*_body.sh`, which in turn include shared snippets such as `clinch_codex.sh`.
pub fn script_for_shell(shell_type: ShellType, assets: &dyn AssetProvider) -> Cow<'static, [u8]> {
    BOOTSTRAP_CACHE
        .get_or_insert(&shell_type, || assemble_script(shell_type, assets))
        .into()
}

/// Real assets nest two levels deep. The limit turns an accidental include cycle into a
/// clear startup panic instead of a stack overflow.
const MAX_INCLUDE_DEPTH: usize = 8;

/// Builds the bootstrap script for `shell_type` without consulting [`BOOTSTRAP_CACHE`].
fn assemble_script(shell_type: ShellType, assets: &dyn AssetProvider) -> Vec<u8> {
    let file = match shell_type {
        ShellType::Bash => "bash.sh",
        ShellType::Zsh => "zsh.sh",
        ShellType::Fish => "fish.sh",
        ShellType::PowerShell => "pwsh.ps1",
    };
    let bootstrap = read_bootstrap_asset(assets, &format!("bundled/bootstrap/{file}"));
    let included_files = MemoMap::new();
    let mut lines = Vec::new();
    push_expanded_lines(
        bootstrap.trim_start_matches(BYTE_ORDER_MARK),
        assets,
        &included_files,
        0,
        &mut lines,
    );

    // Filter out empty lines and comments, to minimize the amount
    // of data we send over the pty during the bootstrap process.
    let mut script = lines
        .into_iter()
        .filter(|line| {
            let line = line.trim_start();
            !(line.is_empty()
                || line.starts_with('#')
                || shell_type == ShellType::PowerShell
                    && line.starts_with("[Diagnostics.CodeAnalysis.SuppressMessageAttribute"))
        })
        .join("\n");

    // Make sure there's a newline at the end of the bootstrap script,
    // otherwise we'll never submit the final line to the shell.
    script.push('\n');
    script.into_bytes()
}

/// Appends the trimmed lines of `text`, replacing each `#include <path>` line with the
/// expanded lines of that asset. An unexpanded directive would otherwise be dropped
/// silently as a comment.
fn push_expanded_lines<'a>(
    text: &'a str,
    assets: &dyn AssetProvider,
    included_files: &'a MemoMap<String, String>,
    depth: usize,
    lines: &mut Vec<&'a str>,
) {
    for line in text.split('\n').map(trim_line) {
        let Some(path) = line.strip_prefix("#include ") else {
            lines.push(line);
            continue;
        };
        assert!(
            depth < MAX_INCLUDE_DEPTH,
            "bootstrap includes nest deeper than {MAX_INCLUDE_DEPTH} levels at {path}; check for an include cycle"
        );
        let included = included_files.get_or_insert(path, || {
            read_bootstrap_asset(assets, path)
                .replace("@@USING_CON_PTY_BOOLEAN@@", &cfg!(windows).to_string())
        });
        push_expanded_lines(included, assets, included_files, depth + 1, lines);
    }
}

/// Trims most leading and all trailing whitespace from a line. A single leading space is
/// kept, if one exists, to avoid interfering with histignorespace behavior. This minimizes
/// the number of bytes we send over the pty during the bootstrap process.
fn trim_line(line: &str) -> &str {
    let leading = line.len() - line.trim_start().len();
    line[leading.saturating_sub(1)..].trim_end()
}

fn read_bootstrap_asset(assets: &dyn AssetProvider, path: &str) -> String {
    let data = assets
        .get(path)
        .unwrap_or_else(|_| panic!("failed to retrieve {path} from assets"));
    // Interpret the file as UTF-8. We do this in an unchecked way for performance,
    // expecting that any issues here will be caught by unit tests.
    unsafe { String::from_utf8_unchecked(data.to_vec()) }
}

/// Generates a cryptographically random session ID for use as both a session
/// identifier and an integrity token for DCS hook validation.
pub fn generate_session_id() -> SessionId {
    let mut rng = rand::thread_rng();
    loop {
        let session_id = rng.gen::<u64>();
        if session_id != 0 {
            return SessionId::from(session_id);
        }
    }
}

/// Placeholder in init shell scripts that gets replaced with the client-generated session ID.
pub(crate) const SESSION_ID_PLACEHOLDER: &str = "@@WARP_SESSION_ID@@";

/// Returns the init shell script for the given `shell_type` (e.g. the script that emits the
/// InitShell DCS hook).
///
/// The returned script is one line and, for shells that need it, has escaped single-quotes for the
/// purposes of being passed as a single-quoted argument to 'eval'.
pub fn init_shell_script_for_shell(
    shell_type: ShellType,
    assets: &dyn AssetProvider,
    session_id: SessionId,
) -> String {
    let script = match shell_type {
        ShellType::Zsh => load_and_escape_script("bundled/bootstrap/zsh_init_shell.sh", assets),
        ShellType::Bash => load_and_escape_script("bundled/bootstrap/bash_init_shell.sh", assets),
        ShellType::Fish => load_and_escape_script("bundled/bootstrap/fish_init_shell.sh", assets),
        ShellType::PowerShell => load_script("bundled/bootstrap/pwsh_init_shell.ps1", assets),
    };
    script.replace(SESSION_ID_PLACEHOLDER, &session_id.as_u64().to_string())
}

/// Returns the command to be used to emit the InitShell hook for a new subshell session.
///
/// If `shell_type` is `Some()`, returns a shell type-specific command (e.g. valid command for
/// bash, fish, or zsh). Otherwise, returns a shell type-agnostic command that emits the right
/// `InitShell` hook based on the shell it is evaluated in.
pub fn init_subshell_command(
    shell_type: Option<ShellType>,
    vars: &[EnvVar],
    session_id: SessionId,
    ctx: &AppContext,
) -> String {
    match shell_type {
        Some(shell_type) => {
            let subshell_script =
                init_subshell_script_for_shell(shell_type, &crate::ASSETS, vars, session_id, ctx);
            format!(r#" [ -z $WARP_BOOTSTRAPPED ] && eval '{subshell_script}'"#)
        }
        None => init_subshell_script_for_unknown_shell(&crate::ASSETS, session_id),
    }
}

/// Returns the init subshell script for the given `shell_type` (e.g. the script that emits the
/// subshell version of the InitShell DCS hook).
///
/// The returned script is one line and has escaped single-quotes for the purposes of being passed
/// as a single-quoted argument to 'eval'.
fn init_subshell_script_for_shell(
    shell_type: ShellType,
    assets: &dyn AssetProvider,
    env_vars: &[EnvVar],
    session_id: SessionId,
    ctx: &AppContext,
) -> String {
    let honor_ps1 = *SessionSettings::as_ref(ctx).honor_ps1;
    let honor_ps1_env_var_value = if honor_ps1 { "1" } else { "0" };

    // Prepend environment variable settings to the script
    let env_setup_script = format!(
        "export WARP_HONOR_PS1={}; {}",
        honor_ps1_env_var_value,
        env_vars
            .iter()
            .map(|var| var.get_initialization_string(shell_type))
            .collect_vec()
            .join(" ")
    );

    // Load and escape the shell-specific init script
    let shell_init_script = match shell_type {
        ShellType::Zsh => load_and_escape_script("bundled/bootstrap/zsh_init_subshell.sh", assets),
        ShellType::Bash => {
            load_and_escape_script("bundled/bootstrap/bash_init_subshell.sh", assets)
        }
        ShellType::Fish => {
            load_and_escape_script("bundled/bootstrap/fish_init_subshell.sh", assets)
        }
        // TODO(PLAT-750)
        ShellType::PowerShell => todo!(),
    };
    let shell_init_script =
        shell_init_script.replace(SESSION_ID_PLACEHOLDER, &session_id.as_u64().to_string());

    // Combine the environment setup script with the shell-specific init script
    format!("{env_setup_script} {shell_init_script}")
}

/// Returns the init subshell script for an unknown shell which detects the shell type.
///
/// The returned script is one line and has escaped single-quotes for the purposes of being passed
/// as a single-quoted argument to 'eval'.
fn init_subshell_script_for_unknown_shell(
    assets: &dyn AssetProvider,
    session_id: SessionId,
) -> String {
    // Load and escape the shell-specific init script
    load_and_escape_script("bundled/bootstrap/unknown_init_subshell.sh", assets)
        .replace("HOOK_NAME", "InitSubshell")
        .replace(SESSION_ID_PLACEHOLDER, &session_id.as_u64().to_string())
}

/// Returns the raw init shell script for the given `shell_type`, without
/// single-quote escaping. Suitable for passing as an environment variable
/// where the caller controls the eval context (e.g. Docker sandbox init).
///
/// Gated on `unix` because the sole caller today is the Unix Docker
/// sandbox spawn path (`local_tty::unix::prepare_docker_sandbox`); on
/// Windows/wasm the function is dead code.
#[cfg(unix)]
pub fn raw_init_shell_script_for_shell(
    shell_type: ShellType,
    assets: &dyn AssetProvider,
    session_id: SessionId,
) -> String {
    let file = match shell_type {
        ShellType::Bash => "bundled/bootstrap/bash_init_shell.sh",
        ShellType::Zsh => "bundled/bootstrap/zsh_init_shell.sh",
        ShellType::Fish => "bundled/bootstrap/fish_init_shell.sh",
        ShellType::PowerShell => "bundled/bootstrap/pwsh_init_shell.ps1",
    };
    load_script(file, assets)
        .replace("@@USING_CON_PTY_BOOLEAN@@", &(cfg!(windows).to_string()))
        .replace(SESSION_ID_PLACEHOLDER, &session_id.as_u64().to_string())
}

/// Returns the script in the file at `file_path` to be passed as a single-quoted argument in the
/// shell (e.g. as a single quoted argument to `eval`).
///
/// The script is transformed in two ways:
///   * Newlines are stripped and replaced with semi-colons
///   * Single quotes are escaped (' is replaced with '"'"')
///   * Lines starting with '#' are removed -- this enables use of comments in scripts. Note,
///   however, that you still cannot use a 'partial line' comment, since this logic only considers
///   whole lines.
fn load_and_escape_script(file_path: &str, assets: &dyn AssetProvider) -> String {
    load_script(file_path, assets)
        .replace('\'', r#"'"'"'"#)
        .replace("@@USING_CON_PTY_BOOLEAN@@", &(cfg!(windows).to_string()))
}

fn load_script(file_path: &str, assets: &dyn AssetProvider) -> String {
    let script_bytes = assets
        .get(file_path)
        .unwrap_or_else(|_| panic!("Failed to retrieve {file_path} from assets"));

    std::str::from_utf8(&script_bytes)
        .expect("InitShell script should be utf8 encoded.")
        .trim_start_matches(BYTE_ORDER_MARK)
        .lines()
        .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
        .join(";")
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
