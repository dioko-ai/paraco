use super::{Binding, Installation, Remote};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    super::migrate(&mut db).unwrap();
    db
}

fn shared(id: &str, write: bool) -> Installation {
    Installation {
        datasets: BTreeMap::from([(
            "data".into(),
            Binding {
                id: id.into(),
                write,
            },
        )]),
        ..Installation::default()
    }
}

fn call(
    db: &mut Connection,
    installation: &str,
    application: &str,
    config: &Installation,
    request: Value,
) -> Result<Value, String> {
    super::execute(db, installation, application, &json!({}), config, &request)
}

fn put_record(
    db: &mut Connection,
    installation: &str,
    config: &Installation,
    expected: Value,
    value: Value,
) -> Result<Value, String> {
    call(
        db,
        installation,
        "app",
        config,
        json!({"dataset":"private","action":"record.put","key":"item","expectedVersion":expected,"value":value}),
    )
}

fn publish(
    db: &mut Connection,
    installation: &str,
    application: &str,
    config: &Installation,
    draft: Value,
) -> Result<Value, String> {
    call(
        db,
        installation,
        application,
        config,
        json!({"dataset":"data","action":"artifact.publish","draft":draft}),
    )
}

fn draft(content: Value, idempotency: &str) -> Value {
    json!({
        "output":"report",
        "mediaType":"application/json",
        "content":content,
        "idempotencyKey":idempotency
    })
}

fn artifact_id(value: &Value) -> &str {
    value["id"].as_str().unwrap()
}

fn remote_server(
    status: u16,
    response: &'static str,
) -> (
    String,
    std::sync::mpsc::Receiver<(String, String, String)>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, received) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        let mut header = String::new();
        let mut authorization = String::new();
        reader.read_line(&mut request).unwrap();
        let mut content_length = 0;
        loop {
            header.clear();
            reader.read_line(&mut header).unwrap();
            if header == "\r\n" || header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().unwrap();
                } else if name.eq_ignore_ascii_case("authorization") {
                    authorization = value.trim().to_owned();
                }
            }
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();
        let body = String::from_utf8(body).unwrap();
        sender.send((request, authorization, body)).unwrap();
        let response_bytes = response.as_bytes();
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response_bytes.len()).unwrap();
        stream.write_all(response_bytes).unwrap();
    });
    (format!("http://{address}/platform"), received, worker)
}

fn remote_installation(url: String) -> Installation {
    Installation {
        datasets: BTreeMap::from([(
            "data".into(),
            Binding {
                id: "shared-dataset".into(),
                write: true,
            },
        )]),
        remote: Some(Remote {
            url,
            token: "scoped-secret".into(),
        }),
    }
}

#[test]
fn private_records_are_isolated_per_installation() {
    let mut db = database();
    let config = Installation::default();
    put_record(&mut db, "one", &config, Value::Null, json!({"secret":1})).unwrap();

    let one = call(
        &mut db,
        "one",
        "app",
        &config,
        json!({"dataset":"private","action":"record.get","key":"item"}),
    )
    .unwrap();
    let two = call(
        &mut db,
        "two",
        "app",
        &config,
        json!({"dataset":"private","action":"record.get","key":"item"}),
    )
    .unwrap();
    assert_eq!(one["value"], json!({"secret":1}));
    assert_eq!(two, Value::Null);
}

#[test]
fn explicit_shared_binding_shares_data_across_installations_and_read_only_denies_writes() {
    let mut db = database();
    let writer = shared("team-dataset", true);
    let reader = shared("team-dataset", false);
    call(
        &mut db,
        "one",
        "writer",
        &writer,
        json!({"dataset":"data","action":"record.put","key":"item","expectedVersion":null,"value":42}),
    )
    .unwrap();
    let read = call(
        &mut db,
        "two",
        "reader",
        &reader,
        json!({"dataset":"data","action":"record.get","key":"item"}),
    )
    .unwrap();
    assert_eq!(read["value"], 42);
    assert!(call(
        &mut db,
        "two",
        "reader",
        &reader,
        json!({"dataset":"data","action":"record.put","key":"item","expectedVersion":1,"value":43}),
    )
    .unwrap_err()
    .contains("read-only"));

    assert!(
        call(
            &mut db,
            "three",
            "unbound",
            &Installation::default(),
            json!({"dataset":"data","action":"record.get","key":"item"}),
        )
        .unwrap_err()
        .contains("not granted")
    );
}

#[test]
fn record_compare_and_swap_preserves_tombstone_versions() {
    let mut db = database();
    let config = Installation::default();
    let created = put_record(&mut db, "one", &config, Value::Null, json!("first")).unwrap();
    assert_eq!(created["version"], 1);
    assert!(
        put_record(&mut db, "one", &config, json!(null), json!("stale"))
            .unwrap_err()
            .contains("conflict")
    );

    let deleted = call(
        &mut db,
        "one",
        "app",
        &config,
        json!({"dataset":"private","action":"record.delete","key":"item","expectedVersion":1}),
    )
    .unwrap();
    assert_eq!(deleted["version"], 2);
    assert_eq!(deleted["deleted"], true);
    assert!(
        put_record(&mut db, "one", &config, json!(1), json!("aba"))
            .unwrap_err()
            .contains("conflict")
    );
    let restored = put_record(&mut db, "one", &config, json!(2), json!("second")).unwrap();
    assert_eq!(restored["version"], 3);
    assert_eq!(restored["deleted"], false);
}

#[test]
fn publication_is_immutable_idempotent_and_checks_output_media_type() {
    let mut db = database();
    let config = shared("team", true);
    let declared = json!({"report":{"mediaType":"application/json"}});
    let same = draft(json!({"value":1}), "once");
    let first = super::execute(
        &mut db,
        "one",
        "app",
        &declared,
        &config,
        &json!({"dataset":"data","action":"artifact.publish","draft":same}),
    )
    .unwrap();
    let again = super::execute(&mut db, "one", "app", &declared, &config, &json!({"dataset":"data","action":"artifact.publish","draft":draft(json!({"value":1}),"once")})).unwrap();
    assert_eq!(artifact_id(&first), artifact_id(&again));
    assert!(
        publish(
            &mut db,
            "one",
            "app",
            &config,
            draft(json!({"value":2}), "once")
        )
        .unwrap_err()
        .contains("idempotency key")
    );
    let wrong_media = json!({"output":"report","mediaType":"text/plain","content":"x"});
    assert!(
        super::execute(
            &mut db,
            "one",
            "app",
            &declared,
            &config,
            &json!({"dataset":"data","action":"artifact.publish","draft":wrong_media})
        )
        .unwrap_err()
        .contains("declared output")
    );

    let other = publish(
        &mut db,
        "one",
        "app",
        &config,
        draft(json!({"value":3}), "other"),
    )
    .unwrap();
    assert_ne!(artifact_id(&first), artifact_id(&other));
    assert_eq!(first["content"]["value"], 1);
}

#[test]
fn listing_order_filters_and_tags_are_stable() {
    let mut db = database();
    let config = shared("team", true);
    let mut a = draft(json!({"n":1}), "a");
    a["project"] = json!("alpha");
    a["status"] = json!("ready");
    a["tags"] = json!(["red", "daily"]);
    let one = publish(&mut db, "i1", "app-a", &config, a).unwrap();
    let mut b = draft(json!({"n":2}), "b");
    b["project"] = json!("beta");
    b["status"] = json!("draft");
    b["tags"] = json!(["blue"]);
    let two = publish(&mut db, "i2", "app-b", &config, b).unwrap();

    let all = call(
        &mut db,
        "reader",
        "x",
        &config,
        json!({"dataset":"data","action":"artifact.list","query":{}}),
    )
    .unwrap();
    assert_eq!(
        all.as_array()
            .unwrap()
            .iter()
            .map(artifact_id)
            .collect::<Vec<_>>(),
        vec![artifact_id(&two), artifact_id(&one)]
    );
    let filtered = call(&mut db, "reader", "x", &config, json!({"dataset":"data","action":"artifact.list","query":{"project":"alpha","tag":"red","status":"ready","applicationId":"app-a","installationId":"i1","output":"report","limit":1}})).unwrap();
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(artifact_id(&filtered[0]), artifact_id(&one));
}

#[test]
fn dynamic_resolutions_are_snapshots_and_slots_reject_reference_reuse() {
    let mut db = database();
    let config = shared("team", true);
    let first = publish(&mut db, "one", "app", &config, draft(json!(1), "one")).unwrap();
    let reference = json!({"kind":"dynamic","query":{"output":"report"}});
    let request = |reference: Value| json!({"dataset":"data","action":"artifact.resolve","executionId":"run-1","slot":"input","reference":reference});
    let resolved = call(&mut db, "one", "app", &config, request(reference.clone())).unwrap();
    assert_eq!(resolved.as_array().unwrap().len(), 1);
    assert_eq!(artifact_id(&resolved[0]), artifact_id(&first));

    publish(&mut db, "one", "app", &config, draft(json!(2), "two")).unwrap();
    let repeated = call(&mut db, "one", "app", &config, request(reference.clone())).unwrap();
    assert_eq!(repeated, resolved);
    assert!(
        call(
            &mut db,
            "one",
            "app",
            &config,
            request(json!({"kind":"dynamic","query":{"tag":"later"}}))
        )
        .unwrap_err()
        .contains("different reference")
    );
}

#[test]
fn inputs_and_lineage_must_resolve_inside_the_authorized_dataset() {
    let mut db = database();
    let team = shared("team", true);
    let private = Installation::default();
    let source = publish(
        &mut db,
        "one",
        "source-app",
        &team,
        draft(json!("source"), "source"),
    )
    .unwrap();
    let dependent_draft = json!({
        "output":"report","mediaType":"application/json","content":"dependent",
        "idempotencyKey":"dependent","inputs":[artifact_id(&source)]
    });
    let dependent = publish(&mut db, "one", "consumer", &team, dependent_draft).unwrap();
    let lineage = call(
        &mut db,
        "two",
        "consumer",
        &shared("team", false),
        json!({"dataset":"data","action":"artifact.lineage","id":artifact_id(&dependent)}),
    )
    .unwrap();
    assert_eq!(lineage["inputs"].as_array().unwrap().len(), 1);
    assert_eq!(artifact_id(&lineage["inputs"][0]), artifact_id(&source));

    let unauthorized = json!({"output":"report","mediaType":"application/json","content":"bad","inputs":[artifact_id(&source)]});
    assert!(
        super::execute(
            &mut db,
            "one",
            "private-app",
            &json!({}),
            &private,
            &json!({"dataset":"private","action":"artifact.publish","draft":unauthorized})
        )
        .unwrap_err()
        .contains("not found or not authorized")
    );
    assert!(
        call(
            &mut db,
            "three",
            "consumer",
            &shared("another-team", false),
            json!({"dataset":"data","action":"artifact.get","id":artifact_id(&dependent)})
        )
        .unwrap_err()
        .contains("not found or not authorized")
    );
}

#[test]
fn records_and_artifacts_survive_reopening_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("platform.sqlite");
    let config = shared("team", true);
    let id;
    {
        let mut db = Connection::open(&path).unwrap();
        super::migrate(&mut db).unwrap();
        let record = call(&mut db, "one", "app", &config, json!({"dataset":"data","action":"record.put","key":"persist","expectedVersion":null,"value":"saved"})).unwrap();
        assert_eq!(record["version"], 1);
        id = artifact_id(
            &publish(
                &mut db,
                "one",
                "app",
                &config,
                draft(json!({"saved":true}), "persisted"),
            )
            .unwrap(),
        )
        .to_owned();
    }
    let mut db = Connection::open(&path).unwrap();
    super::migrate(&mut db).unwrap();
    let record = call(
        &mut db,
        "two",
        "app",
        &config,
        json!({"dataset":"data","action":"record.get","key":"persist"}),
    )
    .unwrap();
    assert_eq!(record["value"], "saved");
    let artifact = call(
        &mut db,
        "two",
        "app",
        &config,
        json!({"dataset":"data","action":"artifact.get","id":id}),
    )
    .unwrap();
    assert_eq!(artifact["content"]["saved"], true);
}

#[test]
fn remote_shared_requests_forward_bearer_and_request_while_private_stays_local() {
    let (url, received, worker) = remote_server(200, r#"{"value":{"remote":true}}"#);
    let config = remote_installation(url);
    let mut db = database();
    let request = json!({"dataset":"data","action":"record.get","key":"remote-key"});
    let result = call(&mut db, "install-1", "app-1", &config, request.clone()).unwrap();
    assert_eq!(result, json!({"remote":true}));
    let (line, authorization, body) = received.recv().unwrap();
    assert!(line.starts_with("POST /platform HTTP/1.1"));
    assert_eq!(authorization, "Bearer scoped-secret");
    let mut expected_request = request;
    expected_request["expectedScope"] = json!({"installationId":"install-1","applicationId":"app-1","datasetId":"shared:shared-dataset"});
    assert_eq!(body, expected_request.to_string());
    worker.join().unwrap();

    let private = call(
        &mut db,
        "install-1",
        "app-1",
        &config,
        json!({"dataset":"private","action":"record.put","key":"local-key","expectedVersion":null,"value":"local"}),
    )
    .unwrap();
    assert_eq!(private["version"], 1);
    assert_eq!(
        call(
            &mut db,
            "install-1",
            "app-1",
            &config,
            json!({"dataset":"private","action":"record.get","key":"local-key"})
        )
        .unwrap()["value"],
        "local"
    );
    let count: u32 = db
        .query_row(
            "SELECT count(*) FROM platform_records WHERE dataset='shared:shared-dataset'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn failed_remote_shared_request_never_falls_back_to_local_storage() {
    let (url, received, worker) =
        remote_server(503, r#"{"error":"remote temporarily unavailable"}"#);
    let config = remote_installation(url);
    let mut db = database();
    let error = call(
        &mut db,
        "install-1",
        "app-1",
        &config,
        json!({"dataset":"data","action":"record.put","key":"must-not-be-local","expectedVersion":null,"value":"value"}),
    ).unwrap_err();
    assert!(error.contains("remote temporarily unavailable"));
    let (line, authorization, body) = received.recv().unwrap();
    assert!(line.starts_with("POST /platform HTTP/1.1"));
    assert_eq!(authorization, "Bearer scoped-secret");
    assert!(body.contains("must-not-be-local"));
    let forwarded: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        forwarded["expectedScope"],
        json!({"installationId":"install-1","applicationId":"app-1","datasetId":"shared:shared-dataset"})
    );
    worker.join().unwrap();
    let count: u32 = db
        .query_row(
            "SELECT count(*) FROM platform_records WHERE dataset='shared:shared-dataset'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
