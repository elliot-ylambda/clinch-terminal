//! Shell completion generation for the local-control CLI.
use clap_complete::aot::{Shell, generate};
use local_control::protocol::{ControlError, ErrorCode};

use crate::local_control::ControlArgs;

pub(super) fn generate_completions_to_stdout(shell: Option<Shell>) -> Result<(), ControlError> {
    let shell = shell.or_else(Shell::from_env).ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            "could not determine shell from environment; provide a shell argument",
        )
    })?;
    let invocation = crate::binary_name().unwrap_or_else(|| "warpctrl".to_owned());
    let (mut cmd, bin_name) = completion_command_for_invocation(invocation);
    generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}

fn completion_command_for_invocation(invocation: String) -> (clap::Command, String) {
    if invocation == "clinch" || invocation.starts_with("clinch-") {
        let ctrl = ControlArgs::clap_command_for_bin_name(format!("{invocation} ctrl"))
            .name("ctrl")
            .display_name("ctrl");
        let command = clap::Command::new("clinch")
            .bin_name(invocation.clone())
            .about("Clinch command-line interface")
            .subcommand(ctrl);
        (command, invocation)
    } else {
        let bin_name = if invocation == "warpctrl" || invocation.starts_with("warpctrl-") {
            invocation
        } else {
            "warpctrl".to_owned()
        };
        (
            ControlArgs::clap_command_for_bin_name(bin_name.clone()),
            bin_name,
        )
    }
}

#[cfg(test)]
pub(crate) fn generate_completion_string(shell: Shell) -> Result<String, ControlError> {
    generate_completion_string_for_bin(shell, "warpctrl")
}

#[cfg(test)]
pub(crate) fn generate_completion_string_for_bin(
    shell: Shell,
    bin_name: &str,
) -> Result<String, ControlError> {
    let (mut cmd, bin_name) = completion_command_for_invocation(bin_name.to_owned());
    let mut output = Vec::new();
    generate(shell, &mut cmd, bin_name, &mut output);
    String::from_utf8(output).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to render local-control completions",
            err.to_string(),
        )
    })
}
