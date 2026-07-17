use super::*;

#[tokio::test]
async fn update_user_library_permissions_changes_grants() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    let updated = app
        .set_user_library_permissions(&user, &created.id, grants)
        .await
        .expect("update permissions");

    let authorization = app
        .load_user_authorization(&updated)
        .await
        .expect("load authorization");
    assert!(
        authorization.has_any_library_permission(scryer_domain::LibraryPermission::ManageTitles)
    );
}

#[tokio::test]
async fn update_user_password_is_hashed() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let updated = app
        .set_user_password(&user, &created.id, "after-pass".to_string())
        .await
        .expect("update password");

    assert!(updated.password_hash.is_some());
    assert_ne!(
        updated.password_hash, created.password_hash,
        "password hash should change when password is updated"
    );
    assert_ne!(updated.password_hash, Some("after-pass".to_string()));
}

#[tokio::test]
async fn create_user_rejects_password_shorter_than_minimum() {
    let (app, user) = bootstrap();

    let result = create_user_with_permissions(
        &app,
        &user,
        "short-password-user",
        "1234567",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn set_user_password_rejects_password_shorter_than_minimum() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "password-reset-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let result = app
        .set_user_password(&user, &created.id, "1234567".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn self_password_change_is_hashed() {
    let (app, admin) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &admin,
        "self-password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let updated = app
        .change_own_password(
            &created,
            "after-pass".to_string(),
            "before-pass".to_string(),
        )
        .await
        .expect("update own password");

    assert!(updated.password_hash.is_some());
    assert_ne!(
        updated.password_hash, created.password_hash,
        "password hash should change when password is updated"
    );
    assert_ne!(updated.password_hash, Some("after-pass".to_string()));
}

#[tokio::test]
async fn self_password_change_rejects_password_shorter_than_minimum() {
    let (app, admin) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &admin,
        "self-short-password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let result = app
        .change_own_password(&created, "1234567".to_string(), "before-pass".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn set_initial_own_password_rejects_password_shorter_than_minimum() {
    let (app, _) = bootstrap();
    let mut user =
        test_user_with_app_permissions("initial-short-password-user", AppPermissionMask::NONE);
    user.authorization.actor_capabilities = scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT;
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create passwordless user");

    let result = app
        .set_initial_own_password(&user, "1234567".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn set_initial_own_password_requires_own_account_capability() {
    let (app, _) = bootstrap();
    let user =
        test_user_with_app_permissions("initial-password-unauthorized", AppPermissionMask::NONE);
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create passwordless user");

    let result = app
        .set_initial_own_password(&user, "valid-password".to_string())
        .await;

    assert!(matches!(result, Err(AppError::Unauthorized(_))));
}

#[tokio::test]
async fn delete_other_user_removes_user() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "removable",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    app.delete_user(&user, &created.id)
        .await
        .expect("delete user");

    let users = app.list_users(&user).await.expect("list users");
    assert!(!users.iter().any(|entry| entry.id == created.id));
}

#[tokio::test]
async fn delete_user_rejects_removing_last_full_administrator() {
    let (app, actor) = bootstrap();
    let bootstrap_admin = app
        .find_or_create_default_user()
        .await
        .expect("create bootstrap admin");

    let result = app.delete_user(&actor, &bootstrap_admin.id).await;

    assert!(matches!(
        result,
        Err(AppError::Validation(message))
            if message == "cannot delete the last full administrator"
    ));
}

#[tokio::test]
async fn delete_user_allows_removing_bootstrap_admin_after_replacement_exists() {
    let (app, actor) = bootstrap();
    let bootstrap_admin = app
        .find_or_create_default_user()
        .await
        .expect("create bootstrap admin");
    app.create_user(
        &actor,
        "replacement-admin".to_string(),
        "password123".to_string(),
        scryer_domain::UserAuthorization::full_admin().app,
        vec![],
    )
    .await
    .expect("create replacement full admin");

    app.delete_user(&actor, &bootstrap_admin.id)
        .await
        .expect("delete bootstrap admin");

    assert!(app.find_default_user().await.unwrap().is_none());
}
