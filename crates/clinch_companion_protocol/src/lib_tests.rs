use super::*;

fn target() -> TargetRef {
    TargetRef {
        app_instance_id: AppInstanceId::new(),
        project_id: "project-1".to_owned(),
        tab_id: "tab-1".to_owned(),
        pane_id: "pane-1".to_owned(),
    }
}

#[test]
fn validates_exact_target_composer_input() {
    let envelope = ClientEnvelope {
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        payload: ClientMessage::SubmitComposerText(SubmitComposerText {
            target: target(),
            workspace_revision: 7,
            text: "echo safe".to_owned(),
        }),
    };

    assert_eq!(envelope.validate(), Ok(()));
}

#[test]
fn rejects_missing_target_ids_and_oversized_input() {
    let mut invalid_target = target();
    invalid_target.pane_id.clear();
    let missing = ClientEnvelope {
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        payload: ClientMessage::SubmitComposerText(SubmitComposerText {
            target: invalid_target,
            workspace_revision: 7,
            text: "echo safe".to_owned(),
        }),
    };
    assert_eq!(
        missing.validate(),
        Err(ProtocolValidationError::InvalidOpaqueId("pane_id"))
    );

    let oversized = ClientEnvelope {
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        payload: ClientMessage::SubmitComposerText(SubmitComposerText {
            target: target(),
            workspace_revision: 7,
            text: "x".repeat(MAX_PROMPT_BYTES + 1),
        }),
    };
    assert!(matches!(
        oversized.validate(),
        Err(ProtocolValidationError::InvalidTextLength {
            field: "composer_text",
            ..
        })
    ));
}

#[test]
fn rejects_hostile_upload_metadata() {
    let envelope = ClientEnvelope {
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        payload: ClientMessage::UploadBegin(UploadBegin {
            target: target(),
            workspace_revision: 7,
            filename: "../secret".to_owned(),
            mime: "text/plain".to_owned(),
            size: 4,
            sha256: "0".repeat(64),
        }),
    };

    assert_eq!(
        envelope.validate(),
        Err(ProtocolValidationError::InvalidFilename)
    );
}

#[test]
fn upload_binary_frame_round_trips() {
    let upload_id = UploadId::new();
    let encoded = encode_upload_chunk(upload_id, 3, b"chunk").unwrap();
    let decoded = decode_upload_chunk(&encoded).unwrap();

    assert_eq!(decoded.upload_id, upload_id);
    assert_eq!(decoded.chunk_index, 3);
    assert_eq!(decoded.payload, b"chunk");
}

#[test]
fn terminal_output_binary_frame_round_trips_arbitrary_bytes() {
    let stream_id = TerminalStreamId::new();
    let bytes = [0, 0xff, b'\n', 0x1b];
    let encoded = encode_terminal_output(stream_id, 41, &bytes).unwrap();
    let decoded = decode_terminal_output(&encoded).unwrap();

    assert_eq!(decoded.stream_id, stream_id);
    assert_eq!(decoded.terminal_sequence, 41);
    assert_eq!(decoded.payload, bytes);
    assert!(matches!(
        decode_upload_chunk(&encoded),
        Err(BinaryFrameError::UnexpectedKind { .. })
    ));
}

#[test]
fn raw_input_requires_bounded_valid_base64() {
    let valid = ClientEnvelope {
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        payload: ClientMessage::RawTerminalInput(RawTerminalInput {
            target: target(),
            workspace_revision: 7,
            data_base64: BASE64_STANDARD.encode([0xff, 0, 3]),
        }),
    };
    assert_eq!(valid.validate(), Ok(()));

    let invalid = ClientEnvelope {
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        payload: ClientMessage::RawTerminalInput(RawTerminalInput {
            target: target(),
            workspace_revision: 7,
            data_base64: "not base64".to_owned(),
        }),
    };
    assert_eq!(
        invalid.validate(),
        Err(ProtocolValidationError::InvalidBase64Payload(
            "raw_terminal_input"
        ))
    );
}

#[test]
fn project_and_terminal_creation_allow_clinchs_default_directory() {
    let app_instance_id = AppInstanceId::new();
    for payload in [
        ClientMessage::CreateProject(CreateProject {
            app_instance_id,
            workspace_revision: 7,
            project_id: "project-1".to_owned(),
            cwd: None,
        }),
        ClientMessage::CreateSession(CreateSession {
            app_instance_id,
            workspace_revision: 7,
            project_id: "project-1".to_owned(),
            kind: SessionKind::Terminal,
            cwd: None,
            initial_prompt: None,
        }),
    ] {
        assert_eq!(
            ClientEnvelope {
                version: PROTOCOL_VERSION,
                request_id: RequestId::new(),
                payload,
            }
            .validate(),
            Ok(())
        );
    }
}

#[test]
fn binary_frame_rejects_oversized_chunks() {
    let error =
        encode_upload_chunk(UploadId::new(), 0, &vec![0; MAX_UPLOAD_CHUNK_BYTES + 1]).unwrap_err();
    assert_eq!(error, BinaryFrameError::ChunkTooLarge);
}

#[test]
fn protocol_schema_serializes() {
    let schema = schemars::schema_for!(CompanionProtocolSchema);
    let json = serde_json::to_string(&schema).unwrap();
    assert!(json.contains("ClientEnvelope"));
    assert!(json.contains("PairingClaimRequest"));
    assert!(json.contains("UploadBegin"));
}
