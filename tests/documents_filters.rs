mod common;

use common::user_id;

#[allow(clippy::too_many_arguments)]
async fn seed_document(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    tags: &[&str],
    date_issued: Option<time::Date>,
    language: Option<&str>,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, date_issued, ocr_status, language)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, $5, $6, 'done', $7)",
        id,
        user_id,
        filename,
        blob_key,
        &tags,
        date_issued,
        language,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

fn date(year: i32, month: u8, day: u8) -> time::Date {
    time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day).unwrap()
}

/// Finds the `filter-count` value immediately following a given facet
/// option's label — robust to exact template whitespace, unlike a raw
/// substring match, and reusable across the tags/date/language facets
/// since they all share the same `filter-option-label`/`filter-count`
/// markup shape.
fn facet_count_after_label(body: &str, label: &str) -> Option<String> {
    let label_marker = format!("filter-option-label\">{label}</span>");
    let after_label = &body[body.find(&label_marker)? + label_marker.len()..];
    let count_marker = "filter-count\">";
    let after_count_open = &after_label[after_label.find(count_marker)? + count_marker.len()..];
    let end = after_count_open.find('<')?;
    Some(after_count_open[..end].to_string())
}

#[tokio::test]
async fn tag_facet_narrows_to_documents_with_all_selected_tags() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "tagfacet.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "tagfacet.docs@example.com").await;

    seed_document(&app.state.pool, user, "both_tags.pdf", &["insurance", "medical"], None, None).await;
    seed_document(&app.state.pool, user, "one_tag.pdf", &["insurance"], None, None).await;

    let response = common::get_with_cookie(&app, "/documents?tags=insurance&tags=medical", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("both_tags.pdf"), "expected a document with both selected tags, got: {body}");
    assert!(!body.contains("one_tag.pdf"), "a document missing one of the selected tags should be excluded, got: {body}");
}

#[tokio::test]
async fn tag_facet_is_independent_of_the_free_text_search_box() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "tagfacetq.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "tagfacetq.docs@example.com").await;

    seed_document(&app.state.pool, user, "matches_both.pdf", &["insurance", "auto"], None, None).await;
    seed_document(&app.state.pool, user, "matches_facet_only.pdf", &["insurance"], None, None).await;
    seed_document(&app.state.pool, user, "matches_q_only.pdf", &["auto"], None, None).await;

    // q is an OR-search for "auto"; the tags facet is an AND-narrow for "insurance" —
    // the two conditions AND together.
    let response = common::get_with_cookie(&app, "/documents?q=auto&tags=insurance", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("matches_both.pdf"), "expected the doc matching both conditions, got: {body}");
    assert!(!body.contains("matches_facet_only.pdf"), "q=auto should still exclude docs that don't match it, got: {body}");
    assert!(!body.contains("matches_q_only.pdf"), "tags=insurance should still exclude docs that don't have it, got: {body}");
}

async fn set_ocr_text(pool: &sqlx::PgPool, document_id: uuid::Uuid, ocr_text: &str) {
    sqlx::query!("update documents set ocr_text = $1 where id = $2", ocr_text, document_id).execute(pool).await.unwrap();
}

async fn set_doc_type(pool: &sqlx::PgPool, document_id: uuid::Uuid, doc_type: &str) {
    sqlx::query!("update documents set doc_type = $1 where id = $2", doc_type, document_id).execute(pool).await.unwrap();
}

#[tokio::test]
async fn doc_type_facet_filters_to_documents_with_selected_type() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "doctypefacet.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "doctypefacet.docs@example.com").await;

    let bill = seed_document(&app.state.pool, user, "bill.pdf", &["utilities"], None, None).await;
    set_doc_type(&app.state.pool, bill, "bill").await;
    let insurance = seed_document(&app.state.pool, user, "insurance.pdf", &["utilities"], None, None).await;
    set_doc_type(&app.state.pool, insurance, "insurance").await;

    let response = common::get_with_cookie(&app, "/documents?doc_type=bill", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("bill.pdf"));
    assert!(!body.contains("insurance.pdf"));
}

#[tokio::test]
async fn doc_type_facet_unset_option_filters_to_documents_without_a_type() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "doctypeunset.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "doctypeunset.docs@example.com").await;

    let bill = seed_document(&app.state.pool, user, "bill.pdf", &["utilities"], None, None).await;
    set_doc_type(&app.state.pool, bill, "bill").await;
    seed_document(&app.state.pool, user, "no_type.pdf", &["utilities"], None, None).await;

    let response = common::get_with_cookie(&app, "/documents?doc_type=unset", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("no_type.pdf"));
    assert!(!body.contains("bill.pdf"));
}

#[tokio::test]
async fn doc_type_facet_ors_multiple_selected_values() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "doctypeor.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "doctypeor.docs@example.com").await;

    let bill = seed_document(&app.state.pool, user, "bill.pdf", &["utilities"], None, None).await;
    set_doc_type(&app.state.pool, bill, "bill").await;
    let insurance = seed_document(&app.state.pool, user, "insurance.pdf", &["utilities"], None, None).await;
    set_doc_type(&app.state.pool, insurance, "insurance").await;
    seed_document(&app.state.pool, user, "no_type.pdf", &["utilities"], None, None).await;

    let response = common::get_with_cookie(&app, "/documents?doc_type=bill&doc_type=insurance", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("bill.pdf"));
    assert!(body.contains("insurance.pdf"));
    assert!(!body.contains("no_type.pdf"));
}

#[tokio::test]
async fn doc_type_facet_counts_narrow_when_a_tag_filter_is_active() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "doctypecountnarrow.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "doctypecountnarrow.docs@example.com").await;

    let bill_insurance = seed_document(&app.state.pool, user, "bill_insurance.pdf", &["insurance"], None, None).await;
    set_doc_type(&app.state.pool, bill_insurance, "bill").await;
    let bill_utilities = seed_document(&app.state.pool, user, "bill_utilities.pdf", &["utilities"], None, None).await;
    set_doc_type(&app.state.pool, bill_utilities, "bill").await;

    let unfiltered = common::get_with_cookie(&app, "/documents", &cookie).await;
    let unfiltered_body = common::body_string(unfiltered).await;
    assert_eq!(facet_count_after_label(&unfiltered_body, "Bill").as_deref(), Some("2"), "unfiltered: both bills count, got: {unfiltered_body}");

    let filtered = common::get_with_cookie(&app, "/documents?tags=insurance", &cookie).await;
    let filtered_body = common::body_string(filtered).await;
    assert_eq!(
        facet_count_after_label(&filtered_body, "Bill").as_deref(),
        Some("1"),
        "with tags=insurance active, only the insurance-tagged bill should count, got: {filtered_body}"
    );
}

#[tokio::test]
async fn free_text_search_matches_ocr_text_not_just_tags() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "ocrsearch.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "ocrsearch.docs@example.com").await;

    let matching = seed_document(&app.state.pool, user, "verizon_bill.pdf", &["bill"], None, None).await;
    set_ocr_text(&app.state.pool, matching, "Verizon Wireless monthly invoice, account ending 4521").await;
    let non_matching = seed_document(&app.state.pool, user, "other_bill.pdf", &["bill"], None, None).await;
    set_ocr_text(&app.state.pool, non_matching, "Acme Water Utility statement").await;

    let response = common::get_with_cookie(&app, "/documents?q=verizon", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("verizon_bill.pdf"), "expected a doc whose OCR text (not tags) matches q, got: {body}");
    assert!(!body.contains("other_bill.pdf"), "a doc whose OCR text doesn't match q should be excluded, got: {body}");
}

#[tokio::test]
async fn free_text_search_with_no_ocr_or_tag_match_excludes_the_document() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "ocrsearchmiss.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "ocrsearchmiss.docs@example.com").await;

    let doc = seed_document(&app.state.pool, user, "unrelated.pdf", &["bill"], None, None).await;
    set_ocr_text(&app.state.pool, doc, "Acme Water Utility statement").await;

    let response = common::get_with_cookie(&app, "/documents?q=verizon", &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("unrelated.pdf"), "q matching neither tags nor OCR text should exclude the doc, got: {body}");
}

#[tokio::test]
async fn free_text_search_matches_a_multi_word_phrase_across_ocr_text() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "ocrsearchphrase.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "ocrsearchphrase.docs@example.com").await;

    let both_words = seed_document(&app.state.pool, user, "electric_company.pdf", &["bill"], None, None).await;
    set_ocr_text(&app.state.pool, both_words, "Springfield Electric Company annual statement").await;
    let one_word = seed_document(&app.state.pool, user, "electric_car.pdf", &["bill"], None, None).await;
    set_ocr_text(&app.state.pool, one_word, "Electric car charging receipt").await;

    let response = common::get_with_cookie(&app, "/documents?q=electric+company", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("electric_company.pdf"), "expected the doc containing both words, got: {body}");
    assert!(!body.contains("electric_car.pdf"), "a doc missing one of the words shouldn't match a multi-word q, got: {body}");
}

#[tokio::test]
async fn free_text_ocr_search_combines_with_an_active_tags_facet() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "ocrsearchtags.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "ocrsearchtags.docs@example.com").await;

    let matches_both = seed_document(&app.state.pool, user, "insurance_verizon.pdf", &["insurance"], None, None).await;
    set_ocr_text(&app.state.pool, matches_both, "Verizon Wireless invoice").await;
    let wrong_tag = seed_document(&app.state.pool, user, "auto_verizon.pdf", &["auto"], None, None).await;
    set_ocr_text(&app.state.pool, wrong_tag, "Verizon Wireless invoice").await;

    let response = common::get_with_cookie(&app, "/documents?q=verizon&tags=insurance", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("insurance_verizon.pdf"), "expected the doc matching both q's OCR text and the tags facet, got: {body}");
    assert!(!body.contains("auto_verizon.pdf"), "tags facet should still AND-narrow even though q matches via OCR text, got: {body}");
}

#[tokio::test]
async fn free_text_search_result_shows_a_highlighted_snippet_from_matching_ocr_text() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "snippet.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "snippet.docs@example.com").await;

    let doc = seed_document(&app.state.pool, user, "electric_bill.pdf", &["bill"], None, None).await;
    set_ocr_text(&app.state.pool, doc, "Springfield Electric Company annual statement for March").await;

    let response = common::get_with_cookie(&app, "/documents?q=electric+company", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("doc-row-snippet"), "expected a snippet line under the matching row, got: {body}");
    assert!(body.contains("<mark>Electric</mark>"), "expected the matched word marked in the snippet, got: {body}");
    assert!(body.contains("<mark>Company</mark>"), "expected the matched word marked in the snippet, got: {body}");
}

#[tokio::test]
async fn free_text_search_result_matched_via_tag_overlap_only_shows_no_snippet() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "nosnippet.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "nosnippet.docs@example.com").await;

    // Matches q via the search box's own comma-parsed tag-overlap (feature
    // 023's `parse_tag_search`), not via its OCR text — no OCR hit means
    // no excerpt to show (AC-2).
    let doc = seed_document(&app.state.pool, user, "renewal_notice.pdf", &["electric company"], None, None).await;
    set_ocr_text(&app.state.pool, doc, "Your policy is due for renewal next month").await;

    let response = common::get_with_cookie(&app, "/documents?q=electric+company", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("renewal_notice.pdf"), "expected the doc to match via tag overlap, got: {body}");
    assert!(!body.contains("doc-row-snippet"), "a tag-only match shouldn't render a snippet line, got: {body}");
}

#[tokio::test]
async fn free_text_search_result_link_carries_q_into_the_detail_page() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "carryq.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "carryq.docs@example.com").await;

    let doc = seed_document(&app.state.pool, user, "electric_bill2.pdf", &["bill"], None, None).await;
    set_ocr_text(&app.state.pool, doc, "Electric Company statement").await;

    let response = common::get_with_cookie(&app, "/documents?q=electric+company", &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains(&format!("/documents/{doc}?q=electric%20company")),
        "expected the row's link to carry the active search q through to the detail page, got: {body}"
    );
}

#[tokio::test]
async fn language_facet_filters_by_selected_language() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "langfacet.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "langfacet.docs@example.com").await;

    seed_document(&app.state.pool, user, "english.pdf", &["bill"], None, Some("en")).await;
    seed_document(&app.state.pool, user, "german.pdf", &["bill"], None, Some("de")).await;

    let response = common::get_with_cookie(&app, "/documents?lang=en", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("english.pdf"));
    assert!(!body.contains("german.pdf"));
}

#[tokio::test]
async fn language_facet_unset_option_filters_to_documents_without_a_language() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "langunset.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "langunset.docs@example.com").await;

    seed_document(&app.state.pool, user, "english.pdf", &["bill"], None, Some("en")).await;
    seed_document(&app.state.pool, user, "no_language.pdf", &["bill"], None, None).await;

    let response = common::get_with_cookie(&app, "/documents?lang=unset", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("no_language.pdf"));
    assert!(!body.contains("english.pdf"));
}

#[tokio::test]
async fn language_facet_ors_multiple_selected_values() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "langor.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "langor.docs@example.com").await;

    seed_document(&app.state.pool, user, "english.pdf", &["bill"], None, Some("en")).await;
    seed_document(&app.state.pool, user, "german.pdf", &["bill"], None, Some("de")).await;
    seed_document(&app.state.pool, user, "unset_lang.pdf", &["bill"], None, None).await;

    let response = common::get_with_cookie(&app, "/documents?lang=en&lang=de", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("english.pdf"));
    assert!(body.contains("german.pdf"));
    assert!(!body.contains("unset_lang.pdf"));
}

#[tokio::test]
async fn date_year_facet_filters_to_documents_issued_in_that_year() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "dateyear.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "dateyear.docs@example.com").await;

    seed_document(&app.state.pool, user, "in_2026.pdf", &["bill"], Some(date(2026, 3, 14)), None).await;
    seed_document(&app.state.pool, user, "in_2025.pdf", &["bill"], Some(date(2025, 6, 1)), None).await;

    let response = common::get_with_cookie(&app, "/documents?date_year=2026", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("in_2026.pdf"));
    assert!(!body.contains("in_2025.pdf"));
}

#[tokio::test]
async fn date_year_and_month_facet_narrows_further() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "datemonth.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "datemonth.docs@example.com").await;

    seed_document(&app.state.pool, user, "march.pdf", &["bill"], Some(date(2026, 3, 14)), None).await;
    seed_document(&app.state.pool, user, "july.pdf", &["bill"], Some(date(2026, 7, 2)), None).await;

    let response = common::get_with_cookie(&app, "/documents?date_year=2026&date_month=3", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("march.pdf"));
    assert!(!body.contains("july.pdf"));
}

#[tokio::test]
async fn undated_facet_filters_to_documents_without_a_date_issued() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "undated.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "undated.docs@example.com").await;

    seed_document(&app.state.pool, user, "has_date.pdf", &["bill"], Some(date(2026, 3, 14)), None).await;
    seed_document(&app.state.pool, user, "no_date.pdf", &["bill"], None, None).await;

    let response = common::get_with_cookie(&app, "/documents?undated=true", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("no_date.pdf"));
    assert!(!body.contains("has_date.pdf"));
}

#[tokio::test]
async fn undated_ors_with_an_active_year_selection() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "undatedor.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "undatedor.docs@example.com").await;

    seed_document(&app.state.pool, user, "in_2026.pdf", &["bill"], Some(date(2026, 3, 14)), None).await;
    seed_document(&app.state.pool, user, "no_date.pdf", &["bill"], None, None).await;
    seed_document(&app.state.pool, user, "in_2025.pdf", &["bill"], Some(date(2025, 1, 1)), None).await;

    let response = common::get_with_cookie(&app, "/documents?date_year=2026&undated=true", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("in_2026.pdf"));
    assert!(body.contains("no_date.pdf"));
    assert!(!body.contains("in_2025.pdf"));
}

#[tokio::test]
async fn facets_combine_with_each_other_and_with_search_and_sort() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "combo.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "combo.docs@example.com").await;

    seed_document(&app.state.pool, user, "matches_all.pdf", &["insurance"], Some(date(2026, 3, 14)), Some("en")).await;
    seed_document(&app.state.pool, user, "wrong_language.pdf", &["insurance"], Some(date(2026, 3, 14)), Some("de")).await;
    seed_document(&app.state.pool, user, "wrong_year.pdf", &["insurance"], Some(date(2025, 3, 14)), Some("en")).await;

    let response = common::get_with_cookie(&app, "/documents?tags=insurance&date_year=2026&lang=en&sort=date_issued_desc", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("matches_all.pdf"));
    assert!(!body.contains("wrong_language.pdf"));
    assert!(!body.contains("wrong_year.pdf"));
}

#[tokio::test]
async fn default_view_with_no_facet_params_matches_pre_015_behavior() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "nofacet.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "nofacet.docs@example.com").await;

    seed_document(&app.state.pool, user, "any_doc.pdf", &["insurance"], Some(date(2026, 3, 14)), Some("en")).await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("any_doc.pdf"), "with no facet params, every document should still show, got: {body}");
}

#[tokio::test]
async fn filters_panel_shows_tag_counts() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "tagcounts.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "tagcounts.docs@example.com").await;

    seed_document(&app.state.pool, user, "a.pdf", &["insurance"], None, None).await;
    seed_document(&app.state.pool, user, "b.pdf", &["insurance"], None, None).await;
    seed_document(&app.state.pool, user, "c.pdf", &["utilities"], None, None).await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains(r#"value="insurance""#) && body.contains("insurance") && body.contains(">2<"),
        "expected a tag facet option for insurance with a count of 2, got: {body}"
    );
}

#[tokio::test]
async fn active_filters_render_as_removable_chips() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "chips.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "chips.docs@example.com").await;

    seed_document(&app.state.pool, user, "a.pdf", &["insurance"], None, Some("en")).await;

    let response = common::get_with_cookie(&app, "/documents?tags=insurance&lang=en", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("filter-chip"), "expected at least one applied-filter chip, got: {body}");
    // Removing the "insurance" tag chip should link to a URL that keeps the language filter
    // but drops the tag.
    assert!(
        body.contains("lang=en") && !body.contains("tags=insurance&amp;lang=en\" class=\"filter-chip\""),
        "expected the tag's own removal link not to include itself, got: {body}"
    );
}

#[tokio::test]
async fn zero_results_from_filters_shows_a_distinct_empty_state() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "zerofilter.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "zerofilter.docs@example.com").await;

    seed_document(&app.state.pool, user, "a.pdf", &["insurance"], None, None).await;

    let response = common::get_with_cookie(&app, "/documents?tags=nonexistent-tag", &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains("No documents match these filters"),
        "expected the filtered-to-zero empty state, got: {body}"
    );
    assert!(!body.contains("No documents yet"), "the true first-run empty state should not show when filters are just narrow, got: {body}");
}

#[tokio::test]
async fn tag_facet_counts_narrow_when_a_language_filter_is_active() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "tagcountnarrow.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "tagcountnarrow.docs@example.com").await;

    seed_document(&app.state.pool, user, "en_insurance.pdf", &["insurance"], None, Some("en")).await;
    seed_document(&app.state.pool, user, "de_insurance.pdf", &["insurance"], None, Some("de")).await;
    seed_document(&app.state.pool, user, "en_utilities.pdf", &["utilities"], None, Some("en")).await;

    let unfiltered = common::get_with_cookie(&app, "/documents", &cookie).await;
    let unfiltered_body = common::body_string(unfiltered).await;
    assert_eq!(facet_count_after_label(&unfiltered_body, "insurance").as_deref(), Some("2"), "unfiltered: both insurance docs count, got: {unfiltered_body}");

    let filtered = common::get_with_cookie(&app, "/documents?lang=en", &cookie).await;
    let filtered_body = common::body_string(filtered).await;
    assert_eq!(
        facet_count_after_label(&filtered_body, "insurance").as_deref(),
        Some("1"),
        "with lang=en active, only the English insurance doc should count, got: {filtered_body}"
    );
}

#[tokio::test]
async fn language_facet_counts_narrow_when_a_tag_filter_is_active() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "langcountnarrow.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "langcountnarrow.docs@example.com").await;

    seed_document(&app.state.pool, user, "en_insurance.pdf", &["insurance"], None, Some("en")).await;
    seed_document(&app.state.pool, user, "en_utilities.pdf", &["utilities"], None, Some("en")).await;
    seed_document(&app.state.pool, user, "de_insurance.pdf", &["insurance"], None, Some("de")).await;

    let unfiltered = common::get_with_cookie(&app, "/documents", &cookie).await;
    let unfiltered_body = common::body_string(unfiltered).await;
    assert_eq!(facet_count_after_label(&unfiltered_body, "English").as_deref(), Some("2"), "unfiltered: both English docs count, got: {unfiltered_body}");

    let filtered = common::get_with_cookie(&app, "/documents?tags=insurance", &cookie).await;
    let filtered_body = common::body_string(filtered).await;
    assert_eq!(
        facet_count_after_label(&filtered_body, "English").as_deref(),
        Some("1"),
        "with tags=insurance active, only the English insurance doc should count, got: {filtered_body}"
    );
}

#[tokio::test]
async fn date_facet_counts_narrow_when_a_tag_filter_is_active() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "datecountnarrow.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "datecountnarrow.docs@example.com").await;

    seed_document(&app.state.pool, user, "insurance_2026.pdf", &["insurance"], Some(date(2026, 3, 1)), None).await;
    seed_document(&app.state.pool, user, "utilities_2026.pdf", &["utilities"], Some(date(2026, 5, 1)), None).await;

    let unfiltered = common::get_with_cookie(&app, "/documents", &cookie).await;
    let unfiltered_body = common::body_string(unfiltered).await;
    assert_eq!(facet_count_after_label(&unfiltered_body, "2026").as_deref(), Some("2"), "unfiltered: both 2026 docs count, got: {unfiltered_body}");

    let filtered = common::get_with_cookie(&app, "/documents?tags=insurance", &cookie).await;
    let filtered_body = common::body_string(filtered).await;
    assert_eq!(
        facet_count_after_label(&filtered_body, "2026").as_deref(),
        Some("1"),
        "with tags=insurance active, only the insurance 2026 doc should count, got: {filtered_body}"
    );
}
