//! Requires `docker compose up -d localstack` (or the full stack) running —
//! this test round-trips a real upload through LocalStack's S3 API.

mod common;

#[tokio::test]
async fn uploading_a_picture_streams_it_to_blob_storage_and_saves_the_key() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "avatar.user@example.com", "avataruserpassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let image_bytes = b"not a real png, just test bytes";
    let response = common::post_multipart_with_cookie(
        &app,
        "/profile/picture",
        &cookie,
        "picture",
        "avatar.png",
        "image/png",
        image_bytes,
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        common::location(&response),
        Some("/profile?saved=true".to_string())
    );

    let row = sqlx::query!(
        "select profile_picture_key from users where email = $1",
        "avatar.user@example.com",
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    let key = row
        .profile_picture_key
        .expect("profile_picture_key should be set after a successful upload");
    assert!(key.starts_with("profile-pictures/"));

    // Confirm the object actually landed in blob storage (not just that our
    // code believes it did) by fetching it straight from LocalStack's S3 API.
    let (s3_client, _) = docuflow::blob::clients_from_env().await;
    let object = s3_client
        .get_object()
        .bucket("docuflow-uploads")
        .key(&key)
        .send()
        .await
        .expect("uploaded object should be readable back from S3");
    let stored_bytes = object.body.collect().await.unwrap().into_bytes();
    assert_eq!(&stored_bytes[..], image_bytes);
}

#[tokio::test]
async fn uploading_a_non_image_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "notimage.user@example.com", "notimageuserpw").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::post_multipart_with_cookie(
        &app,
        "/profile/picture",
        &cookie,
        "picture",
        "notes.txt",
        "text/plain",
        b"just some text",
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

    let row = sqlx::query!(
        "select profile_picture_key from users where email = $1",
        "notimage.user@example.com",
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert!(row.profile_picture_key.is_none());
}

#[tokio::test]
async fn uploading_a_picture_without_a_session_is_rejected() {
    let app = common::test_state().await;
    let response = common::post_multipart_with_cookie(
        &app,
        "/profile/picture",
        "id=not-a-real-session",
        "picture",
        "avatar.png",
        "image/png",
        b"irrelevant",
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response), Some("/login".to_string()));
}
