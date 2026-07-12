mod common;

/// Every test below needs real OCR to produce `ocr_text` in the first
/// place — soft-skip the whole file's tests on a box without `tesseract`,
/// same convention as `documents_upload.rs`'s real-OCR tests.
fn tesseract_available() -> bool {
    common::command_on_path("tesseract")
}

#[tokio::test]
async fn ocr_with_a_recognizable_date_suggests_it_but_does_not_set_date_issued() {
    if !tesseract_available() {
        eprintln!("skipping ocr_with_a_recognizable_date_suggests_it_but_does_not_set_date_issued: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "datedsuggest.docs@example.com",
        "tests/fixtures/dated_sample.png",
        "dated_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done", "ocr should complete within the timeout");
    assert_eq!(
        uploaded.outcome.suggested_date_issued,
        Some(time::Date::from_calendar_date(2024, time::Month::March, 15).unwrap()),
        "expected the fixture's printed date to be recognized, got: {:?}",
        uploaded.outcome.suggested_date_issued
    );

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("Use this date"), "a fresh suggestion with no date_issued set should show the accept action");
    assert!(body.contains("2024-03-15"), "the suggested date should be shown to the user");
}

#[tokio::test]
async fn no_suggestion_when_ocr_text_has_no_recognizable_date() {
    if !tesseract_available() {
        eprintln!("skipping no_suggestion_when_ocr_text_has_no_recognizable_date: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded =
        common::upload_and_wait_for_ocr(&app, "nodatesuggest.docs@example.com", "tests/fixtures/ocr_sample.png", "ocr_sample.png", "image/png")
            .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert_eq!(uploaded.outcome.suggested_date_issued, None);

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("Use this date"), "no suggestion should be shown when OCR found no recognizable date");
}

#[tokio::test]
async fn suggestion_is_hidden_once_date_issued_is_set_manually() {
    if !tesseract_available() {
        eprintln!("skipping suggestion_is_hidden_once_date_issued_is_set_manually: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "manualdate.docs@example.com",
        "tests/fixtures/dated_sample.png",
        "dated_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert!(uploaded.outcome.suggested_date_issued.is_some(), "fixture should have produced a suggestion to hide");

    let form_body = "title=Manually+dated&tags=&date_issued=2020-01-01";
    let response =
        common::post_form_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie, form_body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(
        !body.contains("Use this date"),
        "the suggestion should stop showing once date_issued has a value, even though ocr_suggested_date_issued is still set in the DB"
    );
}

#[tokio::test]
async fn accept_suggested_date_copies_it_into_date_issued_and_redirects() {
    if !tesseract_available() {
        eprintln!("skipping accept_suggested_date_copies_it_into_date_issued_and_redirects: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "acceptdate.docs@example.com",
        "tests/fixtures/dated_sample.png",
        "dated_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");

    let response = common::post_with_cookie(&app, &format!("/documents/{}/accept_suggested_date", uploaded.id), &uploaded.cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response).unwrap(), format!("/documents/{}?saved=true", uploaded.id));

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains(r#"value="2024-03-15""#), "date_issued should now hold the accepted suggestion");
    assert!(!body.contains("Use this date"), "the suggestion box should disappear once accepted");
}

#[tokio::test]
async fn accept_suggested_date_never_overwrites_an_already_set_date_issued() {
    if !tesseract_available() {
        eprintln!("skipping accept_suggested_date_never_overwrites_an_already_set_date_issued: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "noaccept.docs@example.com",
        "tests/fixtures/dated_sample.png",
        "dated_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");

    let form_body = "title=Already+dated&tags=&date_issued=2020-01-01";
    let response =
        common::post_form_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie, form_body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let response = common::post_with_cookie(&app, &format!("/documents/{}/accept_suggested_date", uploaded.id), &uploaded.cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains(r#"value="2020-01-01""#),
        "accepting a suggestion must never overwrite a date_issued the user already set"
    );
}

#[tokio::test]
async fn accept_suggested_date_is_tenant_scoped() {
    if !tesseract_available() {
        eprintln!("skipping accept_suggested_date_is_tenant_scoped: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "tenantA.datesuggest@example.com",
        "tests/fixtures/dated_sample.png",
        "dated_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");

    let other_login = common::signup_and_login(&app, "tenantB.datesuggest@example.com", "documentspassword").await;
    let other_cookie = common::session_cookie(&other_login).expect("login should set a session cookie");

    let response =
        common::post_with_cookie(&app, &format!("/documents/{}/accept_suggested_date", uploaded.id), &other_cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("Use this date"), "another tenant's failed accept attempt must not consume the real owner's suggestion");
}
