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
async fn the_use_this_date_button_is_not_a_form_nested_inside_the_metadata_form() {
    // Regression test for a real bug (found 2026-07-13): the "Use this
    // date" button used to be its own `<form action=".../accept_suggested_date">`
    // nested inside the page's single metadata-edit `<form action="/documents/{id}">`.
    // Nested `<form>` elements are invalid HTML — browsers drop the inner
    // form's opening tag but still process its closing tag against the
    // *outer* form, closing it early. Everything rendered after the
    // suggestion box (the Language field, the "Save changes" button) ended
    // up outside any form at all, silently breaking every edit once a
    // suggestion was showing — completely invisible to endpoint-level
    // tests (posting straight to a URL bypasses HTML parsing entirely,
    // which is exactly how this bug went unnoticed). Fixed with the same
    // `formaction`/`formmethod` button-override pattern features 016/017
    // already use for this exact reason.
    if !tesseract_available() {
        eprintln!("skipping the_use_this_date_button_is_not_a_form_nested_inside_the_metadata_form: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "nonestedform.docs@example.com",
        "tests/fixtures/dated_sample.png",
        "dated_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("Use this date"), "expected a date suggestion to be showing, got: {body}");

    let metadata_form_start = body.find(&format!("action=\"/documents/{}\"", uploaded.id)).expect("expected the metadata form");
    let suggestion_button = body.find("Use this date").expect("expected the suggestion button");
    assert!(metadata_form_start < suggestion_button, "the metadata form should open before the suggestion button");

    // No second `<form` between the outer form's opening tag and the
    // suggestion button — i.e. the button lives directly inside the one
    // metadata form, not a nested form of its own.
    let between = &body[metadata_form_start..suggestion_button];
    assert!(!between.contains("<form"), "the suggestion button must not be inside a nested <form>, got the region: {between}");

    // The button reaches its own endpoint via formaction/formmethod, not
    // via being wrapped in a second form.
    assert!(
        body.contains(&format!("formaction=\"/documents/{}/accept_suggested_date\"", uploaded.id)),
        "expected the suggestion button to use a formaction override, got: {body}"
    );
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

#[tokio::test]
async fn an_exif_capture_date_is_suggested_when_ocr_text_has_no_recognizable_date() {
    if !tesseract_available() {
        eprintln!("skipping an_exif_capture_date_is_suggested_when_ocr_text_has_no_recognizable_date: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "exifdatesuggest.docs@example.com",
        "tests/fixtures/exif_dated_sample.jpg",
        "exif_dated_sample.jpg",
        "image/jpeg",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done", "ocr should complete within the timeout even with no legible text");
    assert_eq!(
        uploaded.outcome.suggested_date_issued,
        Some(time::Date::from_calendar_date(2026, time::Month::March, 14).unwrap()),
        "expected the fixture's embedded EXIF DateTimeOriginal to be used as a fallback suggestion, got: {:?}",
        uploaded.outcome.suggested_date_issued
    );

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("Use this date"));
    assert!(body.contains("2026-03-14"));
}

/// Real-OCR, real-fixture coverage for feature 030's non-English month
/// names — reuses feature 020's language-detection fixtures rather than
/// inventing new ones. Unlike the fixture above (whose embedded date is
/// known and asserted exactly), these fixtures' exact OCR'd text isn't
/// verifiable in an environment without the `deu`/`nld`/`ukr` tesseract
/// packs installed (this sandbox has none of them), so these assert only
/// that *some* date was recognized — proof the non-English month-name
/// path fired at all — not a specific value. The precise-value unit
/// tests in `src/date_extract.rs` are what pin down exact correctness.
#[tokio::test]
async fn german_text_can_produce_a_date_suggestion() {
    if !tesseract_available() {
        eprintln!("skipping german_text_can_produce_a_date_suggestion: `tesseract` not found on PATH");
        return;
    }
    if !common::tesseract_has_lang("deu") {
        eprintln!("skipping german_text_can_produce_a_date_suggestion: tesseract-ocr-deu (deu.traineddata) not installed");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "germandate.docs@example.com",
        "tests/fixtures/german_sample.png",
        "german_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert!(
        uploaded.outcome.suggested_date_issued.is_some(),
        "expected a German month name in the fixture's OCR'd text to produce a date suggestion"
    );
}

#[tokio::test]
async fn dutch_text_can_produce_a_date_suggestion() {
    if !tesseract_available() {
        eprintln!("skipping dutch_text_can_produce_a_date_suggestion: `tesseract` not found on PATH");
        return;
    }
    if !common::tesseract_has_lang("nld") {
        eprintln!("skipping dutch_text_can_produce_a_date_suggestion: tesseract-ocr-nld (nld.traineddata) not installed");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "dutchdate.docs@example.com",
        "tests/fixtures/dutch_sample.png",
        "dutch_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert!(
        uploaded.outcome.suggested_date_issued.is_some(),
        "expected a Dutch month name in the fixture's OCR'd text to produce a date suggestion"
    );
}

#[tokio::test]
async fn ukrainian_text_can_produce_a_date_suggestion() {
    if !tesseract_available() {
        eprintln!("skipping ukrainian_text_can_produce_a_date_suggestion: `tesseract` not found on PATH");
        return;
    }
    if !common::tesseract_has_lang("ukr") {
        eprintln!("skipping ukrainian_text_can_produce_a_date_suggestion: tesseract-ocr-ukr (ukr.traineddata) not installed");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "ukrainiandate.docs@example.com",
        "tests/fixtures/ukrainian_sample.png",
        "ukrainian_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert!(
        uploaded.outcome.suggested_date_issued.is_some(),
        "expected a Ukrainian genitive month name in the fixture's OCR'd text to produce a date suggestion"
    );
}

#[tokio::test]
async fn an_ocr_derived_date_takes_priority_over_a_conflicting_exif_capture_date() {
    if !tesseract_available() {
        eprintln!("skipping an_ocr_derived_date_takes_priority_over_a_conflicting_exif_capture_date: `tesseract` not found on PATH");
        return;
    }

    // This fixture carries both a legible printed "March 15, 2024" date
    // (OCR-recognizable, same text as dated_sample.png) and an embedded
    // EXIF DateTimeOriginal of 2020-01-01 — a real, deliberately
    // conflicting second source, to prove OCR wins rather than merely
    // being present alongside an absent EXIF value.
    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "ocrwinsoverexif.docs@example.com",
        "tests/fixtures/dated_sample_with_exif.jpg",
        "dated_sample_with_exif.jpg",
        "image/jpeg",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert_eq!(
        uploaded.outcome.suggested_date_issued,
        Some(time::Date::from_calendar_date(2024, time::Month::March, 15).unwrap()),
        "the OCR-derived date should win over the fixture's conflicting EXIF date, got: {:?}",
        uploaded.outcome.suggested_date_issued
    );
}
