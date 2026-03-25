//! Integration tests for rssify's feed-processing pipeline.
//!
//! These tests hit live websites and require network access.
//! Run sequentially to avoid rate-limiting servers:
//!
//!   cargo test -- --test-threads=1 --nocapture

use chrono::{DateTime, NaiveDate, Utc};
use rssify::{process_feed, FeedConfig, RunOptions};
use std::fs;
use tempfile::TempDir;

// ─── Shared helpers ───────────────────────────────────────────────────────────

fn by_link<'a>(items: &'a [rss::Item], url: &str) -> &'a rss::Item {
    items
        .iter()
        .find(|i| i.link() == Some(url))
        .unwrap_or_else(|| {
            let links: Vec<_> = items.iter().filter_map(|i| i.link()).collect();
            panic!(
                "expected item with link '{}' but found only:\n  {}",
                url,
                links.join("\n  ")
            )
        })
}

fn date(s: &str) -> DateTime<Utc> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .unwrap_or_else(|_| panic!("invalid date literal in test: {}", s))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
}

fn opts(top_n: usize, retrieve_before: Option<DateTime<Utc>>) -> RunOptions {
    RunOptions {
        retrieve_before,
        top_n,
        delay_ms: 500,
    }
}

// ─── Config helpers ───────────────────────────────────────────────────────────

fn zen_habits_cfg() -> FeedConfig {
    FeedConfig {
        name: "Zen Habits".to_string(),
        url: "https://zenhabits.net/archives/".to_string(),
        post_selector: Some("div.post-title a".to_string()),
        link_selector: None,
        sitemap_url: Some("https://zenhabits.net/sitemap.xml".to_string()),
        sitemap_filter: None,
        content_selector: Some("div.post-content".to_string()),
        title_selector: Some("article.post h2".to_string()),
        title_strip_suffix: None,
    }
}

fn mistral_cfg() -> FeedConfig {
    FeedConfig {
        name: "Mistral AI News".to_string(),
        url: "https://mistral.ai/news".to_string(),
        post_selector: None,
        link_selector: None,
        sitemap_url: Some("https://mistral.ai/sitemap.xml".to_string()),
        sitemap_filter: Some("https://mistral.ai/news/".to_string()),
        content_selector: Some("div.blog-rich-text".to_string()),
        title_selector: Some("title".to_string()),
        title_strip_suffix: Some(" | Mistral AI".to_string()),
    }
}

fn hn_cfg() -> FeedConfig {
    FeedConfig {
        name: "Hacker News – Best".to_string(),
        url: "https://news.ycombinator.com/best".to_string(),
        post_selector: Some("span.titleline > a".to_string()),
        link_selector: Some("span.age > a".to_string()),
        sitemap_url: None,
        sitemap_filter: None,
        content_selector: None,
        title_selector: None,
        title_strip_suffix: None,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Zen Habits (index-page mode): --retrieve-before drops posts on-or-after
/// the threshold and only keeps older ones.
///
/// Posts known to exist before 2026-03-10 (stable):
///   zen-writing (Mar 9), become-want (Mar 6), self-doubt (Mar 2)
#[test]
fn zen_habits_retrieve_before_filters_by_date() {
    let out_dir = TempDir::new().unwrap();
    let cfg = zen_habits_cfg();
    let o = opts(3, Some(date("2026-03-10")));

    process_feed(&cfg, out_dir.path(), &o).unwrap();

    let xml = fs::read_to_string(out_dir.path().join("zen-habits.xml")).unwrap();
    let channel: rss::Channel = xml.parse().unwrap();
    let threshold = date("2026-03-10");

    assert!(
        !channel.items().is_empty(),
        "Expected at least one item published before 2026-03-10, got zero"
    );
    for item in channel.items() {
        let raw = item.pub_date().expect("every item must carry a <pubDate>");
        let dt = DateTime::parse_from_rfc2822(raw)
            .unwrap_or_else(|_| panic!("unparseable <pubDate>: {}", raw))
            .with_timezone(&Utc);
        assert!(
            dt < threshold,
            "Item '{}' has pubDate '{}' which is NOT before the threshold {}",
            item.title().unwrap_or("(no title)"),
            raw,
            threshold.format("%Y-%m-%d"),
        );
    }

    let items = channel.items();

    let zen_writing = by_link(items, "https://zenhabits.net/zen-writing/");
    assert_eq!(
        zen_writing.title().unwrap_or(""),
        "Writing a Book in Public",
    );
    assert_eq!(
        zen_writing.pub_date().unwrap_or(""),
        "Mon, 9 Mar 2026 00:00:00 +0000",
    );
    let zen_writing_desc = zen_writing.description().unwrap_or("");
    assert!(
        zen_writing_desc.contains("Zen of Doing"),
        "zen-writing description should mention 'Zen of Doing'; got: {}",
        &zen_writing_desc[..zen_writing_desc.len().min(200)]
    );

    let self_doubt = by_link(items, "https://zenhabits.net/self-doubt/");
    assert_eq!(
        self_doubt.title().unwrap_or(""),
        "How We Trip Ourselves Up with Self-Doubt",
    );
    assert_eq!(
        self_doubt.pub_date().unwrap_or(""),
        "Mon, 2 Mar 2026 00:00:00 +0000",
    );
    let self_doubt_desc = self_doubt.description().unwrap_or("");
    assert!(
        self_doubt_desc.contains("doubting"),
        "self-doubt description should contain 'doubting'; got: {}",
        &self_doubt_desc[..self_doubt_desc.len().min(200)]
    );
}

/// Mistral AI News (sitemap mode): --retrieve-before drops posts on-or-after
/// the threshold.
///
/// Posts known to exist before 2026-03-10 (stable):
///   pixtral-12b (Mar 2), pixtral-large (Mar 1)
#[test]
fn mistral_retrieve_before_filters_by_date() {
    let out_dir = TempDir::new().unwrap();
    let cfg = mistral_cfg();
    let o = opts(3, Some(date("2026-03-10")));

    process_feed(&cfg, out_dir.path(), &o).unwrap();

    let xml = fs::read_to_string(out_dir.path().join("mistral-ai-news.xml")).unwrap();
    let channel: rss::Channel = xml.parse().unwrap();
    let threshold = date("2026-03-10");

    assert!(
        !channel.items().is_empty(),
        "Expected at least one Mistral item published before 2026-03-10, got zero"
    );
    for item in channel.items() {
        let raw = item.pub_date().expect("every item must carry a <pubDate>");
        let dt = DateTime::parse_from_rfc2822(raw)
            .unwrap_or_else(|_| panic!("unparseable <pubDate>: {}", raw))
            .with_timezone(&Utc);
        assert!(
            dt < threshold,
            "Item '{}' has pubDate '{}' which is NOT before the threshold {}",
            item.title().unwrap_or("(no title)"),
            raw,
            threshold.format("%Y-%m-%d"),
        );
    }

    let items = channel.items();

    let pixtral_12b = by_link(items, "https://mistral.ai/news/pixtral-12b");
    assert_eq!(
        pixtral_12b.title().unwrap_or(""),
        "[Deprecated] Pixtral 12B",
    );
    assert_eq!(
        pixtral_12b.pub_date().unwrap_or(""),
        "Mon, 2 Mar 2026 10:38:40 +0000",
    );
    let pixtral_12b_desc = pixtral_12b.description().unwrap_or("");
    assert!(
        pixtral_12b_desc.contains("multimodal"),
        "pixtral-12b description should contain 'multimodal'; got: {}",
        &pixtral_12b_desc[..pixtral_12b_desc.len().min(300)]
    );

    let pixtral_large = by_link(items, "https://mistral.ai/news/pixtral-large");
    assert_eq!(
        pixtral_large.title().unwrap_or(""),
        "[Deprecated] Pixtral Large",
    );
    assert_eq!(
        pixtral_large.pub_date().unwrap_or(""),
        "Sun, 1 Mar 2026 08:46:58 +0000",
    );
    let pixtral_large_desc = pixtral_large.description().unwrap_or("");
    assert!(
        pixtral_large_desc.contains("multimodal"),
        "pixtral-large description should contain 'multimodal'; got: {}",
        &pixtral_large_desc[..pixtral_large_desc.len().min(300)]
    );
}

/// Hacker News – Best (link-only mode, no content_selector):
/// Scrapes news.ycombinator.com/best for top stories of the past few days.
///
/// Structural checks only (titles change daily):
///   - items returned
///   - each has a non-empty title
///   - each links to an http(s) URL
#[test]
fn hn_returns_stories_as_link_only_items() {
    let out_dir = TempDir::new().unwrap();
    let cfg = hn_cfg();
    let o = opts(5, None);

    process_feed(&cfg, out_dir.path(), &o).unwrap();

    let xml = fs::read_to_string(out_dir.path().join("hacker-news-_-best.xml")).unwrap();
    let channel: rss::Channel = xml.parse().unwrap();

    assert_eq!(
        channel.items().len(),
        5,
        "Expected 5 items, got {}",
        channel.items().len()
    );

    for item in channel.items() {
        let title = item.title().unwrap_or("");
        assert!(
            !title.is_empty(),
            "Item title should not be empty"
        );

        let link = item.link().expect("item must have a <link>");
        assert!(
            link.starts_with("https://news.ycombinator.com/item?id="),
            "<link> should point to HN discussion page, got: {}",
            link
        );
    }
}
