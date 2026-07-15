use std::collections::HashMap;
use std::io::{self, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use command::blocking::Command;
use futures::channel::oneshot;
use uuid::Uuid;
use warpui::r#async::{block_on, FutureExt as _};

use super::protocol::{
    BridgeCommand, BridgeEvent, BridgeRequest, BridgeResponse, BRIDGE_PROTOCOL_VERSION,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_LINE_BYTES: usize = 1_048_576;
const BRIDGE_EXECUTABLE: &str = "clinch-imessage-bridge";

#[derive(Clone, Debug)]
pub(crate) enum IMessageBridgeEvent {
    Message(BridgeEvent),
    Exited,
    ProtocolError,
}

pub(crate) struct IMessageBridge {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<BridgeResponse>>>>,
    events: async_channel::Receiver<IMessageBridgeEvent>,
}

impl IMessageBridge {
    pub(crate) fn spawn(path: &Path) -> Result<Self> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("start Messages bridge at {}", path.display()))?;
        let stdin = child.stdin.take().context("Messages bridge has no stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("Messages bridge has no stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("Messages bridge has no stderr")?;

        let pending = Arc::new(Mutex::new(
            HashMap::<String, oneshot::Sender<BridgeResponse>>::new(),
        ));
        let (event_tx, events) = async_channel::unbounded();
        let reader_pending = Arc::clone(&pending);
        let reader_event_tx = event_tx.clone();
        thread::Builder::new()
            .name("clinch-imessage-bridge-stdout".to_owned())
            .spawn(move || {
                let mut stdout = BufReader::new(stdout);
                loop {
                    let line = match read_bounded_line(&mut stdout) {
                        Ok(BoundedLine::Line(line)) => line,
                        Ok(BoundedLine::TooLong) => {
                            let _ =
                                block_on(reader_event_tx.send(IMessageBridgeEvent::ProtocolError));
                            continue;
                        }
                        Ok(BoundedLine::Eof) => break,
                        Err(_) => {
                            let _ =
                                block_on(reader_event_tx.send(IMessageBridgeEvent::ProtocolError));
                            break;
                        }
                    };
                    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
                        let _ = block_on(reader_event_tx.send(IMessageBridgeEvent::ProtocolError));
                        continue;
                    };
                    if value.get("event").is_some() {
                        match serde_json::from_value::<BridgeEvent>(value) {
                            Ok(event) => {
                                let _ = block_on(
                                    reader_event_tx.send(IMessageBridgeEvent::Message(event)),
                                );
                            }
                            Err(_) => {
                                let _ = block_on(
                                    reader_event_tx.send(IMessageBridgeEvent::ProtocolError),
                                );
                            }
                        }
                        continue;
                    }
                    let Ok(response) = serde_json::from_value::<BridgeResponse>(value) else {
                        let _ = block_on(reader_event_tx.send(IMessageBridgeEvent::ProtocolError));
                        continue;
                    };
                    let sender = reader_pending
                        .lock()
                        .ok()
                        .and_then(|mut pending| pending.remove(&response.id));
                    if let Some(sender) = sender {
                        let _ = sender.send(response);
                    }
                }
                if let Ok(mut pending) = reader_pending.lock() {
                    pending.clear();
                }
                let _ = block_on(reader_event_tx.send(IMessageBridgeEvent::Exited));
            })
            .context("start Messages bridge stdout reader")?;

        // Drain stderr so the child cannot block. Do not log its contents:
        // upstream errors may contain paths or snippets derived from Messages.
        thread::Builder::new()
            .name("clinch-imessage-bridge-stderr".to_owned())
            .spawn(move || {
                let mut emitted_warning = false;
                let mut stderr = stderr;
                let mut buffer = [0_u8; 8_192];
                loop {
                    let Ok(bytes_read) = stderr.read(&mut buffer) else {
                        break;
                    };
                    if bytes_read == 0 {
                        break;
                    }
                    if !emitted_warning {
                        emitted_warning = true;
                        log::warn!("Messages bridge emitted redacted diagnostic output");
                    }
                }
            })
            .context("start Messages bridge stderr reader")?;

        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            events,
        })
    }

    pub(crate) async fn request(&self, command: BridgeCommand) -> Result<BridgeResponse> {
        self.request_with_id(Uuid::new_v4().to_string(), command)
            .await
    }

    async fn request_with_id(&self, id: String, command: BridgeCommand) -> Result<BridgeResponse> {
        let request = BridgeRequest::new(id.clone(), command);
        let mut encoded = serde_json::to_vec(&request).context("encode Messages bridge request")?;
        encoded.push(b'\n');
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("Messages bridge request table is unavailable"))?
            .insert(id.clone(), sender);

        let write_result = self
            .stdin
            .lock()
            .map_err(|_| anyhow::anyhow!("Messages bridge input is unavailable"))?
            .write_all(&encoded);
        if let Err(error) = write_result {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(error).context("write Messages bridge request");
        }

        let response = receiver
            .with_timeout(REQUEST_TIMEOUT)
            .await
            .map_err(|_| anyhow::anyhow!("Messages bridge request timed out"))?
            .context("Messages bridge exited before responding")?;
        if response.version != BRIDGE_PROTOCOL_VERSION {
            anyhow::bail!(
                "Messages bridge returned unsupported protocol version {}",
                response.version
            );
        }
        Ok(response)
    }

    pub(crate) fn events(&self) -> async_channel::Receiver<IMessageBridgeEvent> {
        self.events.clone()
    }

    pub(crate) fn terminate(&self) {
        if let Ok(mut child) = self.child.lock() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

enum BoundedLine {
    Line(Vec<u8>),
    TooLong,
    Eof,
}

fn read_bounded_line(reader: &mut impl io::BufRead) -> io::Result<BoundedLine> {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if too_long {
                Ok(BoundedLine::TooLong)
            } else if line.is_empty() {
                Ok(BoundedLine::Eof)
            } else {
                Ok(BoundedLine::Line(line))
            };
        }

        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            if !too_long {
                if line.len().saturating_add(newline) > MAX_OUTPUT_LINE_BYTES {
                    too_long = true;
                } else {
                    line.extend_from_slice(&available[..newline]);
                }
            }
            reader.consume(newline + 1);
            return if too_long {
                Ok(BoundedLine::TooLong)
            } else {
                Ok(BoundedLine::Line(line))
            };
        }

        let chunk_len = available.len();
        if !too_long {
            if line.len().saturating_add(chunk_len) > MAX_OUTPUT_LINE_BYTES {
                too_long = true;
            } else {
                line.extend_from_slice(available);
            }
        }
        reader.consume(chunk_len);
    }
}

impl Drop for IMessageBridge {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub(crate) fn bridge_executable_path() -> Option<PathBuf> {
    if let Some(override_path) = std::env::var_os("CLINCH_IMESSAGE_BRIDGE_PATH") {
        return Some(PathBuf::from(override_path));
    }
    let bundle = PathBuf::from(warp_core::macos::get_bundle_path().ok()?);
    Some(bundle.join("Contents/Helpers").join(BRIDGE_EXECUTABLE))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use tempfile::tempdir;

    use super::*;
    use crate::imessage::protocol::BridgeResult;

    fn mock_bridge() -> (tempfile::TempDir, PathBuf) {
        let temp = tempdir().unwrap();
        let path = temp.path().join("mock-bridge");
        fs::write(
            &path,
            r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' '{"event":"permission_required","version":1,"permission":"automation"}'
  printf '%s\n' '{"version":1,"id":"test","ok":true,"result":{"type":"health","messages_running":true,"database_readable":true,"automation_authorized":true}}'
done
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        (temp, path)
    }

    #[test]
    fn request_response_and_event_are_correlated_without_logging_payloads() {
        futures::executor::block_on(async {
            let (_temp, path) = mock_bridge();
            let bridge = IMessageBridge::spawn(&path).unwrap();
            let response = bridge
                .request_with_id("test".to_owned(), BridgeCommand::Health)
                .await
                .unwrap();
            assert!(response.ok);
            assert!(matches!(response.result, Some(BridgeResult::Health { .. })));
            assert!(matches!(
                bridge.events().recv().await.unwrap(),
                IMessageBridgeEvent::Message(BridgeEvent::PermissionRequired { .. })
            ));
        });
    }

    #[test]
    fn oversized_helper_lines_are_discarded_without_losing_the_next_event() {
        let mut input = vec![b'x'; MAX_OUTPUT_LINE_BYTES + 1];
        input.extend_from_slice(b"\n{}\n");
        let mut reader = Cursor::new(input);
        assert!(matches!(
            read_bounded_line(&mut reader).unwrap(),
            BoundedLine::TooLong
        ));
        let BoundedLine::Line(next) = read_bounded_line(&mut reader).unwrap() else {
            panic!("expected the next complete protocol line");
        };
        assert_eq!(next, b"{}");
    }
}
