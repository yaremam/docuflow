//! Shared facet-resolution primitives used by `/documents`' five facets
//! (tags, date year/month/undated, language, doc_type) and its applied-
//! filter chips. Deliberately has no dependency on the `documents` table
//! or SQL, so this module stays a plain, unit-testable core.
//!
//! `list()` still fetches each facet's narrowed counts itself (a plain
//! per-candidate loop calling `count_documents` — SQL-specific, and small
//! enough that sharing it generically isn't worth fighting Rust's current
//! rough edges around async closures + higher-ranked `Send` bounds).
//! What this module collapses is the *other* half — "given already-
//! fetched (candidate, count) pairs, decide checked and build the right
//! option" — which was hand-copied once per facet (plus a sixth near-copy
//! for applied-filter chips); [`assemble_facet_options`] is that shape
//! used five times instead, and it's pure enough to unit test with no
//! Postgres or axum involved.

/// The request's active `/documents` filter state, normalized once from
/// `ListQuery` — replaces three parallel shapes that used to exist side
/// by side (loose locals in `list()`, `FacetFilters`, and the positional
/// args `build_query_string`/`build_documents_url` took). Deliberately
/// excludes `sort`: sort is display-order state, not a filter dimension —
/// `count_documents`/`count_matching_documents` never take it, and the
/// two URL builders take it as a separate parameter alongside `&Self`.
#[derive(Debug, Clone)]
pub struct ActiveFilters {
    pub q: String,
    pub tags: Vec<String>,
    pub date_year: Option<i32>,
    /// Already gated: `None` unless `date_year` is also `Some` (TDR 015 §3).
    pub date_month: Option<i32>,
    pub undated: bool,
    /// Raw, including the `"unset"` sentinel — the shape a URL round-trips.
    pub lang: Vec<String>,
    /// Raw, including the `"unset"` sentinel — same shape as `lang`.
    pub doc_type: Vec<String>,
    /// `"expired"`/`"soon"`/`"later"`/`"unset"`, OR-combined (feature
    /// 031) — unlike `lang`/`doc_type`, none of these correspond to a
    /// literal stored column value (they're all computed from `date_
    /// expires` vs. today), so there's no separate values/unset split
    /// here — every string, "unset" included, is just checked directly
    /// against this same list in SQL (`= any(...)`).
    pub expiry_status: Vec<String>,
    q_tags: Option<Vec<String>>,
    lang_values: Vec<String>,
    lang_unset: bool,
    doc_type_values: Vec<String>,
    doc_type_unset: bool,
}

impl ActiveFilters {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        q: String,
        tags: Vec<String>,
        date_year: Option<i32>,
        date_month: Option<i32>,
        undated: bool,
        lang: Vec<String>,
        doc_type: Vec<String>,
        expiry_status: Vec<String>,
    ) -> Self {
        let q_tags = parse_tag_search(&q);
        let date_month = if date_year.is_some() {
            date_month
        } else {
            None
        };
        let lang_values: Vec<String> = lang
            .iter()
            .filter(|v| v.as_str() != "unset")
            .cloned()
            .collect();
        let lang_unset = lang.iter().any(|v| v == "unset");
        let doc_type_values: Vec<String> = doc_type
            .iter()
            .filter(|v| v.as_str() != "unset")
            .cloned()
            .collect();
        let doc_type_unset = doc_type.iter().any(|v| v == "unset");
        Self {
            q,
            tags,
            date_year,
            date_month,
            undated,
            lang,
            doc_type,
            expiry_status,
            q_tags,
            lang_values,
            lang_unset,
            doc_type_values,
            doc_type_unset,
        }
    }

    /// The search box's free-text half of `q` (feature 023) — `None` when
    /// `q` is empty, distinct from `Some("")`, since a `NULL` bind
    /// parameter (not an empty string) is what makes the OCR-match clause
    /// a no-op in `count_documents`'s SQL.
    pub fn search_text(&self) -> Option<&str> {
        free_text_search(&self.q)
    }

    /// The search box's own comma-parsed tag-overlap half of `q` — `None`
    /// (not `Some(&[])`) when `q` is empty, for the same SQL-null reason
    /// as `search_text`.
    pub fn q_tags(&self) -> Option<&[String]> {
        self.q_tags.as_deref()
    }

    pub fn lang_values(&self) -> &[String] {
        &self.lang_values
    }

    pub fn lang_unset(&self) -> bool {
        self.lang_unset
    }

    pub fn doc_type_values(&self) -> &[String] {
        &self.doc_type_values
    }

    pub fn doc_type_unset(&self) -> bool {
        self.doc_type_unset
    }

    /// Whether any facet or the free-text search box is active — gates
    /// the "Save this search" control (feature 016 AC-3) and rejects a
    /// no-op save server-side (TDR 016 AC-4).
    pub fn has_active_filters(&self) -> bool {
        self.has_active_facets() || !self.q.trim().is_empty()
    }

    /// Whether any *facet* (not the free-text box) is active — gates
    /// "Clear all" / the empty-vs-filtered-to-zero distinction (AC-8).
    pub fn has_active_facets(&self) -> bool {
        !self.tags.is_empty()
            || self.date_year.is_some()
            || self.undated
            || !self.lang.is_empty()
            || !self.doc_type.is_empty()
            || !self.expiry_status.is_empty()
    }
}

/// Parses the search box's comma-separated tag list into an overlap
/// filter. A deliberately ad hoc parse (not the `Tags` form newtype)
/// since this is a transient query filter, not data being stored.
fn parse_tag_search(q: &str) -> Option<Vec<String>> {
    let tags: Vec<String> = q
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect();

    if tags.is_empty() {
        None
    } else {
        Some(tags)
    }
}

/// The same search box's second, OR'd way to match a document: full-text
/// against `documents.ocr_search` (feature 023), independent of
/// `parse_tag_search`'s comma-split tag overlap.
fn free_text_search(q: &str) -> Option<&str> {
    let trimmed = q.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Turns already-fetched `(candidate, narrowed count)` pairs into
/// caller-shaped facet options — pure, no I/O, unit-testable without a
/// database. `is_checked` and `build` both close over whatever "is this
/// value currently active" state and whatever concrete `O` type (e.g.
/// `TagFacetOption`) the caller's template needs; this function only
/// owns the "for each candidate, decide checked, then build" shape that
/// used to be hand-copied once per facet.
pub fn assemble_facet_options<T, O>(
    candidates_with_counts: Vec<(T, i64)>,
    is_checked: impl Fn(&T) -> bool,
    build: impl Fn(T, i64, bool) -> O,
) -> Vec<O> {
    candidates_with_counts
        .into_iter()
        .map(|(candidate, count)| {
            let checked = is_checked(&candidate);
            build(candidate, count, checked)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filters(q: &str, tags: Vec<&str>, lang: Vec<&str>, doc_type: Vec<&str>) -> ActiveFilters {
        ActiveFilters::new(
            q.to_string(),
            tags.into_iter().map(str::to_string).collect(),
            None,
            None,
            false,
            lang.into_iter().map(str::to_string).collect(),
            doc_type.into_iter().map(str::to_string).collect(),
            vec![],
        )
    }

    #[test]
    fn empty_q_yields_no_search_text_and_no_q_tags() {
        let active = filters("", vec![], vec![], vec![]);
        assert_eq!(active.search_text(), None);
        assert_eq!(active.q_tags(), None);
    }

    #[test]
    fn whitespace_only_q_yields_no_search_text_and_no_q_tags() {
        let active = filters("   ", vec![], vec![], vec![]);
        assert_eq!(active.search_text(), None);
        assert_eq!(active.q_tags(), None);
    }

    #[test]
    fn multi_word_q_is_its_own_search_text_and_a_single_comma_free_tag() {
        let active = filters("electric company", vec![], vec![], vec![]);
        assert_eq!(active.search_text(), Some("electric company"));
        assert_eq!(
            active.q_tags(),
            Some(["electric company".to_string()].as_slice())
        );
    }

    #[test]
    fn comma_separated_q_splits_into_multiple_tags() {
        let active = filters("insurance, auto", vec![], vec![], vec![]);
        assert_eq!(
            active.q_tags(),
            Some(["insurance".to_string(), "auto".to_string()].as_slice())
        );
    }

    #[test]
    fn date_month_is_dropped_when_no_year_is_selected() {
        let active = ActiveFilters::new(
            "".to_string(),
            vec![],
            None,
            Some(3),
            false,
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(active.date_month, None);
    }

    #[test]
    fn date_month_survives_when_a_year_is_selected() {
        let active = ActiveFilters::new(
            "".to_string(),
            vec![],
            Some(2026),
            Some(3),
            false,
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(active.date_month, Some(3));
    }

    #[test]
    fn lang_splits_unset_sentinel_from_real_values() {
        let active = filters("", vec![], vec!["en", "unset", "cyr"], vec![]);
        assert_eq!(active.lang_values(), &["en".to_string(), "cyr".to_string()]);
        assert!(active.lang_unset());
    }

    #[test]
    fn lang_without_unset_sentinel_has_lang_unset_false() {
        let active = filters("", vec![], vec!["en"], vec![]);
        assert!(!active.lang_unset());
    }

    #[test]
    fn doc_type_splits_unset_sentinel_from_real_values() {
        let active = filters("", vec![], vec![], vec!["bill", "unset"]);
        assert_eq!(active.doc_type_values(), &["bill".to_string()]);
        assert!(active.doc_type_unset());
    }

    #[test]
    fn no_filters_and_no_q_has_no_active_filters() {
        let active = filters("", vec![], vec![], vec![]);
        assert!(!active.has_active_filters());
        assert!(!active.has_active_facets());
    }

    #[test]
    fn a_tag_facet_alone_counts_as_an_active_filter_and_facet() {
        let active = filters("", vec!["insurance"], vec![], vec![]);
        assert!(active.has_active_filters());
        assert!(active.has_active_facets());
    }

    #[test]
    fn free_text_q_alone_is_an_active_filter_but_not_an_active_facet() {
        let active = filters("electric", vec![], vec![], vec![]);
        assert!(active.has_active_filters());
        assert!(!active.has_active_facets());
    }

    #[derive(Debug, PartialEq)]
    struct FakeOption {
        name: String,
        count: i64,
        checked: bool,
    }

    #[test]
    fn assemble_facet_options_marks_checked_candidates_via_the_predicate() {
        let pairs = vec![("a".to_string(), 5_i64), ("b".to_string(), 2_i64)];
        let checked_name = "b".to_string();
        let options = assemble_facet_options(
            pairs,
            |name| *name == checked_name,
            |name, count, checked| FakeOption {
                name,
                count,
                checked,
            },
        );
        assert_eq!(
            options,
            vec![
                FakeOption {
                    name: "a".to_string(),
                    count: 5,
                    checked: false
                },
                FakeOption {
                    name: "b".to_string(),
                    count: 2,
                    checked: true
                },
            ]
        );
    }

    #[test]
    fn assemble_facet_options_on_an_empty_candidate_list_is_empty() {
        let options: Vec<FakeOption> = assemble_facet_options(
            Vec::<(String, i64)>::new(),
            |_: &String| false,
            |name, count, checked| FakeOption {
                name,
                count,
                checked,
            },
        );
        assert!(options.is_empty());
    }
}
