// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Unit tests for the feeds widget. Split out of `mod.rs` per the repo standard.

use super::*;

fn article(topic: &str, url: &str, minutes_ago: i64) -> FeedArticle {
    FeedArticle {
        title: url.to_string(),
        url: url.to_string(),
        topic: topic.to_string(),
        source: "src".to_string(),
        published: chrono::Utc::now() - chrono::Duration::minutes(minutes_ago),
        summary: None,
        hero_image_url: None,
        authors: vec![],
    }
}

/// A topic whose feed failed this round must keep its previous articles
/// instead of vanishing, while a topic that succeeded gets fully replaced
/// by the fresh fetch.
#[test]
fn merge_with_stale_topics_keeps_previous_articles_for_failed_topics() {
    let previous: Vec<Arc<FeedArticle>> = vec![
        Arc::new(article("Reddit", "https://reddit/old", 60)),
        Arc::new(article("WSJ", "https://wsj/old", 60)),
    ];
    let outcome = FetchOutcome {
        articles: vec![article("WSJ", "https://wsj/new", 1)],
        failed_topics: ["Reddit".to_string()].into_iter().collect(),
    };

    let merged = merge_with_stale_topics(outcome, &previous);

    assert!(
        merged.iter().any(|a| a.url == "https://reddit/old"),
        "stale Reddit article should be carried forward, not dropped"
    );
    assert!(merged.iter().any(|a| a.url == "https://wsj/new"));
    assert!(
        !merged.iter().any(|a| a.url == "https://wsj/old"),
        "successfully-refreshed topic should not keep its stale article"
    );
}

/// When every feed succeeds, no stale articles are pulled in.
#[test]
fn merge_with_stale_topics_is_a_noop_when_nothing_failed() {
    let previous: Vec<Arc<FeedArticle>> = vec![Arc::new(article("WSJ", "https://wsj/old", 60))];
    let outcome = FetchOutcome {
        articles: vec![article("WSJ", "https://wsj/new", 1)],
        failed_topics: Default::default(),
    };

    let merged = merge_with_stale_topics(outcome, &previous);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].url, "https://wsj/new");
}

#[test]
fn summary_length_cycles_short_medium_long() {
    assert_eq!(SummaryLength::Short.next(), SummaryLength::Medium);
    assert_eq!(SummaryLength::Medium.next(), SummaryLength::Long);
    assert_eq!(SummaryLength::Long.next(), SummaryLength::Short);
}

#[test]
fn summary_length_max_tokens_increases_with_length() {
    assert!(SummaryLength::Short.max_tokens() < SummaryLength::Medium.max_tokens());
    assert!(SummaryLength::Medium.max_tokens() < SummaryLength::Long.max_tokens());
}

#[test]
fn summary_keys_differ_by_length() {
    let url = "https://www.wsj.com/articles/xyz";
    assert_ne!(
        summary_key(url, SummaryLength::Short),
        summary_key(url, SummaryLength::Medium)
    );
    assert_ne!(
        summary_cache_key(url, SummaryLength::Short),
        summary_cache_key(url, SummaryLength::Medium)
    );
}

#[test]
fn split_widths_gives_list_60pct_when_room_for_summary() {
    // 200 cols → list = 120 (60%), summary = 80.
    let (l, s) = split_list_summary_widths(200);
    assert_eq!(l, 120);
    assert_eq!(s, 80);
}

#[test]
fn split_widths_caps_list_to_preserve_summary_min() {
    // 90 cols, 60% = 54 → would leave 36 for summary which is
    // < 45 minimum. List must shrink to 90-45 = 45.
    let (l, s) = split_list_summary_widths(90);
    assert_eq!(l, 45);
    assert_eq!(s, 45);
}

#[test]
fn split_widths_collapses_when_total_too_narrow() {
    // 50 cols can't satisfy 45-col summary + 20-col list min.
    let (l, s) = split_list_summary_widths(50);
    assert_eq!(s, 0, "summary collapsed");
    assert_eq!(l, 50, "list takes everything");
}

#[test]
fn wrap_title_returns_single_line_when_fits() {
    let out = wrap_title("hello world", 20, 3);
    assert_eq!(out, vec!["hello world"]);
}

#[test]
fn wrap_title_wraps_on_word_boundary() {
    let out = wrap_title("the quick brown fox jumps", 10, 3);
    for line in &out {
        assert!(line.chars().count() <= 10, "line over budget: {line:?}");
    }
    // Words shouldn't be split mid-word.
    assert!(out
        .iter()
        .all(|l| !l.contains(" the ") || l.starts_with("the ")));
}

#[test]
fn wrap_title_caps_at_max_lines_with_ellipsis() {
    let out = wrap_title(
        "one two three four five six seven eight nine ten eleven",
        8,
        3,
    );
    assert_eq!(out.len(), 3);
    assert!(out[2].ends_with('…'), "last line should end with …");
}

#[test]
fn wrap_title_handles_very_long_word() {
    // A word longer than width gets mid-broken.
    let out = wrap_title("verylongunbreakableword", 5, 2);
    assert_eq!(out[0].chars().count(), 5);
}

#[test]
fn format_relative_time_buckets_cover_the_common_ranges() {
    use chrono::Duration as ChronoDuration;
    let now = chrono::Utc::now();
    assert_eq!(format_relative_time(now, now), "now");
    assert_eq!(
        format_relative_time(now - ChronoDuration::seconds(30), now),
        "now"
    );
    assert_eq!(
        format_relative_time(now - ChronoDuration::minutes(5), now),
        "5m"
    );
    assert_eq!(
        format_relative_time(now - ChronoDuration::minutes(59), now),
        "59m"
    );
    assert_eq!(
        format_relative_time(now - ChronoDuration::hours(3), now),
        "3h"
    );
    assert_eq!(
        format_relative_time(now - ChronoDuration::hours(23), now),
        "23h"
    );
    assert_eq!(
        format_relative_time(now - ChronoDuration::days(2), now),
        "2d"
    );
    assert_eq!(
        format_relative_time(now - ChronoDuration::days(6), now),
        "6d"
    );
    assert_eq!(
        format_relative_time(now - ChronoDuration::days(14), now),
        "2w"
    );
}

#[test]
fn format_relative_time_falls_back_to_month_day_after_a_few_weeks() {
    use chrono::Duration as ChronoDuration;
    let now = chrono::Utc::now();
    let out = format_relative_time(now - ChronoDuration::days(60), now);
    // Expect "Mon DD" — 6 chars, contains a space.
    assert_eq!(out.chars().count(), 6, "{out:?}");
    assert!(out.contains(' '));
}

#[test]
fn format_relative_time_handles_future_timestamps() {
    use chrono::Duration as ChronoDuration;
    let now = chrono::Utc::now();
    // Article published "later" than now → clamp to "now"
    // rather than emitting negative buckets.
    assert_eq!(
        format_relative_time(now + ChronoDuration::minutes(5), now),
        "now"
    );
}

#[test]
fn article_prefix_width_matches_marker_plus_topic_tag() {
    // "▶ " (counted as 2 chars in display terms — `▶` + space) +
    // "[" + topic + "]" + trailing " ".
    assert_eq!(article_prefix_width("Tech"), 2 + 1 + 4 + 1 + 1);
    assert_eq!(article_prefix_width("Politics"), 2 + 1 + 8 + 1 + 1);
}

// ─────────────────────────────────────────────────────────────────────
// expand_user_set / auto-expand render tests
// ─────────────────────────────────────────────────────────────────────

fn build_widget_for_expand_tests() -> FeedsWidget {
    use std::sync::Arc;

    let cfg = FeedsConfig {
        feeds: vec![FeedSpec {
            topic: "Test".to_string(),
            url: "https://example.com/feed".to_string(),
        }],
        ..FeedsConfig::default()
    };
    let widget = FeedsWidget::with_config(
        "main".to_string(),
        cfg,
        Arc::new(crate::theme::Theme::builtin_defaults()),
        crate::cache::ScopedCache::ephemeral(),
        None,
    );
    // Seed one article so selected_article() returns Some.
    {
        use chrono::Utc;
        let article = Arc::new(provider::FeedArticle {
            title: "Test Expand Article Title".to_string(),
            url: "https://example.com/article/1".to_string(),
            topic: "Test".to_string(),
            source: "example.com".to_string(),
            published: Utc::now(),
            summary: Some("Test article body.".to_string()),
            hero_image_url: None,
            authors: vec![],
        });
        widget
            .state
            .lock()
            .unwrap()
            .articles
            .push(article);
    }
    widget
}

/// At Full size (105 × 30), with no manual toggle, the article panel
/// auto-expands (expand_user_set = false, tier = Full → effective = true).
/// expanded_rect is Some after render.
#[test]
fn full_tier_auto_expands_when_no_manual_toggle() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let (w, h) = (
        crate::widgets::view_tier::FULL_MIN_W,
        crate::widgets::view_tier::FULL_MIN_H,
    );
    assert_eq!(
        crate::widgets::ViewTier::from_rect(ratatui::layout::Rect::new(0, 0, w, h)),
        crate::widgets::ViewTier::Full,
        "precondition: must resolve to Full"
    );

    let widget = build_widget_for_expand_tests();
    // No manual toggle — expand_user_set stays false.
    assert!(!widget.state.lock().unwrap().expand_user_set);

    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| widget.render(frame, frame.area(), false))
        .unwrap();

    assert!(
        widget.state.lock().unwrap().expanded_rect.is_some(),
        "expanded panel must be shown automatically at Full tier"
    );
}

/// At Standard size (50 × 20), the tier is not Full so auto-expand does
/// not fire. expanded_rect remains None.
#[test]
fn non_full_tier_does_not_auto_expand() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let (w, h) = (50u16, 20u16);
    assert_eq!(
        crate::widgets::ViewTier::from_rect(ratatui::layout::Rect::new(0, 0, w, h)),
        crate::widgets::ViewTier::Standard,
        "precondition: must resolve to Standard"
    );

    let widget = build_widget_for_expand_tests();

    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| widget.render(frame, frame.area(), false))
        .unwrap();

    assert!(
        widget.state.lock().unwrap().expanded_rect.is_none(),
        "expanded panel must not show at Standard tier without a manual toggle"
    );
}

/// At Full size, a manual toggle-off (expand_user_set = true, expanded = false)
/// overrides auto-expand — the panel stays collapsed.
#[test]
fn manual_toggle_off_overrides_auto_expand_at_full() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let (w, h) = (
        crate::widgets::view_tier::FULL_MIN_W,
        crate::widgets::view_tier::FULL_MIN_H,
    );

    let widget = build_widget_for_expand_tests();
    // Simulate the user explicitly pressing `e` to collapse.
    {
        let mut st = widget.state.lock().unwrap();
        st.expand_user_set = true;
        st.expanded = false;
    }

    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| widget.render(frame, frame.area(), false))
        .unwrap();

    assert!(
        widget.state.lock().unwrap().expanded_rect.is_none(),
        "manual toggle-off must keep the panel collapsed even at Full tier"
    );
}

// ─────────────────────────────────────────────────────────────────────
// AI digest
// ─────────────────────────────────────────────────────────────────────

fn valid_urls(urls: &[&str]) -> std::collections::HashSet<String> {
    urls.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_digest_accepts_well_formed_json() {
    let json = r#"{"topics": [{"title": "Models", "subtopics": [
        {"title": "Open weights", "items": [
            {"title": "New model released", "url": "https://a", "source": "HF"}
        ]}
    ]}]}"#;
    let taxonomy = parse_digest(json, &valid_urls(&["https://a"])).expect("should parse");
    assert_eq!(taxonomy.topics.len(), 1);
    assert_eq!(taxonomy.topics[0].subtopics[0].items[0].url, "https://a");
}

#[test]
fn parse_digest_strips_markdown_code_fence() {
    let json = "```json\n{\"topics\": [{\"title\": \"T\", \"subtopics\": [{\"title\": \"S\", \"items\": [{\"title\": \"x\", \"url\": \"https://a\", \"source\": \"HF\"}]}]}]}\n```";
    let taxonomy = parse_digest(json, &valid_urls(&["https://a"])).expect("should parse");
    assert_eq!(taxonomy.topics.len(), 1);
}

#[test]
fn parse_digest_rejects_plain_prose() {
    assert!(parse_digest("Sure, here are the topics: ...", &valid_urls(&[])).is_none());
}

/// The one gotcha this feature deliberately avoids: an LLM reply that's
/// syntactically valid JSON but references URLs we never sent (hallucinated
/// or rewritten links) must not be trusted verbatim.
#[test]
fn parse_digest_drops_items_with_urls_we_never_sent() {
    let json = r#"{"topics": [{"title": "T", "subtopics": [{"title": "S", "items": [
        {"title": "real", "url": "https://a", "source": "HF"},
        {"title": "hallucinated", "url": "https://not-sent", "source": "HF"}
    ]}]}]}"#;
    let taxonomy = parse_digest(json, &valid_urls(&["https://a"])).expect("should parse");
    let items = &taxonomy.topics[0].subtopics[0].items;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].url, "https://a");
}

/// If every item in a subtopic gets filtered out, the now-empty subtopic
/// (and topic, if it becomes empty too) must be pruned rather than left
/// as a dangling empty node.
#[test]
fn parse_digest_prunes_now_empty_subtopics_and_topics() {
    let json = r#"{"topics": [{"title": "T", "subtopics": [{"title": "S", "items": [
        {"title": "hallucinated", "url": "https://not-sent", "source": "HF"}
    ]}]}]}"#;
    assert!(
        parse_digest(json, &valid_urls(&["https://a"])).is_none(),
        "a taxonomy left with zero real items must be treated as unparseable, not an empty success"
    );
}

fn sample_taxonomy() -> DigestTaxonomy {
    DigestTaxonomy {
        generated_at: chrono::Utc::now(),
        source_article_count: 3,
        topics: vec![DigestTopic {
            title: "Models".to_string(),
            subtopics: vec![
                DigestSubtopic {
                    title: "Open weights".to_string(),
                    items: vec![
                        DigestItem {
                            title: "Item A".to_string(),
                            url: "https://a".to_string(),
                            source: "HF".to_string(),
                        },
                        DigestItem {
                            title: "Item B".to_string(),
                            url: "https://b".to_string(),
                            source: "HF".to_string(),
                        },
                    ],
                },
                DigestSubtopic {
                    title: "Closed models".to_string(),
                    items: vec![DigestItem {
                        title: "Item C".to_string(),
                        url: "https://c".to_string(),
                        source: "OpenAI".to_string(),
                    }],
                },
            ],
        }],
    }
}

#[test]
fn digest_leaf_count_and_lookup_flatten_across_subtopics() {
    let t = sample_taxonomy();
    assert_eq!(digest_leaf_count(&t), 3);
    assert_eq!(digest_leaf(&t, 0).unwrap().url, "https://a");
    assert_eq!(digest_leaf(&t, 1).unwrap().url, "https://b");
    assert_eq!(digest_leaf(&t, 2).unwrap().url, "https://c");
    assert!(digest_leaf(&t, 3).is_none());
}

#[test]
fn move_digest_selection_clamps_to_leaf_bounds() {
    let widget = build_widget_for_expand_tests();
    widget.state.lock().unwrap().digest = Some(DigestState::Ready(sample_taxonomy()));

    widget.move_digest_selection(-1);
    assert_eq!(widget.state.lock().unwrap().digest_selected, 0);

    widget.move_digest_selection(5);
    assert_eq!(widget.state.lock().unwrap().digest_selected, 2);
}

/// Selecting a digest leaf and "opening" it must land on the real article
/// in its own topic tab — reusing the existing article-view state rather
/// than a parallel digest-mode viewer.
#[test]
fn open_digest_selected_jumps_to_the_real_article_in_its_own_tab() {
    let widget = build_widget_for_expand_tests();
    // build_widget_for_expand_tests seeds one "Test"-topic article at
    // https://example.com/article/1 — point a digest leaf at it.
    let taxonomy = DigestTaxonomy {
        generated_at: chrono::Utc::now(),
        source_article_count: 1,
        topics: vec![DigestTopic {
            title: "T".to_string(),
            subtopics: vec![DigestSubtopic {
                title: "S".to_string(),
                items: vec![DigestItem {
                    title: "Test Expand Article Title".to_string(),
                    url: "https://example.com/article/1".to_string(),
                    source: "Test".to_string(),
                }],
            }],
        }],
    };
    {
        let mut st = widget.state.lock().unwrap();
        st.digest = Some(DigestState::Ready(taxonomy));
        st.digest_selected = 0;
        st.active_tab = DIGEST_TAB_LABEL.to_string();
    }

    widget.open_digest_selected();

    let st = widget.state.lock().unwrap();
    assert_eq!(st.active_tab, "Test");
    assert!(st.expanded);
    assert!(st.expand_user_set);
}

#[test]
fn digest_tab_appears_right_after_all() {
    let widget = build_widget_for_expand_tests();
    let tabs = widget.tab_labels();
    assert_eq!(tabs, vec!["All", "Digest", "Test"]);
}

#[test]
fn feeds_digest_command_matches_kind_level_and_alias() {
    let mut cfg = FeedsConfig {
        commands: vec!["ai".to_string()],
        ..FeedsConfig::default()
    };
    cfg.feeds.push(FeedSpec {
        topic: "Test".to_string(),
        url: "https://example.com/feed".to_string(),
    });
    let widget = FeedsWidget::with_config(
        "ai".to_string(),
        cfg,
        std::sync::Arc::new(crate::theme::Theme::builtin_defaults()),
        crate::cache::ScopedCache::ephemeral(),
        None,
    );
    assert!(matches!(
        widget.match_command("feeds-digest"),
        Some(CommandAction::Digest)
    ));
    assert!(matches!(
        widget.match_command("ai-digest"),
        Some(CommandAction::Digest)
    ));
    assert!(widget.match_command("ai-digestx").is_none());
}

/// With no LLM configured, triggering a digest must fail loudly (a
/// `Failed` state the Digest tab's header shows) rather than silently
/// doing nothing — the mistake fixed in email's extract-todo action.
#[test]
fn trigger_digest_without_llm_sets_a_visible_failed_state() {
    let widget = build_widget_for_expand_tests();
    widget.trigger_digest();
    let st = widget.state.lock().unwrap();
    assert!(matches!(st.digest, Some(DigestState::Failed(_))));
    assert_eq!(st.active_tab, DIGEST_TAB_LABEL);
}
