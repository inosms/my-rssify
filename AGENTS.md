# AGENTS.md — rssify coding agent guide

How to add a new RSS feed. See [README.md](README.md) for the full config reference.

---

## Workflow

### 1. Discover post links

```bash
curl -s "URL" | grep -B2 -A3 "href" | head -60
```

Find the CSS selector for repeating post `<a>` links.

**JS-rendered site?** Use sitemap mode:

```bash
curl -s "https://DOMAIN/sitemap.xml" | grep '<loc>' | head -10
```

**Link aggregator (no articles to scrape)?** Omit `content_selector` — items
get title + link only, no per-article fetching.

### 2. Discover article selectors (if scraping)

```bash
curl -s "ARTICLE_URL" | grep -E 'class="[^"]*(content|body|post)[^"]*"' | head -10
```

### 3. Write the config

```toml
# Site Name  (verified YYYY-MM-DD)
[[feeds]]
name             = "Site Name"
url              = "INDEX_URL"
post_selector    = "SELECTOR"
link_selector    = "SELECTOR"          # optional: override item link
sitemap_url      = "SITEMAP_URL"       # optional: for pub dates
content_selector = "SELECTOR"          # optional: omit for link-only
title_selector   = "SELECTOR"
```

### 4. Test

```bash
cargo build --release && ./target/release/rssify -n 3 -o /tmp/rss-test
grep -c "<item>" /tmp/rss-test/*.xml
cargo test -- --test-threads=1 --nocapture
```

---

## Rules

1. **Verify selectors with curl first.** Never guess.
2. **Add `(verified YYYY-MM-DD)`** on the feed comment line.
3. **Run `cargo test`** after any change.
