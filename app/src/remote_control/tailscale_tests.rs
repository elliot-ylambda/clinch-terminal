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
async fn first_time_setup_surfaces_serve_consent_without_launching_serve() {
    let route = "/clinch-remote-0123456789abcdef01234567";
    let client = client(FakeRunner::with_outputs(vec![output(
        true,
        r#"{"BackendState":"Running","CertDomains":null,"Self":{"ID":"nRi8c6fHQP11CNTRL","Online":true,"DNSName":"mac.tail.ts.net"}}"#,
    )]));

    assert_eq!(
        client.configure_private_route(route, 4567).await.unwrap(),
        TailscaleSetupOutcome::ConsentRequired {
            action_url: Some(
                "https://login.tailscale.com/f/serve?node=nRi8c6fHQP11CNTRL".to_owned()
            ),
        }
    );
    let calls = client.runner.calls.lock().unwrap();
    assert_eq!(
        calls.as_slice(),
        &[vec!["status".to_owned(), "--json".to_owned()]]
    );
}

#[tokio::test]
async fn first_time_setup_rejects_an_unsafe_node_id() {
    let route = "/clinch-remote-0123456789abcdef01234567";
    let client = client(FakeRunner::with_outputs(vec![output(
        true,
        r#"{"BackendState":"Running","CertDomains":[],"Self":{"ID":"bad&node=other","Online":true,"DNSName":"mac.tail.ts.net"}}"#,
    )]));

    assert_eq!(
        client.configure_private_route(route, 4567).await.unwrap(),
        TailscaleSetupOutcome::ConsentRequired { action_url: None }
    );
    assert_eq!(client.runner.calls.lock().unwrap().len(), 1);
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
