//! Scope routes: replace, read back, import a pasted list, and the compile
//! error path.

mod common;

use common::TestApp;

fn admin(cookie: &str) -> [(&str, &str); 3] {
    [
        ("Cookie", cookie),
        ("Sec-Fetch-Site", "same-origin"),
        ("Content-Type", "application/json"),
    ]
}

#[tokio::test]
async fn put_then_get_round_trips() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let h = admin(&cookie);

    let scope = r#"{"include":[{"host":{"type":"exact","value":"a.test"}}],"exclude":[]}"#;
    let put = app
        .send("PUT", "/api/v1/scope", &h, Some(scope.as_bytes()))
        .await;
    assert_eq!(put.status, 200, "{}", put.body_string());

    let get = app
        .get(
            "/api/v1/scope",
            &[("Cookie", &cookie), ("Sec-Fetch-Site", "same-origin")],
        )
        .await;
    assert_eq!(get.status, 200);
    let v = get.json();
    assert_eq!(v["include"][0]["host"]["type"], "exact");
    assert_eq!(v["include"][0]["host"]["value"], "a.test");

    // The swap reaches the live compiled scope: an audit row records it.
    let log = std::fs::read_to_string(app.store.dir().join("audit.jsonl")).unwrap();
    assert!(log.contains("\"scope.set\""), "scope.set is audited");
}

#[tokio::test]
async fn put_scope_persists_the_setting() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let h = admin(&cookie);

    let scope = r#"{"include":[{"host":{"type":"exact","value":"persist.test"}}],"exclude":[]}"#;
    let put = app
        .send("PUT", "/api/v1/scope", &h, Some(scope.as_bytes()))
        .await;
    assert_eq!(put.status, 200, "{}", put.body_string());

    // The source document is written to the "scope" setting, which the server's
    // `load_scope` reads at open, so a restart keeps the scope.
    let stored = app
        .store
        .control()
        .unwrap()
        .setting_get("scope")
        .unwrap()
        .expect("the scope setting was written");
    let doc: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(doc["include"][0]["host"]["value"], "persist.test");
}

#[tokio::test]
async fn import_parses_a_pasted_list() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let h = admin(&cookie);

    let body = r#"{"text":"example.com\n*.foo.test"}"#;
    let resp = app
        .send("POST", "/api/v1/scope/import", &h, Some(body.as_bytes()))
        .await;
    assert_eq!(resp.status, 200, "{}", resp.body_string());
    let v = resp.json();
    assert!(
        v["include"].as_array().unwrap().len() >= 2,
        "two rules parsed"
    );
}

#[tokio::test]
async fn compile_error_is_422_scope_invalid() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let h = admin(&cookie);

    // An invalid host regex does not compile.
    let bad = r#"{"include":[{"host":{"type":"regex","value":"["}}],"exclude":[]}"#;
    let resp = app
        .send("PUT", "/api/v1/scope", &h, Some(bad.as_bytes()))
        .await;
    assert_eq!(resp.status, 422);
    assert_eq!(resp.json()["error"]["code"], "scope_invalid");
}
