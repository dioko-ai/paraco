//! Host-controlled evidence for the bundled diagnostic plugin. Never loaded
//! from an application manifest, query parameter, or persisted guest result.
use serde_json::Value;
use std::sync::OnceLock;

static EVIDENCE: OnceLock<Value> = OnceLock::new();

pub fn evidence() -> Option<&'static Value> {
    EVIDENCE.get()
}

pub fn run(port: u16) -> Result<(), String> {
    let report = crate::containment::diagnostics::verify_with_canary();
    eprintln!("Native containment checks: {}", report["status"]);
    EVIDENCE
        .set(report)
        .map_err(|_| "Containment probe already initialized")?;
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let app = temporary.path().join("app");
    std::fs::create_dir(&app).map_err(|e| e.to_string())?;
    std::fs::write(
        app.join("paraco.json"),
        include_str!("../examples/sandbox-probe/paraco.json"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        app.join("main.ts"),
        include_str!("../examples/sandbox-probe/main.ts"),
    )
    .map_err(|e| e.to_string())?;
    let data = crate::data::Directory::resolve(Some(&temporary.path().join("data")))?;
    crate::runner::run(&app, port, None, &temporary.path().join("logs"), &data)
}
