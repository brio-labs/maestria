use std::collections::BTreeMap;

use maestria_extensions::{
    CapabilityError, CapabilityFailure, CapabilityRequest, CapabilityResponse, CapabilitySuccess,
    FileSearchResult, FormValue, HostMessage, Manifest, ProtocolError, StorageOperation,
    WorkerMessage,
};
use serde_json::Value;

const EXAMPLE: &[u8] =
    include_bytes!("../../../../extension-sdk/examples/greetings.extension.json");

fn modified_example(
    mut change: impl FnMut(&mut Value),
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut manifest: Value = serde_json::from_slice(EXAMPLE)?;
    change(&mut manifest);
    Ok(serde_json::to_vec(&manifest)?)
}

#[test]
fn published_sdk_manifest_is_accepted_with_matching_commands_and_permissions()
-> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::parse(EXAMPLE)?;
    assert_eq!(manifest.id, "dev.sillage.greetings");
    assert_eq!(manifest.entrypoints[0].file, "dist/main.js");
    assert_eq!(manifest.commands[0].entrypoint_id, "main");
    Ok(())
}

#[test]
fn malformed_manifest_cannot_acquire_broader_capabilities_or_unchecked_entrypoints()
-> Result<(), Box<dyn std::error::Error>> {
    for file in ["dist/../secret.js", "dist/%2fsecret.js", "/tmp/code.js"] {
        let invalid = modified_example(|manifest| {
            manifest["entrypoints"][0]["file"] = Value::from(file);
        })?;
        assert!(
            Manifest::parse(&invalid).is_err(),
            "accepted unsafe entrypoint {file}"
        );
    }
    let wildcard_permission: Value =
        serde_json::from_str(r#"[{"type":"http","origins":["https://*.example.com"]}]"#)?;
    let wildcard = modified_example(|manifest| {
        manifest["permissions"] = wildcard_permission.clone();
    })?;
    assert!(Manifest::parse(&wildcard).is_err());
    let unknown_field = modified_example(|manifest| {
        manifest["surprise"] = Value::from("ignored");
    })?;
    assert!(Manifest::parse(&unknown_field).is_err());
    let undeclared = modified_example(|manifest| {
        manifest["commands"][0]["entrypointId"] = Value::from("missing");
    })?;
    assert!(Manifest::parse(&undeclared).is_err());
    Ok(())
}

#[test]
fn worker_views_are_strictly_versioned_and_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let valid: Value = serde_json::from_str(
        r#"{"protocolVersion":1,"kind":"view.update","commandId":"greetings","view":{"kind":"list","title":"Greetings","items":[{"id":"one","title":"One"}]}}"#,
    )?;
    let line = serde_json::to_vec(&valid)?;
    assert!(matches!(
        WorkerMessage::parse_line(&line)?,
        WorkerMessage::ViewUpdate { .. }
    ));
    let mut unknown = valid.clone();
    unknown["view"]["unexpected"] = Value::from(true);
    assert!(WorkerMessage::parse_line(&serde_json::to_vec(&unknown)?).is_err());
    let mut oversized = valid.clone();
    let mut items = Vec::with_capacity(101);
    for index in 0..101 {
        let mut item = valid["view"]["items"][0].clone();
        item["id"] = Value::from(format!("item{index}"));
        items.push(item);
    }
    oversized["view"]["items"] = Value::Array(items);
    assert!(WorkerMessage::parse_line(&serde_json::to_vec(&oversized)?).is_err());
    let mut old_version = valid;
    old_version["protocolVersion"] = Value::from(0);
    assert!(WorkerMessage::parse_line(&serde_json::to_vec(&old_version)?).is_err());
    Ok(())
}

#[test]
fn http_request_method_keeps_uppercase_sdk_wire_encoding() -> Result<(), Box<dyn std::error::Error>>
{
    let request: CapabilityRequest = serde_json::from_str(
        r#"{"capability":"http","url":"https://api.example.test","method":"GET"}"#,
    )?;
    assert_eq!(serde_json::to_value(request)?["method"], "GET");
    Ok(())
}

#[test]
fn worker_rejects_storage_shape_confusion_and_utf16_overlimit_titles()
-> Result<(), Box<dyn std::error::Error>> {
    let mut message: Value = serde_json::from_str(
        r#"{"protocolVersion":1,"kind":"capability.request","commandId":"greetings","requestId":"one","request":{"capability":"storage","operation":"get","key":"greeting","value":"unauthorized"}}"#,
    )?;
    assert!(WorkerMessage::parse_line(&serde_json::to_vec(&message)?).is_err());
    message["request"]["operation"] = Value::from("set");
    message["request"]
        .as_object_mut()
        .ok_or("missing request object")?
        .remove("value");
    assert!(WorkerMessage::parse_line(&serde_json::to_vec(&message)?).is_err());

    let mut view: Value = serde_json::from_str(
        r#"{"protocolVersion":1,"kind":"view.update","commandId":"greetings","view":{"kind":"list","title":"valid","items":[]}}"#,
    )?;
    view["view"]["title"] = Value::from("😀".repeat(61));
    assert!(WorkerMessage::parse_line(&serde_json::to_vec(&view)?).is_err());
    Ok(())
}

#[test]
fn host_pipe_enforces_form_and_capability_response_budgets()
-> Result<(), Box<dyn std::error::Error>> {
    let mut input = BTreeMap::new();
    input.insert("name".into(), FormValue::Text("Sillage".into()));
    let command = HostMessage::CommandInvoke {
        protocol_version: 1,
        command_id: "greetings".into(),
        input,
    };
    let line = command.encode_line()?;
    assert_eq!(line.last(), Some(&b'\n'));
    let parsed: HostMessage = serde_json::from_slice(&line[..line.len() - 1])?;
    assert!(matches!(parsed, HostMessage::CommandInvoke { .. }));

    let invalid_failure = HostMessage::CapabilityResponse {
        protocol_version: 1,
        request_id: "one".into(),
        response: CapabilityResponse::Failure(CapabilityFailure {
            ok: true,
            capability: "fileSearch".into(),
            error: CapabilityError {
                code: "permission_denied".into(),
                message: "Denied".into(),
            },
        }),
    };
    assert!(invalid_failure.encode_line().is_err());

    let results = (0..10)
        .map(|index| FileSearchResult {
            file_id: format!("result{index}"),
            title: "Document".into(),
            snippet: Some("😀".repeat(2048)),
        })
        .collect();
    let oversized = HostMessage::CapabilityResponse {
        protocol_version: 1,
        request_id: "one".into(),
        response: CapabilityResponse::Success(CapabilitySuccess::FileSearch { ok: true, results }),
    };
    assert!(matches!(
        oversized.encode_line(),
        Err(ProtocolError::ResponseOversized)
    ));
    Ok(())
}

#[test]
fn storage_responses_match_sdk_operation_specific_fields() -> Result<(), Box<dyn std::error::Error>>
{
    let get = HostMessage::CapabilityResponse {
        protocol_version: 1,
        request_id: "one".into(),
        response: CapabilityResponse::Success(CapabilitySuccess::Storage {
            ok: true,
            operation: StorageOperation::Get,
            value: None,
            completed: None,
        }),
    };
    let line = get.encode_line()?;
    let parsed: Value = serde_json::from_slice(&line[..line.len() - 1])?;
    assert_eq!(parsed["response"].get("value"), Some(&Value::Null));
    assert!(parsed["response"].get("completed").is_none());

    let set = HostMessage::CapabilityResponse {
        protocol_version: 1,
        request_id: "two".into(),
        response: CapabilityResponse::Success(CapabilitySuccess::Storage {
            ok: true,
            operation: StorageOperation::Set,
            value: None,
            completed: Some(true),
        }),
    };
    let line = set.encode_line()?;
    let parsed: Value = serde_json::from_slice(&line[..line.len() - 1])?;
    assert!(parsed["response"].get("value").is_none());
    assert_eq!(parsed["response"]["completed"], true);
    Ok(())
}
