# rssify

Turn any website into an RSS feed. Define feeds in TOML, run the scraper, get XML.

## Design philosophy: simplicity

Every feature must justify its complexity. The config file describes *what* to
scrape, the CLI controls *how*. There are no special cases — every feed uses the
same two mechanisms: CSS selectors and sitemaps. If `content_selector` is omitted,
the feed becomes link-only (title + URL, no article scraping).

## Quick start

```bash
cargo build --release
./target/release/rssify                           # reads feeds.toml → ./rss/
./target/release/rssify -n 5 -d 1000              # 5 items, 1s delay
./target/release/rssify --retrieve-before 2025-01-01
```

## Two feed modes

| Mode | How it's detected | What it does |
|------|-------------------|--------------|
| **HTML index** | `post_selector` present | Scrapes `<a>` links from a page |
| **Sitemap** | `sitemap_url` present, no `post_selector` | Parses sitemap.xml for URLs |

If both `post_selector` and `sitemap_url` are set, posts come from the HTML
index and the sitemap provides publication dates.

If `content_selector` is omitted, articles are not fetched — items get title +
link only. This is useful for link aggregators like Hacker News.

## Config reference

```toml
[[feeds]]
name             = "Site Name"         # required — also used for output filename
url              = "https://…"         # required — index page or channel link

# Post discovery (at least one required)
post_selector    = "div.post a"        # CSS selector for post links on the index page
link_selector    = "span.age > a"      # optional: override item link (titles still from post_selector)
sitemap_url      = "https://…/sitemap.xml"  # sitemap for post discovery or dates
sitemap_filter   = "https://…/blog/"   # only URLs matching this prefix

# Article scraping (all optional — omit for link-only feeds)
content_selector = "div.content"       # CSS selector for article body
title_selector   = "h1"               # CSS selector for article title
title_strip_suffix = " | Site Name"    # stripped from title text
```

## CLI options

```
rssify [OPTIONS]

  -c, --config <FILE>           Config file [default: feeds.toml]
  -o, --output-path <DIR>       Output directory [default: ./rss]
  -n, --top-n <N>               Max items per feed [default: 10]
  -d, --delay-ms <MS>           Delay between requests [default: 500]
      --retrieve-before <DATE>  Only items before this date (YYYY-MM-DD)
```

## GitHub Pages deployment

Feeds are rebuilt daily at 06:00 UTC and published to GitHub Pages.

**One-time setup** (in your repo's Settings → Pages):
- Source: **GitHub Actions**

After the first push to `main`, your feeds will be available at:

```
https://<user>.github.io/<repo>/zen-habits.xml
https://<user>.github.io/<repo>/mistral-ai-news.xml
https://<user>.github.io/<repo>/hacker-news-_-best.xml
```

Add these URLs to any RSS reader on your phone. The workflow also caches
previously generated RSS files so items accumulate across runs.

## Testing

```bash
cargo test -- --test-threads=1 --nocapture
```

## Troubleshooting

| Problem | Fix |
|---------|-----|
| 0 items scraped | Wrong `post_selector` — inspect HTML with curl |
| Titles are URLs | Add/fix `title_selector` |
| Empty descriptions | Wrong `content_selector` |
| JS-rendered site | Use `sitemap_url` instead of `post_selector` |
| Same pubDate on all items | Add `sitemap_url` alongside `post_selector` |
