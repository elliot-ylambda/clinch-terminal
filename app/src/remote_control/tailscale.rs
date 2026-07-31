//! Minimal, injectable adapter for configuring one path-scoped Tailscale Serve route.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

const STANDALONE_CLI: &str = "/usr/local/bin/tailscale";
const APP_BUNDLED_CLI: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_COMMAND_OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TailscaleInstallation {
    pub executable: PathBuf,
    pub force_be_cli: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TailscaleServeReady {
    pub base_url: String,
    pub route_path: String,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TailscaleSetupOutcome {
    Ready(TailscaleServeReady),
    Stopped,
    SignInRequired { action_url: Option<String> },
    ConsentRequired { action_url: Option<String> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Error)]
pub enum TailscaleError {
    #[error("Tailscale is not installed in a supported location")]
    NotInstalled,
    #[error("Tailscale command timed out")]
    TimedOut,
    #[error("Tailscale command output exceeded the safety limit")]
    OutputTooLarge,
    #[error("could not run Tailscale: {0}")]
    Command(String),
    #[error("Tailscale returned invalid status JSON")]
    InvalidStatus,
    #[error("Tailscale Serve did not retain Clinch's private route")]
    RouteVerificationFailed,
}

#[async_trait]
pub trait TailscaleCommandRunner: Send + Sync {
    async fn run(
        &self,
        installation: &TailscaleInstallation,
        args: &[String],
    ) -> Result<CommandOutput, TailscaleError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RealTailscaleCommandRunner;

#[async_trait]
impl TailscaleCommandRunner for RealTailscaleCommandRunner {
    async fn run(
        &self,
        installation: &TailscaleInstallation,
        args: &[String],
    ) -> Result<CommandOutput, TailscaleError> {
        let mut command = command::r#async::Command::new(&installation.executable);
        command.args(args).kill_on_drop(true);
        if installation.force_be_cli {
            command.env("TAILSCALE_BE_CLI", "1");
        }

        let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
            .await
            .map_err(|_| TailscaleError::TimedOut)?
            .map_err(|error| TailscaleError::Command(error.to_string()))?;
        if output.stdout.len().saturating_add(output.stderr.len()) > MAX_COMMAND_OUTPUT_BYTES {
            return Err(TailscaleError::OutputTooLarge);
        }
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct TailscaleClient<R = RealTailscaleCommandRunner> {
    installation: TailscaleInstallation,
    runner: R,
}

impl TailscaleClient<RealTailscaleCommandRunner> {
    pub fn discover() -> Result<Self, TailscaleError> {
        let installation = discover_installation(&[
            (Path::new(STANDALONE_CLI), false),
            (Path::new(APP_BUNDLED_CLI), true),
        ])
        .ok_or(TailscaleError::NotInstalled)?;
        Ok(Self {
            installation,
            runner: RealTailscaleCommandRunner,
        })
    }
}

impl<R: TailscaleCommandRunner> TailscaleClient<R> {
    #[cfg(test)]
    fn with_runner(installation: TailscaleInstallation, runner: R) -> Self {
        Self {
            installation,
            runner,
        }
    }

    pub async fn configure_private_route(
        &self,
        route_path: &str,
        loopback_port: u16,
    ) -> Result<TailscaleSetupOutcome, TailscaleError> {
        validate_route_path(route_path)?;
        let node = self.node_status().await?;
        if node.backend_state == "Stopped" {
            return Ok(TailscaleSetupOutcome::Stopped);
        }
        if node.backend_state != "Running"
            || !node.self_node.as_ref().is_some_and(|node| node.online)
        {
            return Ok(TailscaleSetupOutcome::SignInRequired {
                action_url: node.action_url,
            });
        }
        let dns_name = node
            .self_node
            .and_then(|node| node.dns_name)
            .filter(|name| !name.trim_matches('.').is_empty())
            .ok_or(TailscaleError::InvalidStatus)?;
        let target = format!("http://127.0.0.1:{loopback_port}");
        let args = serve_enable_args(route_path, &target);
        let output = self.runner.run(&self.installation, &args).await?;
        if !output.success {
            let action_url = extract_https_url(&format!("{}\n{}", output.stdout, output.stderr));
            return action_url
                .map(|action_url| TailscaleSetupOutcome::ConsentRequired {
                    action_url: Some(action_url),
                })
                .ok_or_else(|| TailscaleError::Command(non_secret_command_error(&output)));
        }

        let status = self
            .runner
            .run(
                &self.installation,
                &["serve".to_owned(), "status".to_owned(), "--json".to_owned()],
            )
            .await?;
        if !status.success || !serve_status_has_route(&status.stdout, route_path, &target) {
            return Err(TailscaleError::RouteVerificationFailed);
        }

        Ok(TailscaleSetupOutcome::Ready(TailscaleServeReady {
            base_url: format!("https://{}", dns_name.trim_end_matches('.')),
            route_path: route_path.to_owned(),
            target,
        }))
    }

    /// Removes only Clinch's mount. It deliberately never invokes `serve off` without a path or
    /// `serve reset`, because either would destroy unrelated user configuration.
    pub async fn remove_private_route(&self, route_path: &str) -> Result<(), TailscaleError> {
        validate_route_path(route_path)?;
        let output = self
            .runner
            .run(&self.installation, &serve_disable_args(route_path))
            .await?;
        if output.success {
            Ok(())
        } else {
            Err(TailscaleError::Command(non_secret_command_error(&output)))
        }
    }

    async fn node_status(&self) -> Result<NodeStatus, TailscaleError> {
        let output = self
            .runner
            .run(
                &self.installation,
                &["status".to_owned(), "--json".to_owned()],
            )
            .await?;
        if output.success {
            serde_json::from_str(&output.stdout).map_err(|_| TailscaleError::InvalidStatus)
        } else {
            let action_url = extract_https_url(&format!("{}\n{}", output.stdout, output.stderr));
            Ok(NodeStatus {
                backend_state: if action_url.is_some() {
                    "NeedsLogin".to_owned()
                } else {
                    "Stopped".to_owned()
                },
                self_node: None,
                action_url,
            })
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NodeStatus {
    #[serde(default)]
    backend_state: String,
    #[serde(rename = "Self")]
    self_node: Option<SelfNode>,
    #[serde(default, alias = "AuthURL", alias = "LoginURL")]
    action_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SelfNode {
    #[serde(default)]
    online: bool,
    #[serde(rename = "DNSName", alias = "DnsName")]
    dns_name: Option<String>,
}

fn discover_installation(candidates: &[(&Path, bool)]) -> Option<TailscaleInstallation> {
    candidates.iter().find_map(|(path, force_be_cli)| {
        crate::util::path::file_exists_and_is_executable(path).then(|| TailscaleInstallation {
            executable: (*path).to_path_buf(),
            force_be_cli: *force_be_cli,
        })
    })
}

fn validate_route_path(route_path: &str) -> Result<(), TailscaleError> {
    if !route_path.starts_with("/clinch-remote-")
        || route_path.len() > 128
        || route_path[1..]
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    {
        return Err(TailscaleError::Command(
            "invalid Clinch Serve route".to_owned(),
        ));
    }
    Ok(())
}

fn serve_enable_args(route_path: &str, target: &str) -> Vec<String> {
    vec![
        "serve".to_owned(),
        "--bg".to_owned(),
        "--yes".to_owned(),
        "--https=443".to_owned(),
        format!("--set-path={route_path}"),
        target.to_owned(),
    ]
}

fn serve_disable_args(route_path: &str) -> Vec<String> {
    vec![
        "serve".to_owned(),
        "--bg".to_owned(),
        "--yes".to_owned(),
        "--https=443".to_owned(),
        format!("--set-path={route_path}"),
        "off".to_owned(),
    ]
}

fn serve_status_has_route(json: &str, route_path: &str, target: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return false;
    };
    json_value_has_route_target(&value, route_path, target)
}

fn json_value_has_route_target(value: &Value, route_path: &str, target: &str) -> bool {
    match value {
        Value::Array(values) => values
            .iter()
            .any(|value| json_value_has_route_target(value, route_path, target)),
        Value::Object(values) => {
            // Current `tailscale serve status --json` uses the route as a Handlers map key.
            // Also accept a future descriptor object with an explicit Path field, but require the
            // target to live inside that same route descriptor. Merely finding both strings in
            // unrelated Serve entries must never make Clinch report Ready.
            values
                .iter()
                .any(|(key, handler)| key == route_path && json_value_contains(handler, target))
                || (values
                    .get("Path")
                    .is_some_and(|path| path.as_str() == Some(route_path))
                    && json_value_contains(value, target))
                || values
                    .values()
                    .any(|value| json_value_has_route_target(value, route_path, target))
        }
        _ => false,
    }
}

fn json_value_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value == needle || value.contains(needle),
        Value::Array(values) => values
            .iter()
            .any(|value| json_value_contains(value, needle)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(needle) || json_value_contains(value, needle)),
        _ => false,
    }
}

fn extract_https_url(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|word| {
        let candidate = word.trim_matches(|character: char| {
            matches!(character, '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';')
        });
        candidate
            .starts_with("https://")
            .then(|| candidate.to_owned())
    })
}

fn non_secret_command_error(output: &CommandOutput) -> String {
    let message = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    message.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<Vec<String>>>,
        outputs: Mutex<VecDeque<CommandOutput>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: Vec<CommandOutput>) -> Self {
            Self {
                calls: Mutex::default(),
                outputs: Mutex::new(outputs.into()),
            }
        }
    }

    #[async_trait]
    impl TailscaleCommandRunner for FakeRunner {
        async fn run(
            &self,
            _: &TailscaleInstallation,
            args: &[String],
        ) -> Result<CommandOutput, TailscaleError> {
            self.calls.lock().unwrap().push(args.to_vec());
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| TailscaleError::Command("missing fake output".to_owned()))
        }
    }

    fn output(success: bool, stdout: &str) -> CommandOutput {
        CommandOutput {
            success,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }

    fn client(runner: FakeRunner) -> TailscaleClient<FakeRunner> {
        TailscaleClient::with_runner(
            TailscaleInstallation {
                executable: PathBuf::from(STANDALONE_CLI),
                force_be_cli: false,
            },
            runner,
        )
    }

    #[tokio::test]
    async fn configures_and_verifies_only_the_clinch_path() {
        let route = "/clinch-remote-0123456789abcdef01234567";
        let target = "http://127.0.0.1:4567";
        let runner = FakeRunner::with_outputs(vec![
            output(
                true,
                r#"{"BackendState":"Running","Self":{"Online":true,"DNSName":"mac.tail.ts.net."}}"#,
            ),
            output(true, "configured"),
            output(
                true,
                &format!(
                    r#"{{"Web":{{"mac:443":{{"Handlers":{{"{route}":{{"Proxy":"{target}"}}}}}}}}}}"#
                ),
            ),
        ]);
        let client = client(runner);

        assert_eq!(
            client.configure_private_route(route, 4567).await.unwrap(),
            TailscaleSetupOutcome::Ready(TailscaleServeReady {
                base_url: "https://mac.tail.ts.net".to_owned(),
                route_path: route.to_owned(),
                target: target.to_owned(),
            })
        );
        let calls = client.runner.calls.lock().unwrap();
        assert!(calls[1].contains(&format!("--set-path={route}")));
        assert!(!calls
            .iter()
            .flatten()
            .any(|arg| arg == "funnel" || arg == "reset"));

        assert!(!serve_status_has_route(
            &format!(
                r#"{{"Web":{{"mac:443":{{"Handlers":{{"{route}":{{"Proxy":"http://127.0.0.1:9999"}},"/somewhere-else":{{"Proxy":"{target}"}}}}}}}}}}"#
            ),
            route,
            target,
        ));
    }

    #[tokio::test]
    async fn cleanup_cannot_clear_unrelated_serve_configuration() {
        let route = "/clinch-remote-0123456789abcdef01234567";
        let client = client(FakeRunner::with_outputs(vec![output(true, "removed")]));
        client.remove_private_route(route).await.unwrap();

        let calls = client.runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(&format!("--set-path={route}")));
        assert_eq!(calls[0].last().map(String::as_str), Some("off"));
        assert!(!calls[0].iter().any(|arg| arg == "reset"));
    }

    #[tokio::test]
    async fn reports_sign_in_and_consent_urls_without_enabling_funnel() {
        let route = "/clinch-remote-0123456789abcdef01234567";
        let signed_out = client(FakeRunner::with_outputs(vec![output(
            false,
            "Sign in at https://login.tailscale.com/a/example",
        )]));
        assert_eq!(
            signed_out
                .configure_private_route(route, 4567)
                .await
                .unwrap(),
            TailscaleSetupOutcome::SignInRequired {
                action_url: Some("https://login.tailscale.com/a/example".to_owned())
            }
        );

        let consent = client(FakeRunner::with_outputs(vec![
            output(
                true,
                r#"{"BackendState":"Running","Self":{"Online":true,"DNSName":"mac.tail.ts.net"}}"#,
            ),
            output(
                false,
                "Enable HTTPS at https://login.tailscale.com/admin/dns",
            ),
        ]));
        assert!(matches!(
            consent.configure_private_route(route, 4567).await.unwrap(),
            TailscaleSetupOutcome::ConsentRequired {
                action_url: Some(_)
            }
        ));
    }

    #[tokio::test]
    async fn distinguishes_a_stopped_daemon_from_signed_out() {
        let route = "/clinch-remote-0123456789abcdef01234567";
        let stopped = client(FakeRunner::with_outputs(vec![output(
            false,
            "Tailscale is stopped",
        )]));
        assert_eq!(
            stopped.configure_private_route(route, 4567).await.unwrap(),
            TailscaleSetupOutcome::Stopped
        );
    }
}
