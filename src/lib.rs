use chrono::{DateTime, NaiveDate, Utc};
use clap::Parser;
use scraper::{Html, Selector};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use url::Url;

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "rssify", version = "0.1.0")]
#[command(about = "Scrape webpages and produce RSS feeds from a TOML config")]
pub struct Cli {
    /// Path to the TOML configuration file
    #[arg(short, long, default_value = "feeds.toml")]
    pub config: PathBuf,

    /// Directory where RSS XML files will be written
    #[arg(short, long, default_value = "./rss")]
    pub output_path: PathBuf,

    /// Only include items published strictly before this date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    pub retrieve_before: Option<String>,

    /// Maximum number of items per feed
    #[arg(short = 'n', long, default_value = "5")]
    pub top_n: usize,

    /// Delay in milliseconds between HTTP requests
    #[arg(short, long, default_value = "500")]
    pub delay_ms: u64,
}

// ─── Config ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
pub struct Config {
    pub feeds: Vec<FeedConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct FeedConfig {
    pub name: String,
    pub url: String,

    // Post discovery (one of):
    //   post_selector → scrape an HTML index page for <a> links
    //   sitemap_url   → parse a sitemap.xml for URLs
    // If both are set, posts come from the index and the sitemap provides dates.
    pub post_selector: Option<String>,
    pub sitemap_url: Option<String>,
    pub sitemap_filter: Option<String>,

    // Article scraping (all optional):
    //   If content_selector is absent, items get title + link only (no scraping).
    pub content_selector: Option<String>,
    pub title_selector: Option<String>,
    pub title_strip_suffix: Option<String>,
}

/// Runtime options passed into process_feed (from CLI or tests).
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub retrieve_before: Option<DateTime<Utc>>,
    pub top_n: usize,
    pub delay_ms: u64,
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let config_str = fs::read_to_string(&cli.config)
        .map_err(|e| format!("Cannot read '{}': {}", cli.config.display(), e))?;
    let config: Config =
        toml::from_str(&config_str).map_err(|e| format!("Invalid TOML: {}", e))?;

    let opts = RunOptions {
        retrieve_before: cli
            .retrieve_before
            .as_deref()
            .map(parse_date)
            .transpose()?,
        top_n: cli.top_n,
        delay_ms: cli.delay_ms,
    };

    fs::create_dir_all(&cli.output_path)?;
    println!("Output directory : {}", cli.output_path.display());
    println!("Feeds to process : {}", config.feeds.len());

    for feed in &config.feeds {
        println!("\n▶  {}", feed.name);
        if let Err(e) = process_feed(feed, &cli.output_path, &opts) {
            eprintln!("   ✗  Error: {}", e);
        }
    }
    println!("\n✓  All done.");
    Ok(())
}

fn parse_date(s: &str) -> Result<DateTime<Utc>, Box<dyn std::error::Error>> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}

// ─── Core feed processor ────────────────────────────────────────────────────

pub fn process_feed(
    cfg: &FeedConfig,
    out_dir: &Path,
    opts: &RunOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let filename = format!("{}.xml", sanitize_filename(&cfg.name));
    let out_path = out_dir.join(&filename);

    let existing_guids = load_existing_guids(&out_path);
    let mut kept_items = load_existing_items(&out_path);
    println!(
        "   {} existing item(s) loaded from {}",
        kept_items.len(),
        out_path.file_name().unwrap_or_default().to_string_lossy()
    );

    // ── Discover posts ───────────────────────────────────────────────────────
    let base_url = Url::parse(&cfg.url)?;
    let mut sitemap_xml_cache: Option<String> = None;
    let has_post_selector = cfg.post_selector.is_some();

    let mut posts: Vec<(String, String)> = if has_post_selector {
        let selector = cfg.post_selector.as_deref().unwrap();
        println!("   Fetching index : {}", cfg.url);
        let html = fetch_url(&cfg.url)?;
        let doc = Html::parse_document(&html);
        let p = extract_posts(&doc, &base_url, selector);
        println!("   Found {} post link(s) on index", p.len());
        p
    } else if let Some(sitemap_url) = &cfg.sitemap_url {
        println!("   Fetching sitemap: {}", sitemap_url);
        let xml = fetch_url(sitemap_url)?;
        let p = parse_sitemap_posts(&xml, cfg.sitemap_filter.as_deref());
        println!("   Found {} post URL(s) in sitemap", p.len());
        sitemap_xml_cache = Some(xml);
        p
    } else {
        return Err(format!(
            "Feed '{}': set post_selector or sitemap_url",
            cfg.name
        )
        .into());
    };

    // ── Build pub-date map from sitemap ──────────────────────────────────────
    let pub_date_map: HashMap<String, String> = {
        let xml_opt = if has_post_selector {
            if let Some(sm_url) = &cfg.sitemap_url {
                println!("   Fetching sitemap for dates : {}", sm_url);
                fetch_url(sm_url).ok()
            } else {
                None
            }
        } else {
            sitemap_xml_cache
        };
        xml_opt
            .map(|xml| build_date_map(&xml, cfg.sitemap_filter.as_deref()))
            .unwrap_or_default()
    };

    // ── Filter by date ───────────────────────────────────────────────────────
    if let Some(threshold) = opts.retrieve_before {
        let before = posts.len();
        posts.retain(|(url, _)| {
            pub_date_map.get(url.as_str()).map_or(true, |d| {
                DateTime::parse_from_rfc2822(d)
                    .map(|dt| dt.with_timezone(&Utc) < threshold)
                    .unwrap_or(true)
            })
        });
        println!(
            "   {} → {} post(s) after date filter (before {})",
            before,
            posts.len(),
            threshold.format("%Y-%m-%d")
        );
    }

    posts.truncate(opts.top_n);
    println!("   (limited to top {})", opts.top_n);

    // ── Scrape articles (if content_selector is set) ─────────────────────────
    let content_sel = cfg
        .content_selector
        .as_deref()
        .map(|s| Selector::parse(s).map_err(|e| format!("Bad content_selector: {}", e)))
        .transpose()?;
    let title_sel = cfg
        .title_selector
        .as_deref()
        .map(|s| Selector::parse(s).map_err(|e| format!("Bad title_selector: {}", e)))
        .transpose()?;

    let run_ts = Utc::now().to_rfc2822();
    let mut new_items: Vec<rss::Item> = Vec::new();
    let mut first = true;

    for (url, idx_title) in &posts {
        if existing_guids.contains(url.as_str()) {
            println!("   ↩  skip (already seen): {}", url);
            continue;
        }

        let pub_date = pub_date_map
            .get(url.as_str())
            .cloned()
            .unwrap_or_else(|| run_ts.clone());

        // If no content_selector, emit link-only item (no article fetch)
        let Some(ref c_sel) = content_sel else {
            println!("   ✦  {}", idx_title);
            new_items.push(build_item(url, idx_title, "", &pub_date));
            continue;
        };

        if !first {
            thread::sleep(Duration::from_millis(opts.delay_ms));
        }
        first = false;
        println!("   ↓  {}", url);

        let (title, content) = match fetch_article(
            url,
            c_sel,
            title_sel.as_ref(),
            cfg.title_strip_suffix.as_deref(),
        ) {
            Ok((t, c)) => (if t.is_empty() { idx_title.clone() } else { t }, c),
            Err(e) => {
                eprintln!("      ⚠  {}", e);
                (idx_title.clone(), String::new())
            }
        };

        new_items.push(build_item(url, &title, &content, &pub_date));
    }

    write_channel(cfg, &out_path, new_items, &mut kept_items)
}

// ─── RSS output ─────────────────────────────────────────────────────────────

fn write_channel(
    cfg: &FeedConfig,
    out_path: &Path,
    mut new_items: Vec<rss::Item>,
    kept_items: &mut Vec<rss::Item>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "   {} new item(s), {} kept from previous run(s)",
        new_items.len(),
        kept_items.len()
    );
    new_items.append(kept_items);

    let channel = rss::ChannelBuilder::default()
        .title(&cfg.name)
        .link(&cfg.url)
        .description(format!("RSS feed for {} — generated by rssify", cfg.name))
        .last_build_date(Some(Utc::now().to_rfc2822()))
        .items(new_items)
        .build();

    fs::write(out_path, channel.to_string())?;
    println!("   ✓  Saved → {}", out_path.display());
    Ok(())
}

fn build_item(url: &str, title: &str, description: &str, pub_date: &str) -> rss::Item {
    let guid = rss::GuidBuilder::default()
        .value(url)
        .permalink(true)
        .build();
    rss::ItemBuilder::default()
        .title(Some(title.to_string()))
        .link(Some(url.to_string()))
        .description(Some(description.to_string()))
        .pub_date(Some(pub_date.to_string()))
        .guid(Some(guid))
        .build()
}

// ─── HTML scraping ──────────────────────────────────────────────────────────

fn extract_posts(doc: &Html, base_url: &Url, post_selector: &str) -> Vec<(String, String)> {
    let sel = match Selector::parse(post_selector) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let a_sel = Selector::parse("a").unwrap();
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for el in doc.select(&sel) {
        let (href, title) = if el.value().name() == "a" {
            (
                el.value().attr("href").unwrap_or(""),
                el.text().collect::<String>(),
            )
        } else if let Some(a) = el.select(&a_sel).next() {
            (
                a.value().attr("href").unwrap_or(""),
                a.text().collect::<String>(),
            )
        } else {
            continue;
        };
        if href.is_empty() || href.starts_with('#') {
            continue;
        }
        let abs = base_url
            .join(href)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| href.to_string());
        if !seen.insert(abs.clone()) {
            continue;
        }
        let title = title.trim().to_string();
        let title = if title.is_empty() {
            abs.clone()
        } else {
            title
        };
        out.push((abs, title));
    }
    out
}

fn fetch_url(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(ureq::get(url)
        .set("User-Agent", "Mozilla/5.0 (compatible; rssify/0.1)")
        .call()?
        .into_string()?)
}

fn fetch_article(
    url: &str,
    content_sel: &Selector,
    title_sel: Option<&Selector>,
    strip_suffix: Option<&str>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let html = fetch_url(url)?;
    let doc = Html::parse_document(&html);

    let raw_title = title_sel
        .and_then(|s| doc.select(s).next())
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();
    let title = strip_suffix
        .and_then(|suf| raw_title.strip_suffix(suf))
        .unwrap_or(&raw_title)
        .trim()
        .to_string();

    let content = doc
        .select(content_sel)
        .next()
        .map(|e| e.inner_html())
        .unwrap_or_default();
    Ok((title, content))
}

// ─── Sitemap helpers ────────────────────────────────────────────────────────

fn parse_sitemap_posts(xml: &str, filter: Option<&str>) -> Vec<(String, String)> {
    let entries = parse_sitemap_entries(xml, filter);
    let mut sorted: Vec<_> = entries.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted
        .into_iter()
        .map(|(url, _)| (url, String::new()))
        .collect()
}

fn build_date_map(xml: &str, filter: Option<&str>) -> HashMap<String, String> {
    parse_sitemap_entries(xml, filter)
        .into_iter()
        .filter_map(|(url, lastmod)| {
            if lastmod.is_empty() {
                return None;
            }
            DateTime::parse_from_rfc3339(&lastmod)
                .map(|dt| (url, dt.to_rfc2822()))
                .ok()
        })
        .collect()
}

fn parse_sitemap_entries(xml: &str, filter: Option<&str>) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut loc: Option<String> = None;
    let mut lastmod: Option<String> = None;

    for line in xml.lines() {
        let line = line.trim();
        if let Some(v) = xml_text(line, "loc") {
            loc = Some(v.to_string());
        } else if let Some(v) = xml_text(line, "lastmod") {
            lastmod = Some(v.to_string());
        } else if line.starts_with("</url>") {
            if let Some(url) = loc.take() {
                if filter.map_or(true, |f| url.starts_with(f)) {
                    entries.push((url, lastmod.take().unwrap_or_default()));
                }
            }
            lastmod = None;
        }
    }
    entries
}

fn xml_text<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = line.find(&open)? + open.len();
    let end = line.find(&close)?;
    if start <= end {
        Some(&line[start..end])
    } else {
        None
    }
}

// ─── Deduplication ──────────────────────────────────────────────────────────

fn load_existing_guids(path: &Path) -> HashSet<String> {
    load_existing_items(path)
        .iter()
        .filter_map(|i| i.guid().map(|g| g.value().to_string()))
        .collect()
}

fn load_existing_items(path: &Path) -> Vec<rss::Item> {
    if !path.exists() {
        return Vec::new();
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse::<rss::Channel>().ok())
        .map(|c| c.into_items())
        .unwrap_or_default()
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            ' ' => '-',
            _ => '_',
        })
        .collect::<String>()
        .to_lowercase()
}
