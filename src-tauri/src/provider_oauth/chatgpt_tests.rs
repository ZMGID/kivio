use super::*;

fn registration() -> Registration {
    Registration {
        client_id: "oaiapp_test".into(),
        subject: Some("test-subject".into()),
        revision: None,
        email: Some("person@example.test".into()),
        name: None,
        scopes: vec![DIRECT.into()],
        id_token: Some("synthetic-id-token".into()),
        earliest_refresh_at: None,
        pending_refresh: None,
    }
}

fn attempt(client_id: &str) -> Attempt {
    Attempt {
        state: "expected".into(),
        nonce: "test-nonce".into(),
        verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
        redirect_uri: "http://127.0.0.1:18765/auth/callback".parse().unwrap(),
        client_id: client_id.into(),
    }
}

#[test]
fn dynamic_registration_and_returning_sign_in_bind_the_issued_client() {
    let dir = tempfile::tempdir().unwrap();
    let host = host_id(dir.path()).unwrap();
    let new = attempt(DYNAMIC);
    let url = new.authorization_url(&host, None);
    let params = url.query_pairs().collect::<HashMap<_, _>>();
    assert_eq!(params["ext_agent_host_id"], host_id(dir.path()).unwrap());
    assert_eq!(params["client_id"], DYNAMIC);
    assert_eq!(params["agent_name_hint"], "Kivio");
    assert_eq!(params["resource"], CHATGPT_BASE);
    assert_eq!(
        params["code_challenge"],
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let callback =
        |query| format!("GET /auth/callback?{query} HTTP/1.1\r\nHost: 127.0.0.1:18765\r\n\r\n");
    let valid_request = callback("code=c&state=expected&client_id=oaiapp_test");
    for host in [
        "localhost:18765",
        "127.0.0.1:1",
        "127.0.0.1:18765\r\nHost: other",
    ] {
        assert!(new
            .callback(&valid_request.replace("127.0.0.1:18765", host))
            .is_err());
    }
    for query in ["code=c&state=wrong", "code=c&state=expected&state=expected"] {
        assert!(new.callback(&callback(query)).is_err());
    }
    for query in [
        "code=c&state=expected",
        "code=c&state=expected&client_id=dynamic_agent_client",
        "code=c&state=expected&error=access_denied",
    ] {
        assert!(new.callback(&callback(query)).unwrap().is_err());
    }
    assert_eq!(
        new.callback(&callback("code=c&state=expected&client_id=oaiapp_test"))
            .unwrap()
            .unwrap(),
        ("c".into(), "oaiapp_test".into())
    );

    let returning = attempt("oaiapp_test");
    let url = returning.authorization_url(&host, Some(&registration()));
    let params = url.query_pairs().collect::<HashMap<_, _>>();
    assert!(!url.as_str().contains("synthetic-id-token"));
    assert!(!params.contains_key("id_token_hint"));
    assert_eq!(params["login_hint"], "person@example.test");
    assert_eq!(params["client_id"], "oaiapp_test");
    assert!(!params.contains_key("agent_name_hint") && !params.contains_key("prompt"));
    assert_eq!(
        returning
            .callback(&callback("state=expected&code=c"))
            .unwrap()
            .unwrap()
            .1,
        "oaiapp_test"
    );
    assert!(returning
        .callback(&callback("state=expected&code=c&client_id=other"))
        .unwrap()
        .is_err());
}

#[test]
fn interrupted_registration_resumes_after_restart_and_a_late_login_cannot_undo_disconnect() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut first = LoginProfile::load(root, None).unwrap();
    first.record_client(root, "oaiapp_test").unwrap();
    drop(first); // No code exchange or login poll completed.
    let pending = saved_accounts(root).unwrap().pop().unwrap();
    assert!(!pending.connected && pending.subject.is_none());
    let mut resumed = LoginProfile::load(root, Some(pending.credential_id.clone())).unwrap();
    let client = &resumed.registration.as_ref().unwrap().client_id;
    assert_eq!(client, "oaiapp_test");
    resumed.record_client(root, "oaiapp_test").unwrap();
    let mut registration = registration();
    registration.revision = Some(uuid::Uuid::new_v4());
    let reply = json!({"scope": DIRECT, "token_type": "Bearer", "access_token": "synthetic", "expires_in": 3600});
    let mut signed_in = session_tokens(&reply, registration, true).unwrap();
    let result = resumed.complete(root, &signed_in).unwrap();
    assert_eq!(result.credential_id.as_ref(), Some(&pending.credential_id));
    let late_login = LoginProfile::load(root, result.credential_id).unwrap();
    let late_reply = saved_session(root, &pending.credential_id).unwrap();
    clear_session(&mut signed_in);
    save_tokens_in(root, &pending.credential_id, &signed_in).unwrap();
    assert!(late_login.complete(root, &late_reply).is_err());
    let accounts = saved_accounts(root).unwrap();
    assert_eq!(accounts.len(), 1);
    assert!(!accounts[0].connected);
    assert_eq!(accounts[0].subject.as_deref(), Some("test-subject"));
}

#[test]
fn identity_requires_a_valid_signature_issuer_audience_expiry_and_nonce() {
    // Generated with an ephemeral RSA key. Only the public key and synthetic signatures are retained.
    let fixture: Value = serde_json::from_str(include_str!("chatgpt_id_token.test.json")).unwrap();
    let keys: JwkSet = serde_json::from_value(fixture.clone()).unwrap();
    let valid = fixture["valid"].as_str().unwrap();
    let current = now();
    let identity =
        verified_identity(valid, "oaiapp_test", Some("test-nonce"), current, &keys).unwrap();
    assert_eq!(identity.sub, "test-subject");
    for field in ["expired", "wrongIssuer", "missingIat"] {
        assert!(verified_identity(
            fixture[field].as_str().unwrap(),
            "oaiapp_test",
            Some("test-nonce"),
            current,
            &keys
        )
        .is_err());
    }
    assert!(verified_identity(valid, "other", Some("test-nonce"), current, &keys).is_err());
    assert!(verified_identity(valid, "oaiapp_test", Some("other"), current, &keys).is_err());
    let (claims, sig) = valid.rsplit_once('.').unwrap();
    let forged = format!(
        "{claims}.{}{rest}",
        if sig.starts_with('A') { "B" } else { "A" },
        rest = &sig[1..]
    );
    assert!(verified_identity(&forged, "oaiapp_test", Some("test-nonce"), current, &keys).is_err());
}

#[tokio::test]
async fn refresh_rotation_and_signout_preserve_only_the_saved_registration() {
    let root = tempfile::tempdir().unwrap();
    let id = new_credential_id();
    assert!(session_tokens(
        &json!({"id_token":"synthetic-id-token"}),
        registration(),
        true
    )
    .is_err());
    let reply = json!({"scope":"openid profile email", "id_token":"synthetic-id-token"});
    let mut tokens = session_tokens(&reply, registration(), true).unwrap();
    assert!(prepare_session(&id, &mut tokens, false)
        .await
        .unwrap_err()
        .contains("disabled"));
    save_tokens_in(root.path(), &id, &tokens).unwrap();
    let signed_in = saved_accounts(root.path()).unwrap();
    assert!(signed_in[0].connected && !signed_in[0].plan_enabled);
    let disabled = attempt("oaiapp_test");
    assert!(disabled
        .authorization_url("host", tokens.chatgpt.as_ref())
        .query_pairs()
        .any(|(k, v)| k == "prompt" && v == "consent"));

    let replacement = json!({"access_token":"new-access","refresh_token":"new-refresh",
        "token_type":"Bearer","expires_in":3600,"scope":DIRECT,"earliest_refresh_at":"2030-01-01T00:00:00Z"});
    record_refresh(root.path(), &id, &mut tokens, &replacement, now()).unwrap();
    let restored = read_tokens_in(root.path(), &id).unwrap();
    assert_eq!(restored.refresh_token, "new-refresh");
    assert_eq!(
        restored.chatgpt.unwrap().earliest_refresh_at,
        Some(1893456000)
    );
    assert!(saved_accounts(root.path()).unwrap()[0].plan_enabled);
    tokens.chatgpt.as_mut().unwrap().earliest_refresh_at = Some(now() + 3600);
    tokens.expires_at = now() + 30;
    prepare_session(&id, &mut tokens, false).await.unwrap();
    tokens.expires_at = now() - 1;
    assert!(prepare_session(&id, &mut tokens, false)
        .await
        .unwrap_err()
        .contains("cannot refresh"));
    clear_session(&mut tokens);
    save_tokens_in(root.path(), &id, &tokens).unwrap();
    let accounts = saved_accounts(root.path()).unwrap();
    assert_eq!(accounts[0].client_id, "oaiapp_test");
    assert!(!accounts[0].connected && !accounts[0].plan_enabled);
    let persisted = fs::read_to_string(credential_path(root.path(), &id).unwrap()).unwrap();
    for secret in ["new-access", "new-refresh", "synthetic-id-token"] {
        assert!(!persisted.contains(secret));
    }
}

#[tokio::test]
async fn signing_key_outage_reopens_the_saved_rotation_without_a_second_grant() {
    use axum::{
        routing::{get, post},
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    let root = tempfile::tempdir().unwrap();
    let id = new_credential_id();
    let fixture: Value = serde_json::from_str(include_str!("chatgpt_id_token.test.json")).unwrap();
    let previous = json!({"access_token":"old-access","refresh_token":"old-refresh","scope":DIRECT,
        "token_type":"Bearer","expires_in":30});
    let mut tokens = session_tokens(&previous, registration(), true).unwrap();
    save_tokens_in(root.path(), &id, &tokens).unwrap();
    let grants = Arc::new(AtomicUsize::new(0));
    let grant_count = grants.clone();
    let successor_id = fixture["refresh"].clone();
    let grant = post(move || {
        grant_count.fetch_add(1, Ordering::Relaxed);
        let id_token = successor_id.clone();
        async move {
            Json(
                json!({"access_token":"successor-access","refresh_token":"successor-refresh",
            "token_type":"Bearer","expires_in":3600,"id_token":id_token}),
            )
        }
    });
    let key_calls = Arc::new(AtomicUsize::new(0));
    let checkpoint_root = root.path().to_owned();
    let checkpoint_id = id.clone();
    let keys = get(move || {
        let call = key_calls.fetch_add(1, Ordering::Relaxed);
        let saved = saved_session(&checkpoint_root, &checkpoint_id).unwrap();
        assert_eq!(saved.refresh_token, "old-refresh");
        assert_eq!(
            saved
                .chatgpt
                .unwrap()
                .pending_refresh
                .unwrap()
                .grant
                .refresh_token,
            "successor-refresh"
        );
        let response = match call {
            0 => json!({"keys":[]}),
            1 => json!({"keys":[{"kty":"RSA","kid":"synthetic","n":"","e":"AQAB"}]}),
            _ => fixture.clone(),
        };
        async move { Json(response) }
    });
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(|| async { Json(json!({"issuer":ISSUER, "jwks_uri":format!("{ISSUER}/keys")})) }),
        )
        .route("/api/accounts/oauth/token", grant)
        .route("/keys", keys);
    let host = "auth.openai.com";
    let tls = rcgen::generate_simple_self_signed(vec![host.into()]).unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .resolve(host, listener.local_addr().unwrap())
        .tls_certs_only([reqwest::Certificate::from_der(tls.cert.der()).unwrap()])
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap();
    let config = axum_server::tls_rustls::RustlsConfig::from_der(
        vec![tls.cert.der().to_vec()],
        tls.key_pair.serialize_der(),
    )
    .await
    .unwrap();
    let server =
        tokio::spawn(axum_server::from_tcp_rustls(listener, config).serve(app.into_make_service()));

    for _ in 0..2 {
        let unavailable = refresh_session(root.path(), &id, &mut tokens, &client)
            .await
            .unwrap_err();
        assert!(
            unavailable.contains("temporarily unavailable"),
            "{unavailable}"
        );
        tokens = saved_session(root.path(), &id).unwrap();
    }
    refresh_session(root.path(), &id, &mut tokens, &client)
        .await
        .unwrap();
    assert_eq!(grants.load(Ordering::Relaxed), 1);
    let verified = saved_session(root.path(), &id).unwrap();
    assert_eq!(verified.access_token, "successor-access");
    assert_eq!(verified.refresh_token, "successor-refresh");
    let account = verified.chatgpt.unwrap();
    assert!(account.pending_refresh.is_none());
    assert_eq!(account.email.as_deref(), Some("refreshed@example.test"));
    server.abort();
}

#[test]
fn persisted_rotations_verify_at_receipt_and_clear_both_generations_on_invalid_identity() {
    let root = tempfile::tempdir().unwrap();
    let id = new_credential_id();
    let fixture: Value = serde_json::from_str(include_str!("chatgpt_id_token.test.json")).unwrap();
    let keys: JwkSet = serde_json::from_value(fixture.clone()).unwrap();
    for (subject, token, received_at, accepted) in [
        ("test-subject", "expired", 1_600_000_030, true),
        ("test-subject", "expired", 1_600_000_090, false),
        ("other-subject", "refresh", now(), false),
        ("test-subject", "wrongIssuer", now(), false),
    ] {
        let mut reg = registration();
        reg.subject = Some(subject.into());
        let previous = json!({"access_token":"old-access","refresh_token":"old-refresh","scope":DIRECT,
            "token_type":"Bearer","expires_in":3600});
        let mut tokens = session_tokens(&previous, reg, true).unwrap();
        let next = json!({"access_token":"next-access","refresh_token":"next-refresh","id_token":fixture[token],
            "token_type":"Bearer","expires_in":3600});
        record_refresh(root.path(), &id, &mut tokens, &next, received_at).unwrap();
        let mut reopened = saved_session(root.path(), &id).unwrap();
        assert_eq!(
            finish_refresh(root.path(), &id, &mut reopened, &keys).is_ok(),
            accepted
        );
        let saved = saved_session(root.path(), &id).unwrap();
        let account = saved.chatgpt.unwrap();
        assert!(account.pending_refresh.is_none());
        assert_eq!(account.subject.as_deref(), Some(subject));
        assert_eq!(account.id_token.is_some(), accepted);
        assert_eq!(
            saved.refresh_token,
            if accepted { "next-refresh" } else { "" }
        );
        assert_eq!(
            saved.access_token,
            if accepted { "next-access" } else { "" }
        );
    }
}
