//! Exercises configured storage policy and its emitted metadata.

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn auto_auth_storage_load_prefers_keyring_value() -> anyhow::Result<()> {
    let codex_home = tempdir()?;
    let mock_keyring = MockKeyringStore::default();
    let storage = AutoAuthStorage::new(
        codex_home.path().to_path_buf(),
        Arc::new(mock_keyring.clone()),
        AuthKeyringBackendKind::Secrets,
    );
    let keyring_auth = auth_with_prefix("keyring");
    seed_secrets_backend_with_auth(&mock_keyring, codex_home.path(), &keyring_auth)?;

    let file_auth = auth_with_prefix("file");
    storage.file_storage.save(&file_auth)?;

    let loaded = storage.load()?;
    assert_eq!(loaded, Some(keyring_auth));
    Ok(())
}

#[test]
fn auto_auth_storage_load_uses_file_when_keyring_empty() -> anyhow::Result<()> {
    let codex_home = tempdir()?;
    let mock_keyring = MockKeyringStore::default();
    let storage = AutoAuthStorage::new(
        codex_home.path().to_path_buf(),
        Arc::new(mock_keyring),
        AuthKeyringBackendKind::Secrets,
    );

    let expected = auth_with_prefix("file-only");
    storage.file_storage.save(&expected)?;

    let loaded = storage.load()?;
    assert_eq!(loaded, Some(expected));
    Ok(())
}

#[test]
fn auto_auth_storage_load_falls_back_when_keyring_errors() -> anyhow::Result<()> {
    let codex_home = tempdir()?;
    let mock_keyring = MockKeyringStore::default();
    let storage = AutoAuthStorage::new(
        codex_home.path().to_path_buf(),
        Arc::new(mock_keyring.clone()),
        AuthKeyringBackendKind::Secrets,
    );
    let key = compute_keyring_account(codex_home.path(), LocalSecretsNamespace::CodexAuth);

    let encrypted = auth_with_prefix("encrypted");
    seed_secrets_backend_with_auth(&mock_keyring, codex_home.path(), &encrypted)?;
    assert_eq!(storage.load()?, Some(encrypted.clone()));
    mock_keyring.set_error(&key, KeyringError::Invalid("error".into(), "load".into()));

    let expected = auth_with_prefix("fallback");
    storage.file_storage.save(&expected)?;

    let loaded = storage.load()?;
    assert_eq!(loaded, Some(expected.clone()));
    mock_keyring.set_error(&key, KeyringError::Invalid("error".into(), "load".into()));
    assert!(storage.load_account_grant().is_err());
    assert_eq!(storage.load()?, Some(encrypted.clone()));
    assert_eq!(storage.file_storage.load()?, Some(expected.clone()));
    mock_keyring.set_error(&key, KeyringError::Invalid("error".into(), "load".into()));
    let refreshed_fallback = auth_with_prefix("refreshed-fallback");
    storage.compare_and_save(&expected, &refreshed_fallback)?;
    // Recovery must not promote the fallback over an unread keyring generation.
    assert_eq!(storage.load()?, Some(encrypted));
    assert_eq!(storage.file_storage.load()?, Some(refreshed_fallback));

    // A recovered keyring may contain a newer generation than the fallback.
    // Never overwrite it with a response obtained using the fallback token.
    let mut previous = auth_with_prefix("same-login");
    previous.auth_mode = Some(AuthMode::Chatgpt);
    previous.openai_api_key = None;
    let backend: Arc<dyn AuthStorageBackend> = Arc::new(storage.clone());
    for recover_before_persist in [false, true] {
        seed_secrets_backend_with_auth(&mock_keyring, codex_home.path(), &previous)?;
        let mut cached = previous.clone();
        if recover_before_persist {
            let tokens = cached.tokens.as_mut().context("fallback account tokens")?;
            tokens.access_token = "fallback-access".into();
            tokens.refresh_token = "fallback-refresh".into();
        }
        storage.file_storage.save(&cached)?;
        mock_keyring.set_error(&key, KeyringError::Invalid("error".into(), "load".into()));
        if recover_before_persist {
            assert_eq!(storage.load()?, Some(cached.clone()));
        }
        let refreshed = crate::auth::manager::persist_tokens(
            &backend,
            cached.tokens.as_ref().context("stored account tokens")?,
            /*provenance*/ None,
            /*id_token*/ None,
            Some("rotated-access".into()),
            Some("rotated-refresh".into()),
        );
        if recover_before_persist {
            assert!(refreshed.is_err());
            assert_eq!(storage.load()?, Some(previous.clone()));
            assert_eq!(storage.file_storage.load()?, Some(cached));
        } else {
            // Matching keyring and fallback generations can safely commit.
            let refreshed = refreshed?;
            assert_eq!(storage.load()?, Some(refreshed.clone()));
            assert_keyring_saved_auth_and_removed_fallback(
                &mock_keyring,
                codex_home.path(),
                &refreshed,
            )?;
        }
    }

    // Once the selected source is stable, a fresh exchange can commit normally.
    let refreshed = crate::auth::manager::persist_tokens(
        &backend,
        previous.tokens.as_ref().context("stored account tokens")?,
        /*provenance*/ None,
        /*id_token*/ None,
        Some("rotated-access".into()),
        Some("rotated-refresh".into()),
    )?;
    let tokens = refreshed
        .tokens
        .as_ref()
        .context("refreshed account tokens")?;
    assert_eq!(tokens.access_token, "rotated-access");
    assert_eq!(tokens.refresh_token, "rotated-refresh");
    assert_eq!(storage.load()?, Some(refreshed.clone()));
    assert_keyring_saved_auth_and_removed_fallback(&mock_keyring, codex_home.path(), &refreshed)?;
    // An absent fallback must not select a plaintext destination for cached credentials.
    storage.file_storage.delete()?;
    mock_keyring.set_error(&key, KeyringError::Invalid("error".into(), "load".into()));
    assert_eq!(storage.load()?, None);
    storage.save(&auth_with_prefix("cached"))?;
    assert_keyring_saved_auth_and_removed_fallback(
        &mock_keyring,
        codex_home.path(),
        &auth_with_prefix("cached"),
    )?;
    Ok(())
}

#[test]
fn auto_auth_storage_refresh_preserves_fallback_when_keyring_stays_unreadable() -> anyhow::Result<()>
{
    let home = tempdir()?;
    let keyring = MockKeyringStore::default();
    let key = compute_store_key(home.path())?;
    // A malformed entry keeps every keyring read failing, while writes would
    // succeed. Reuse the keyring mock without a one-shot failure that recovers.
    keyring.save(KEYRING_SERVICE, &key, "unreadable credential document")?;
    let storage = create_auth_storage_with_store(
        home.path().to_path_buf(),
        AuthCredentialsStoreMode::Auto,
        Arc::new(keyring.clone()),
        AuthKeyringBackendKind::Direct,
    );
    let file = FileAuthStorage::new(home.path().to_path_buf());
    let mut previous = auth_with_prefix("fallback");
    previous.auth_mode = Some(AuthMode::Chatgpt);
    previous.openai_api_key = None;
    file.save(&previous)?;
    let previous_tokens = previous.tokens.as_ref().context("fallback tokens")?;
    let refreshed = crate::auth::manager::persist_tokens(
        &storage,
        previous_tokens,
        /*provenance*/ None,
        /*id_token*/ None,
        Some("rotated-access".into()),
        Some("rotated-refresh".into()),
    )?;
    assert_eq!(
        refreshed.tokens,
        Some(TokenData {
            access_token: "rotated-access".into(),
            refresh_token: "rotated-refresh".into(),
            ..previous_tokens.clone()
        })
    );
    assert_eq!(storage.load()?, Some(refreshed.clone()));
    assert_eq!(file.load()?, Some(refreshed.clone()));

    // Fallback commits must still reject stale generations and logout.
    assert!(storage.compare_and_save(&previous, &refreshed).is_err());
    assert_eq!(file.load()?, Some(refreshed.clone()));
    file.delete()?;
    assert!(storage.compare_and_save(&refreshed, &previous).is_err());
    assert_eq!(file.load()?, None);
    assert_eq!(
        keyring.saved_value(&key),
        Some("unreadable credential document".into())
    );
    Ok(())
}

#[test]
fn auto_auth_storage_save_prefers_keyring() -> anyhow::Result<()> {
    let codex_home = tempdir()?;
    let mock_keyring = MockKeyringStore::default();
    let storage = AutoAuthStorage::new(
        codex_home.path().to_path_buf(),
        Arc::new(mock_keyring.clone()),
        AuthKeyringBackendKind::Secrets,
    );
    let stale = auth_with_prefix("stale");
    storage.file_storage.save(&stale)?;

    let expected = auth_with_prefix("to-save");
    storage.save(&expected)?;

    assert_keyring_saved_auth_and_removed_fallback(&mock_keyring, codex_home.path(), &expected)?;
    Ok(())
}

#[test]
fn auto_auth_storage_save_falls_back_when_keyring_errors() -> anyhow::Result<()> {
    let codex_home = tempdir()?;
    let mock_keyring = MockKeyringStore::default();
    let storage = AutoAuthStorage::new(
        codex_home.path().to_path_buf(),
        Arc::new(mock_keyring.clone()),
        AuthKeyringBackendKind::Secrets,
    );
    let key = compute_keyring_account(codex_home.path(), LocalSecretsNamespace::CodexAuth);
    mock_keyring.set_error(&key, KeyringError::Invalid("error".into(), "save".into()));

    let auth = auth_with_prefix("fallback");
    storage.save(&auth)?;

    let auth_file = get_auth_file(codex_home.path());
    assert!(
        auth_file.exists(),
        "fallback auth.json should be created when keyring save fails"
    );
    let saved = storage
        .file_storage
        .load()?
        .context("fallback auth should exist")?;
    assert_eq!(saved, auth);
    assert!(
        mock_keyring.saved_value(&key).is_none(),
        "keyring should not contain value when save fails"
    );
    Ok(())
}

#[test]
fn auto_auth_storage_delete_removes_keyring_and_file() -> anyhow::Result<()> {
    let codex_home = tempdir()?;
    let mock_keyring = MockKeyringStore::default();
    let storage = AutoAuthStorage::new(
        codex_home.path().to_path_buf(),
        Arc::new(mock_keyring.clone()),
        AuthKeyringBackendKind::Secrets,
    );
    let auth = auth_with_prefix("to-delete");
    let auth_file = seed_secrets_backend_and_fallback_auth_file_for_delete(
        &mock_keyring,
        codex_home.path(),
        &auth,
    )?;

    let removed = storage.delete()?;

    assert!(removed, "delete should report removal");
    assert_eq!(storage.load()?, None, "encrypted auth should be removed");
    assert!(
        !auth_file.exists(),
        "fallback auth.json should be removed after delete"
    );
    Ok(())
}

#[test]
fn direct_keyring_auth_storage_saves_legacy_keyring_entry() -> anyhow::Result<()> {
    let codex_home = tempdir()?;
    let mock_keyring = MockKeyringStore::default();
    let storage = DirectKeyringAuthStorage::new(
        codex_home.path().to_path_buf(),
        Arc::new(mock_keyring.clone()),
        AuthCredentialsStoreMode::Keyring,
    );
    let auth_file = get_auth_file(codex_home.path());
    std::fs::write(&auth_file, "stale")?;
    let auth = auth_with_prefix("direct");

    storage.save(&auth)?;

    let legacy_key = compute_store_key(codex_home.path())?;
    let saved_value = mock_keyring
        .saved_value(&legacy_key)
        .context("direct keyring auth entry should exist")?;
    assert_eq!(saved_value, serde_json::to_string(&auth)?);
    assert!(!encrypted_auth_file(codex_home.path()).exists());
    assert!(
        !auth_file.exists(),
        "fallback auth.json should be removed after keyring save"
    );
    assert_eq!(storage.load()?, Some(auth));
    Ok(())
}

#[test]
fn secrets_keyring_auth_storage_save_persists_and_removes_fallback_file() -> anyhow::Result<()> {
    let codex_home = tempdir()?;
    let mock_keyring = MockKeyringStore::default();
    let storage = SecretsKeyringAuthStorage::new(
        codex_home.path().to_path_buf(),
        Arc::new(mock_keyring.clone()),
        AuthCredentialsStoreMode::Keyring,
    );
    let auth_file = get_auth_file(codex_home.path());
    std::fs::write(&auth_file, "stale")?;
    let auth = AuthDotJson {
        auth_mode: Some(AuthMode::Chatgpt),
        openai_api_key: None,
        tokens: Some(TokenData {
            id_token: Default::default(),
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            oauth_provenance: None,
            login_owner_id: None,
            account_id: Some("account".to_string()),
        }),
        last_refresh: Some(Utc::now()),
        agent_identity: None,
        personal_access_token: None,
        bedrock_api_key: None,
        bedrock_access_keys: None,
    };

    storage.save(&auth)?;

    assert_keyring_saved_auth_and_removed_fallback(&mock_keyring, codex_home.path(), &auth)?;
    Ok(())
}
