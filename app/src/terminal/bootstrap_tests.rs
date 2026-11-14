use super::*;

struct TestAssetProvider;

impl AssetProvider for TestAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "bundled/bootstrap/fish.sh" => "# this is a comment\nthis_is_a_command",
            "bundled/bootstrap/zsh.sh" => {
                "asdf\n#include whitespace\n    prepended whitespace\n\n\n"
            }
            "bundled/bootstrap/pwsh.ps1" => {
                r#"# This is a comment
                Write-Output 'Testing some output'
                function test1 {
                    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
                    param([string]$command)
                    Invoke-Expression $command
                }"#
            }
            "hello_world" => "hello world!",
            "whitespace" => "no whitespace\n\n\n yes whitespace!",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

#[test]
fn test_include_directive() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &TestAssetProvider)),
        "hello world!\n"
    );
}

#[test]
fn test_trims_comments() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Fish, &TestAssetProvider)),
        "this_is_a_command\n"
    );
}

#[test]
fn test_trims_whitespace() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Zsh, &TestAssetProvider)),
        "asdf\nno whitespace\n yes whitespace!\n prepended whitespace\n"
    );
}

#[test]
fn test_trims_powershell_specifics() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::PowerShell, &TestAssetProvider)),
        " Write-Output 'Testing some output'\n function test1 {\n param([string]$command)\n Invoke-Expression $command\n }\n"
    );
}

/// Nested and cyclic includes. Tests that use it call [`assemble_script`] directly so they
/// never share [`BOOTSTRAP_CACHE`] with the tests above.
struct NestedAssetProvider;

impl AssetProvider for NestedAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/zsh.sh" => "before\n#include body\nafter",
            "body" => "  body start\n#include snippet\n# body comment\nbody end",
            "snippet" => "    snippet uses ConPTY: @@USING_CON_PTY_BOOLEAN@@",
            "bundled/bootstrap/bash.sh" => "#include cycle",
            "cycle" => "#include cycle",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

#[test]
fn test_include_directive_expands_includes_inside_included_files() {
    assert_eq!(
        decode_script(&assemble_script(ShellType::Zsh, &NestedAssetProvider)),
        format!(
            "before\n body start\n snippet uses ConPTY: {}\nbody end\nafter\n",
            cfg!(windows)
        )
    );
}

#[test]
#[should_panic(expected = "check for an include cycle")]
fn test_include_cycle_panics_instead_of_overflowing_the_stack() {
    assemble_script(ShellType::Bash, &NestedAssetProvider);
}

/// `zsh_body.sh` and `bash_body.sh` include `clinch_codex.sh` one level below the top-level
/// script. Before includes nested, that line was dropped as a comment, so `codex` and the
/// user's aliases for it skipped `--no-daemon` and its hooks could reach another tab.
#[test]
fn test_bundled_bootstraps_define_the_pane_local_codex_wrapper() {
    for (shell, definition) in [
        (ShellType::Zsh, "function codex() {"),
        (ShellType::Bash, "function codex() {"),
        (ShellType::Fish, "function codex"),
    ] {
        let script = assemble_script(shell, &crate::ASSETS);
        let script = decode_script(&script);
        assert!(
            script.lines().any(|line| line.trim() == definition),
            "the {shell:?} bootstrap must define the Codex wrapper"
        );
        assert!(
            script.contains("/clinch_codex_launch"),
            "the {shell:?} bootstrap must route Codex through the pane-local launcher"
        );
    }
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
