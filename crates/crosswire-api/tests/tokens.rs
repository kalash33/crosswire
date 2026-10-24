//! Token routes: create (secret shown once), list (metadata only), revoke.

mod common;

use common::TestApp;

#[tokio::test]
async fn create_list_revoke() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let admin = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
        ("Content-Type", "application/json"),
    ];
    let read = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
    ];

    // Create: the secret is shown exactly once.
    let created = app
        .send(
            "POST",
            "/api/v1/tokens",
            &admin,
            Some(b"{\"name\":\"ci\",\"scopes\":[\"read\",\"send\"]}"),
        )
        .await;
    assert_eq!(created.status, 200, "{}", created.body_string());
    let v = created.json();
    let secret = v["secret"].as_str().expect("a secret");
    assert!(secret.starts_with("cwt_"), "the secret has the cwt_ prefix");
    let id = v["token"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["token"]["scopes"][0], "read");

    // List: metadata only, never the secret or its hash.
    let list = app.get("/api/v1/tokens", &read).await;
    assert_eq!(list.status, 200);
    let rows = list.json();
    let row = &rows[0];
    assert_eq!(row["id"], id);
    assert!(row.get("secret").is_none(), "no secret in the listing");
    assert!(row.get("hash").is_none(), "no hash in the listing");
    assert!(row["revoked_at"].is_null(), "not yet revoked");

    // The created token authenticates.
    let auth = format!("Bearer {secret}");
    let used = app.get("/api/v1/counts", &[("Authorization", &auth)]).await;
    assert_eq!(used.status, 200, "the token works");

    // Revoke it.
    let revoke = app
        .send("DELETE", &format!("/api/v1/tokens/{id}"), &read, None)
        .await;
    assert_eq!(revoke.status, 200);

    // The listing now shows it revoked, and the token stops working.
    let list2 = app.get("/api/v1/tokens", &read).await;
    assert!(
        !list2.json()[0]["revoked_at"].is_null(),
        "revoked_at is set"
    );
    let after = app.get("/api/v1/counts", &[("Authorization", &auth)]).await;
    assert_eq!(after.status, 401, "a revoked token no longer works");

    // Revoking an unknown token is 404.
    let missing = app
        .send(
            "DELETE",
            "/api/v1/tokens/00000000000000000000000000",
            &read,
            None,
        )
        .await;
    assert_eq!(missing.status, 404);
}
