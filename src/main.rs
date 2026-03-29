mod tui;

use chrono::Local;
use clap::{Parser, Subcommand};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::PathBuf;

#[derive(Debug)]
pub enum BmError {
    NoConfig(PathBuf),
    InvalidConfig(PathBuf, String),
    FileNotFound(PathBuf),
    IoError(String),
    IndexOutOfRange(usize, usize),
    BookmarkNotFound(String),
}

impl fmt::Display for BmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BmError::NoConfig(p) => write!(
                f,
                "No config found. Create {} with:\n\n  bookmarks_file = \"/path/to/bookmarks.csv\"\n",
                p.display()
            ),
            BmError::InvalidConfig(p, e) => write!(f, "Invalid config {}: {}", p.display(), e),
            BmError::FileNotFound(p) => write!(f, "Bookmarks file not found: {}", p.display()),
            BmError::IoError(e) => write!(f, "{}", e),
            BmError::IndexOutOfRange(idx, max) => {
                write!(f, "Index {} out of range (1-{}).", idx, max)
            }
            BmError::BookmarkNotFound(t) => write!(f, "Bookmark not found: {}", t),
        }
    }
}

#[derive(Deserialize)]
struct Config {
    bookmarks_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bookmark {
    pub date: String,
    pub url: String,
    pub title: String,
    pub description: String,
}

#[derive(Parser)]
#[command(name = "bm", about = "CLI bookmark manager backed by a CSV file")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// URL to add (shorthand for `bm add <url>`)
    url: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a URL to bookmarks
    Add {
        /// URL to bookmark
        url: String,
    },
    /// List all bookmarks
    List,
    /// Search bookmarks by keyword
    Search {
        /// Search query (case-insensitive substring match)
        query: String,
    },
    /// Remove a bookmark by index (1-based) or URL
    Remove {
        /// 1-based index or URL to remove
        target: String,
    },
    /// Open interactive TUI browser
    Tui,
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("bm")
        .join("config.toml")
}

fn load_config_from(path: &PathBuf) -> Result<Config, BmError> {
    if path.exists() {
        let content = fs::read_to_string(path)
            .map_err(|e| BmError::IoError(format!("Failed to read {}: {}", path.display(), e)))?;
        toml::from_str(&content)
            .map_err(|e| BmError::InvalidConfig(path.clone(), e.to_string()))
    } else {
        Err(BmError::NoConfig(path.clone()))
    }
}

fn resolve_path(p: &str) -> PathBuf {
    if p.starts_with('~')
        && let Some(home) = dirs::home_dir()
    {
        return home.join(&p[2..]);
    }
    PathBuf::from(p)
}

fn csv_path_from(configured: &std::path::Path) -> PathBuf {
    configured.with_extension("csv")
}

// --- CSV read/write ---

pub fn read_bookmarks(path: &PathBuf) -> Vec<Bookmark> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    if content.trim().is_empty() {
        return Vec::new();
    }
    let mut rdr = csv::Reader::from_reader(content.as_bytes());
    rdr.deserialize()
        .filter_map(|r| r.ok())
        .collect()
}

pub fn write_bookmarks(bookmarks: &[Bookmark], path: &PathBuf) -> Result<(), BmError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    for bm in bookmarks {
        wtr.serialize(bm)
            .map_err(|e| BmError::IoError(format!("CSV write error: {}", e)))?;
    }
    let data = wtr
        .into_inner()
        .map_err(|e| BmError::IoError(format!("CSV flush error: {}", e)))?;
    fs::write(path, data)
        .map_err(|e| BmError::IoError(format!("Failed to write {}: {}", path.display(), e)))
}

fn ensure_csv_file(path: &PathBuf) -> Result<(), BmError> {
    if !path.exists() {
        fs::write(path, "date,url,title,description\n")
            .map_err(|e| BmError::IoError(format!("Failed to create {}: {}", path.display(), e)))?;
    }
    Ok(())
}

// --- Migration from .md to .csv ---

fn migrate_md_to_csv(md_path: &PathBuf, csv_path: &PathBuf) -> Result<(), BmError> {
    let content = fs::read_to_string(md_path)
        .map_err(|e| BmError::IoError(format!("Failed to read {}: {}", md_path.display(), e)))?;

    let today = Local::now().format("%Y-%m-%d").to_string();
    let mut bookmarks = Vec::new();

    for line in content.lines() {
        if !line.starts_with("- ") {
            continue;
        }
        let stripped = line.strip_prefix("- ").unwrap_or(line);
        let url = stripped.split_whitespace().next().unwrap_or("").to_string();
        if url.is_empty() {
            continue;
        }
        let label = stripped
            .strip_prefix(&url)
            .and_then(|s| s.strip_prefix(" - "))
            .unwrap_or("")
            .to_string();

        bookmarks.push(Bookmark {
            date: today.clone(),
            url,
            title: label,
            description: String::new(),
        });
    }

    write_bookmarks(&bookmarks, csv_path)?;
    eprintln!(
        "Migrated {} bookmarks from {} to {}",
        bookmarks.len(),
        md_path.display(),
        csv_path.display()
    );
    Ok(())
}

// --- HTML metadata extraction (unchanged) ---

fn extract_title_from_html(html: &str) -> Option<String> {
    let re = Regex::new(r"(?i)<title[^>]*>([\s\S]*?)</title>").ok()?;
    let caps = re.captures(html)?;
    let title = caps.get(1)?.as_str().trim().to_string();
    let title = decode_html_entities(&title);
    if title.is_empty() { None } else { Some(title) }
}

fn extract_description_from_html(html: &str) -> Option<String> {
    let og_re = Regex::new(r#"(?i)<meta\s[^>]*property\s*=\s*"og:description"[^>]*content\s*=\s*"([^"]*)"[^>]*/?\s*>"#).ok()?;
    if let Some(caps) = og_re.captures(html) {
        let desc = decode_html_entities(caps.get(1)?.as_str().trim());
        if !desc.is_empty() {
            return Some(desc);
        }
    }
    let og_re2 = Regex::new(r#"(?i)<meta\s[^>]*content\s*=\s*"([^"]*)"[^>]*property\s*=\s*"og:description"[^>]*/?\s*>"#).ok()?;
    if let Some(caps) = og_re2.captures(html) {
        let desc = decode_html_entities(caps.get(1)?.as_str().trim());
        if !desc.is_empty() {
            return Some(desc);
        }
    }
    let meta_re = Regex::new(r#"(?i)<meta\s[^>]*name\s*=\s*"description"[^>]*content\s*=\s*"([^"]*)"[^>]*/?\s*>"#).ok()?;
    if let Some(caps) = meta_re.captures(html) {
        let desc = decode_html_entities(caps.get(1)?.as_str().trim());
        if !desc.is_empty() {
            return Some(desc);
        }
    }
    let meta_re2 = Regex::new(r#"(?i)<meta\s[^>]*content\s*=\s*"([^"]*)"[^>]*name\s*=\s*"description"[^>]*/?\s*>"#).ok()?;
    if let Some(caps) = meta_re2.captures(html) {
        let desc = decode_html_entities(caps.get(1)?.as_str().trim());
        if !desc.is_empty() {
            return Some(desc);
        }
    }
    None
}

pub struct PageMeta {
    pub title: Option<String>,
    pub description: Option<String>,
}

fn extract_metadata_from_html(html: &str) -> PageMeta {
    PageMeta {
        title: extract_title_from_html(html),
        description: extract_description_from_html(html),
    }
}

fn uppercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&ndash;", "\u{2013}")
        .replace("&mdash;", "\u{2014}")
}

// --- URL classification helpers ---

fn is_private_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let host_start = if let Some(pos) = lower.find("://") {
        pos + 3
    } else {
        return true;
    };
    let host_part = &lower[host_start..];
    let host_and_port = host_part.split('/').next().unwrap_or("");
    let host = if host_and_port.starts_with('[') {
        host_and_port.split(']').next().map(|s| &s[1..]).unwrap_or(host_and_port)
    } else {
        host_and_port.split(':').next().unwrap_or(host_and_port)
    };

    host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host == "0.0.0.0"
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("172.16.")
        || host.starts_with("172.17.")
        || host.starts_with("172.18.")
        || host.starts_with("172.19.")
        || host.starts_with("172.20.")
        || host.starts_with("172.21.")
        || host.starts_with("172.22.")
        || host.starts_with("172.23.")
        || host.starts_with("172.24.")
        || host.starts_with("172.25.")
        || host.starts_with("172.26.")
        || host.starts_with("172.27.")
        || host.starts_with("172.28.")
        || host.starts_with("172.29.")
        || host.starts_with("172.30.")
        || host.starts_with("172.31.")
        || lower.starts_with("file://")
}

fn is_x_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.starts_with("https://x.com/")
        || lower.starts_with("http://x.com/")
        || lower.starts_with("https://twitter.com/")
        || lower.starts_with("http://twitter.com/")
        || lower.starts_with("https://www.x.com/")
        || lower.starts_with("https://www.twitter.com/")
}

fn extract_first_paragraph(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("[![")
            || trimmed.starts_with('#')
            || trimmed.starts_with('[')
            || trimmed.starts_with("![")
            || trimmed.len() <= 20
        {
            continue;
        }
        let truncated = if trimmed.len() > 120 {
            let boundary = trimmed
                .char_indices()
                .take_while(|(i, _)| *i < 120)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(120);
            format!("{}...", &trimmed[..boundary])
        } else {
            trimmed.to_string()
        };
        return Some(truncated);
    }
    None
}

fn clean_x_title(title: &str) -> Option<String> {
    let t = title.trim();
    if t.is_empty() || t == "X" || t == "Twitter" {
        return None;
    }
    let t = t
        .strip_suffix(" / X")
        .or_else(|| t.strip_suffix(" / Twitter"))
        .unwrap_or(t);
    if t.is_empty() { None } else { Some(t.to_string()) }
}

// --- Metadata fetching ---

fn fetch_metadata_via_jina(url: &str) -> Option<PageMeta> {
    if is_private_url(url) {
        return None;
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;

    let mut req = client
        .post("https://r.jina.ai/")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("X-Engine", "browser")
        .header("X-Timeout", "20")
        .header("X-Retain-Images", "none");

    if is_x_url(url) {
        req = req.header("X-Wait-For-Selector", "[data-testid='tweetText']");
    }

    let body = serde_json::json!({"url": url});
    let resp = req.json(&body).send().ok()?;
    let json: serde_json::Value = resp.json().ok()?;
    let data = json.get("data")?;

    let raw_title = data.get("title").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("");
    let raw_desc = data.get("description").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("");
    let content = data.get("content").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("");

    let title = if is_x_url(url) {
        clean_x_title(raw_title)
    } else {
        let t = raw_title.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    };

    let desc = if raw_desc.trim().is_empty() {
        extract_first_paragraph(content)
    } else {
        Some(raw_desc.trim().to_string())
    };

    if title.is_some() || desc.is_some() {
        Some(PageMeta { title, description: desc })
    } else {
        None
    }
}

fn fetch_metadata(url: &str) -> PageMeta {
    if is_x_url(url) {
        return fetch_metadata_via_jina(url).unwrap_or(PageMeta {
            title: None,
            description: None,
        });
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build();

    if let Ok(client) = client
        && let Ok(resp) = client
            .get(url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) bm/0.1",
            )
            .send()
        && let Ok(body) = resp.text()
    {
        let meta = extract_metadata_from_html(&body);
        if meta.title.is_some() || meta.description.is_some() {
            return meta;
        }
    }

    fetch_metadata_via_jina(url).unwrap_or(PageMeta {
        title: None,
        description: None,
    })
}

// --- Bookmark commands ---

fn is_duplicate(url: &str, bookmarks: &[Bookmark]) -> bool {
    bookmarks.iter().any(|b| b.url == url)
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn cmd_add(url: &str, bm_path: &PathBuf) -> Result<String, BmError> {
    let mut bookmarks = read_bookmarks(bm_path);
    if is_duplicate(url, &bookmarks) {
        return Ok(format!("Already bookmarked: {}", url));
    }

    print!("Fetching metadata... ");
    let meta = fetch_metadata(url);
    let title = meta.title.unwrap_or_default();
    let description = meta.description.unwrap_or_default();

    if !title.is_empty() || !description.is_empty() {
        let display = format_display_label(&title, &description);
        println!("{}", display);
    } else {
        println!("(not found)");
    }

    let bm = Bookmark {
        date: today(),
        url: url.to_string(),
        title,
        description,
    };

    let msg = format!("Bookmarked: {}", bm.url);
    bookmarks.push(bm);
    write_bookmarks(&bookmarks, bm_path)?;
    Ok(msg)
}

pub fn add_entry(url: &str, title: Option<String>, description: Option<String>, bm_path: &PathBuf) -> Result<String, BmError> {
    let mut bookmarks = read_bookmarks(bm_path);
    let bm = Bookmark {
        date: today(),
        url: url.to_string(),
        title: title.unwrap_or_default(),
        description: description.unwrap_or_default(),
    };
    let msg = format!("Bookmarked: {}", bm.url);
    bookmarks.push(bm);
    write_bookmarks(&bookmarks, bm_path)?;
    Ok(msg)
}

fn format_display_label(title: &str, description: &str) -> String {
    match (title.is_empty(), description.is_empty()) {
        (false, false) => {
            let t_lower = title.to_lowercase();
            let d_lower = description.to_lowercase();
            if t_lower == d_lower {
                title.to_string()
            } else if d_lower.starts_with(&format!("{} - ", t_lower))
                || d_lower.starts_with(&format!("{}: ", t_lower))
                || d_lower.starts_with(&format!("{} | ", t_lower))
            {
                description.to_string()
            } else if d_lower.starts_with(&format!("{} is ", t_lower))
                || d_lower.starts_with(&format!("{} — ", t_lower))
            {
                let rest = &description[title.len()..].trim_start();
                let rest = rest.strip_prefix("is ").or_else(|| rest.strip_prefix("— ")).unwrap_or(rest);
                let rest = uppercase_first(rest);
                format!("{}: {}", title, rest)
            } else {
                format!("{}: {}", title, description)
            }
        }
        (false, true) => title.to_string(),
        (true, false) => description.to_string(),
        (true, true) => String::new(),
    }
}

fn cmd_list(bm_path: &PathBuf) -> String {
    let bookmarks = read_bookmarks(bm_path);
    if bookmarks.is_empty() {
        return "No bookmarks found.".to_string();
    }
    let mut output = String::new();
    for (idx, bm) in bookmarks.iter().enumerate() {
        let label = format_display_label(&bm.title, &bm.description);
        if label.is_empty() {
            output.push_str(&format!("  {}. [{}] {}\n", idx + 1, bm.date, bm.url));
        } else {
            output.push_str(&format!("  {}. [{}] {} - {}\n", idx + 1, bm.date, bm.url, label));
        }
    }
    output.push_str(&format!("\n{} bookmark(s)", bookmarks.len()));
    output
}

fn cmd_search(query: &str, bm_path: &PathBuf) -> String {
    let bookmarks = read_bookmarks(bm_path);
    let q = query.to_lowercase();
    let matches: Vec<&Bookmark> = bookmarks
        .iter()
        .filter(|b| {
            b.url.to_lowercase().contains(&q)
                || b.title.to_lowercase().contains(&q)
                || b.description.to_lowercase().contains(&q)
        })
        .collect();

    if matches.is_empty() {
        return format!("No bookmarks matching \"{}\".", query);
    }
    let mut output = String::new();
    for bm in &matches {
        let label = format_display_label(&bm.title, &bm.description);
        if label.is_empty() {
            output.push_str(&format!("  [{}] {}\n", bm.date, bm.url));
        } else {
            output.push_str(&format!("  [{}] {} - {}\n", bm.date, bm.url, label));
        }
    }
    output.push_str(&format!("\n{} result(s)", matches.len()));
    output
}

pub fn cmd_remove(target: &str, bm_path: &PathBuf) -> Result<String, BmError> {
    let mut bookmarks = read_bookmarks(bm_path);
    if bookmarks.is_empty() {
        return Err(BmError::BookmarkNotFound(target.to_string()));
    }

    let remove_idx = if let Ok(idx) = target.parse::<usize>() {
        if idx == 0 || idx > bookmarks.len() {
            return Err(BmError::IndexOutOfRange(idx, bookmarks.len()));
        }
        idx - 1
    } else {
        match bookmarks.iter().position(|b| b.url == target) {
            Some(i) => i,
            None => return Err(BmError::BookmarkNotFound(target.to_string())),
        }
    };

    let removed = bookmarks.remove(remove_idx);
    let msg = format!("Removed: {}", removed.url);
    write_bookmarks(&bookmarks, bm_path)?;
    Ok(msg)
}

fn run() -> Result<(), BmError> {
    let cli = Cli::parse();
    let config = load_config_from(&config_path())?;
    let configured_path = resolve_path(&config.bookmarks_file);
    let bm_path = csv_path_from(&configured_path);

    // Auto-migrate from .md if needed
    if !bm_path.exists() && configured_path.exists() && configured_path.extension().is_some_and(|e| e == "md") {
        migrate_md_to_csv(&configured_path, &bm_path)?;
    }

    if !bm_path.exists() {
        ensure_csv_file(&bm_path)?;
    }

    match cli.command {
        Some(Commands::Add { url }) => println!("{}", cmd_add(&url, &bm_path)?),
        Some(Commands::List) => println!("{}", cmd_list(&bm_path)),
        Some(Commands::Search { query }) => println!("{}", cmd_search(&query, &bm_path)),
        Some(Commands::Remove { target }) => println!("{}", cmd_remove(&target, &bm_path)?),
        Some(Commands::Tui) => tui::run_tui(bm_path)?,
        None => {
            if let Some(url) = cli.url {
                println!("{}", cmd_add(&url, &bm_path)?);
            } else {
                tui::run_tui(bm_path)?;
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_temp_csv(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bookmarks.csv");
        fs::write(&path, content).unwrap();
        (dir, path)
    }

    const SAMPLE_CSV: &str = "date,url,title,description\n\
        2026-03-01,https://example.com,Example Domain,\n\
        2026-03-15,https://rust-lang.org,Rust Programming Language,A systems programming language\n\
        2026-03-29,https://github.com,GitHub,Where people build software\n";

    // -- read/write bookmarks --

    #[test]
    fn test_read_bookmarks_csv() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let bookmarks = read_bookmarks(&path);
        assert_eq!(bookmarks.len(), 3);
        assert_eq!(bookmarks[0].url, "https://example.com");
        assert_eq!(bookmarks[0].date, "2026-03-01");
        assert_eq!(bookmarks[1].title, "Rust Programming Language");
        assert_eq!(bookmarks[2].description, "Where people build software");
    }

    #[test]
    fn test_read_bookmarks_empty() {
        let (_dir, path) = create_temp_csv("date,url,title,description\n");
        let bookmarks = read_bookmarks(&path);
        assert!(bookmarks.is_empty());
    }

    #[test]
    fn test_read_bookmarks_missing_file() {
        let path = PathBuf::from("/nonexistent/bookmarks.csv");
        let bookmarks = read_bookmarks(&path);
        assert!(bookmarks.is_empty());
    }

    #[test]
    fn test_write_bookmarks_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.csv");
        let bookmarks = vec![
            Bookmark {
                date: "2026-03-29".into(),
                url: "https://example.com".into(),
                title: "Example".into(),
                description: "A test site".into(),
            },
            Bookmark {
                date: "2026-03-28".into(),
                url: "https://rust-lang.org".into(),
                title: "Rust".into(),
                description: "".into(),
            },
        ];
        write_bookmarks(&bookmarks, &path).unwrap();
        let read_back = read_bookmarks(&path);
        assert_eq!(read_back, bookmarks);
    }

    #[test]
    fn test_csv_handles_commas_in_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.csv");
        let bookmarks = vec![Bookmark {
            date: "2026-03-29".into(),
            url: "https://example.com".into(),
            title: "Title, with comma".into(),
            description: "Desc \"with quotes\"".into(),
        }];
        write_bookmarks(&bookmarks, &path).unwrap();
        let read_back = read_bookmarks(&path);
        assert_eq!(read_back[0].title, "Title, with comma");
        assert_eq!(read_back[0].description, "Desc \"with quotes\"");
    }

    // -- is_duplicate --

    #[test]
    fn test_is_duplicate_true() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let bookmarks = read_bookmarks(&path);
        assert!(is_duplicate("https://example.com", &bookmarks));
    }

    #[test]
    fn test_is_duplicate_false() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let bookmarks = read_bookmarks(&path);
        assert!(!is_duplicate("https://new-site.com", &bookmarks));
    }

    // -- add_entry --

    #[test]
    fn test_add_entry_with_title() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let result = add_entry("https://new.com", Some("New Site".into()), Some("A new site".into()), &path).unwrap();
        assert!(result.contains("Bookmarked:"));
        let bookmarks = read_bookmarks(&path);
        assert_eq!(bookmarks.len(), 4);
        assert_eq!(bookmarks[3].url, "https://new.com");
        assert_eq!(bookmarks[3].title, "New Site");
        assert_eq!(bookmarks[3].description, "A new site");
        assert!(!bookmarks[3].date.is_empty());
    }

    #[test]
    fn test_add_entry_no_title() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        add_entry("https://notitle.com", None, None, &path).unwrap();
        let bookmarks = read_bookmarks(&path);
        assert_eq!(bookmarks.len(), 4);
        assert_eq!(bookmarks[3].url, "https://notitle.com");
        assert_eq!(bookmarks[3].title, "");
    }

    #[test]
    fn test_add_entry_records_date() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        add_entry("https://dated.com", Some("Dated".into()), None, &path).unwrap();
        let bookmarks = read_bookmarks(&path);
        let added = &bookmarks[3];
        assert_eq!(added.date, today());
    }

    // -- cmd_list --

    #[test]
    fn test_list_shows_all_with_dates() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let output = cmd_list(&path);
        assert!(output.contains("[2026-03-01]"));
        assert!(output.contains("https://example.com"));
        assert!(output.contains("[2026-03-15]"));
        assert!(output.contains("https://rust-lang.org"));
        assert!(output.contains("[2026-03-29]"));
        assert!(output.contains("https://github.com"));
        assert!(output.contains("3 bookmark(s)"));
    }

    #[test]
    fn test_list_empty() {
        let (_dir, path) = create_temp_csv("date,url,title,description\n");
        let output = cmd_list(&path);
        assert_eq!(output, "No bookmarks found.");
    }

    // -- cmd_search --

    #[test]
    fn test_search_found() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let output = cmd_search("rust", &path);
        assert!(output.contains("https://rust-lang.org"));
        assert!(output.contains("1 result(s)"));
    }

    #[test]
    fn test_search_case_insensitive() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let output = cmd_search("GITHUB", &path);
        assert!(output.contains("https://github.com"));
    }

    #[test]
    fn test_search_by_description() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let output = cmd_search("systems programming", &path);
        assert!(output.contains("https://rust-lang.org"));
        assert!(output.contains("1 result(s)"));
    }

    #[test]
    fn test_search_no_match() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let output = cmd_search("xyznonexistent", &path);
        assert!(output.contains("No bookmarks matching"));
    }

    // -- cmd_remove --

    #[test]
    fn test_remove_by_index() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let result = cmd_remove("2", &path).unwrap();
        assert!(result.contains("Removed:"));
        assert!(result.contains("rust-lang.org"));
        let bookmarks = read_bookmarks(&path);
        assert_eq!(bookmarks.len(), 2);
        assert!(!bookmarks.iter().any(|b| b.url.contains("rust-lang")));
    }

    #[test]
    fn test_remove_by_url() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let result = cmd_remove("https://github.com", &path).unwrap();
        assert!(result.contains("Removed:"));
        let bookmarks = read_bookmarks(&path);
        assert_eq!(bookmarks.len(), 2);
    }

    #[test]
    fn test_remove_first_entry() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let result = cmd_remove("1", &path).unwrap();
        assert!(result.contains("example.com"));
        let bookmarks = read_bookmarks(&path);
        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].url, "https://rust-lang.org");
    }

    #[test]
    fn test_remove_index_zero() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let result = cmd_remove("0", &path);
        assert!(result.is_err());
        match result.unwrap_err() {
            BmError::IndexOutOfRange(0, 3) => {}
            other => panic!("Expected IndexOutOfRange(0, 3), got: {}", other),
        }
    }

    #[test]
    fn test_remove_index_too_large() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        assert!(cmd_remove("99", &path).is_err());
    }

    #[test]
    fn test_remove_url_not_found() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let result = cmd_remove("https://nothere.com", &path);
        assert!(result.is_err());
        match result.unwrap_err() {
            BmError::BookmarkNotFound(u) => assert_eq!(u, "https://nothere.com"),
            other => panic!("Expected BookmarkNotFound, got: {}", other),
        }
    }

    // -- add then remove roundtrip --

    #[test]
    fn test_add_then_remove_roundtrip() {
        let (_dir, path) = create_temp_csv(SAMPLE_CSV);
        let original_count = read_bookmarks(&path).len();

        add_entry("https://temp.com", Some("Temp".into()), None, &path).unwrap();
        assert_eq!(read_bookmarks(&path).len(), original_count + 1);

        cmd_remove("https://temp.com", &path).unwrap();
        let after = read_bookmarks(&path);
        assert_eq!(after.len(), original_count);
        assert!(!after.iter().any(|b| b.url == "https://temp.com"));
    }

    // -- migration --

    #[test]
    fn test_migrate_md_to_csv() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("bookmarks.md");
        let csv_path = dir.path().join("bookmarks.csv");
        fs::write(
            &md_path,
            "# Saved URLs\n\n- https://example.com - Example Domain\n- https://bare.com\n",
        )
        .unwrap();
        migrate_md_to_csv(&md_path, &csv_path).unwrap();

        let bookmarks = read_bookmarks(&csv_path);
        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].url, "https://example.com");
        assert_eq!(bookmarks[0].title, "Example Domain");
        assert_eq!(bookmarks[1].url, "https://bare.com");
        assert_eq!(bookmarks[1].title, "");
    }

    // -- format_display_label --

    #[test]
    fn test_format_display_label_both() {
        assert_eq!(format_display_label("GitHub", "Where people build"), "GitHub: Where people build");
    }

    #[test]
    fn test_format_display_label_title_only() {
        assert_eq!(format_display_label("GitHub", ""), "GitHub");
    }

    #[test]
    fn test_format_display_label_desc_only() {
        assert_eq!(format_display_label("", "A great site"), "A great site");
    }

    #[test]
    fn test_format_display_label_neither() {
        assert_eq!(format_display_label("", ""), "");
    }

    #[test]
    fn test_format_display_label_desc_has_title_prefix() {
        assert_eq!(
            format_display_label("GitHub", "GitHub: Where people build software"),
            "GitHub: Where people build software"
        );
    }

    #[test]
    fn test_format_display_label_identical() {
        assert_eq!(format_display_label("Same", "Same"), "Same");
    }

    // -- csv_path_from --

    #[test]
    fn test_csv_path_from_md() {
        let p = PathBuf::from("/tmp/bookmarks.md");
        assert_eq!(csv_path_from(&p), PathBuf::from("/tmp/bookmarks.csv"));
    }

    #[test]
    fn test_csv_path_from_csv() {
        let p = PathBuf::from("/tmp/bookmarks.csv");
        assert_eq!(csv_path_from(&p), PathBuf::from("/tmp/bookmarks.csv"));
    }

    // -- ensure_csv_file --

    #[test]
    fn test_ensure_csv_file_creates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.csv");
        assert!(!path.exists());
        ensure_csv_file(&path).unwrap();
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("date,url,title,description"));
    }

    // -- config --

    #[test]
    fn test_load_config_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "bookmarks_file = \"/tmp/bm.csv\"\n").unwrap();
        let cfg = load_config_from(&path).unwrap();
        assert_eq!(cfg.bookmarks_file, "/tmp/bm.csv");
    }

    #[test]
    fn test_load_config_missing() {
        let path = PathBuf::from("/nonexistent/config.toml");
        assert!(load_config_from(&path).is_err());
    }

    #[test]
    fn test_load_config_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "not valid toml [[[").unwrap();
        assert!(load_config_from(&path).is_err());
    }

    // -- BmError Display --

    #[test]
    fn test_error_display() {
        let e = BmError::IndexOutOfRange(5, 3);
        assert_eq!(format!("{}", e), "Index 5 out of range (1-3).");
        let e = BmError::BookmarkNotFound("https://x.com".into());
        assert_eq!(format!("{}", e), "Bookmark not found: https://x.com");
    }

    // -- resolve_path --

    #[test]
    fn test_resolve_path_absolute() {
        assert_eq!(resolve_path("/tmp/bm.csv"), PathBuf::from("/tmp/bm.csv"));
    }

    #[test]
    fn test_resolve_path_tilde() {
        let p = resolve_path("~/test.csv");
        assert!(p.to_string_lossy().contains("test.csv"));
        assert!(!p.to_string_lossy().starts_with("~"));
    }

    // -- HTML extraction (kept from before) --

    #[test]
    fn test_extract_title_basic() {
        let html = "<html><head><title>Hello World</title></head></html>";
        assert_eq!(extract_title_from_html(html), Some("Hello World".into()));
    }

    #[test]
    fn test_extract_title_with_entities() {
        let html = "<title>Foo &amp; Bar &ndash; Baz</title>";
        assert_eq!(extract_title_from_html(html), Some("Foo & Bar \u{2013} Baz".into()));
    }

    #[test]
    fn test_extract_title_empty() {
        assert_eq!(extract_title_from_html("<title>  </title>"), None);
    }

    #[test]
    fn test_extract_title_missing() {
        assert_eq!(extract_title_from_html("<html><body>No title</body></html>"), None);
    }

    #[test]
    fn test_extract_description_og() {
        let html = r#"<meta property="og:description" content="OG desc">"#;
        assert_eq!(extract_description_from_html(html), Some("OG desc".into()));
    }

    #[test]
    fn test_extract_description_meta() {
        let html = r#"<meta name="description" content="Meta desc">"#;
        assert_eq!(extract_description_from_html(html), Some("Meta desc".into()));
    }

    #[test]
    fn test_decode_all_entities() {
        let s = "&amp; &lt; &gt; &#39; &quot; &#x27; &ndash; &mdash;";
        assert_eq!(decode_html_entities(s), "& < > ' \" ' \u{2013} \u{2014}");
    }

    // -- is_private_url --

    #[test]
    fn test_is_private_url_localhost() {
        assert!(is_private_url("http://localhost:3000/api"));
        assert!(is_private_url("http://127.0.0.1:8080/test"));
        assert!(is_private_url("http://[::1]:9000/"));
    }

    #[test]
    fn test_is_private_url_public() {
        assert!(!is_private_url("https://example.com"));
        assert!(!is_private_url("https://github.com/user/repo"));
    }

    // -- is_x_url --

    #[test]
    fn test_is_x_url() {
        assert!(is_x_url("https://x.com/user/status/123"));
        assert!(is_x_url("https://twitter.com/user/status/123"));
        assert!(!is_x_url("https://example.com"));
    }

    // -- clean_x_title --

    #[test]
    fn test_clean_x_title() {
        assert_eq!(clean_x_title("YQ on X: \"Hello\" / X"), Some("YQ on X: \"Hello\"".into()));
        assert_eq!(clean_x_title("X"), None);
        assert_eq!(clean_x_title(""), None);
    }

    // -- extract_first_paragraph --

    #[test]
    fn test_extract_first_paragraph_skips_images() {
        let content = "[![Image](url)](link)\n\nThe quick brown fox jumps over the lazy dog.";
        assert_eq!(extract_first_paragraph(content), Some("The quick brown fox jumps over the lazy dog.".into()));
    }

    #[test]
    fn test_extract_first_paragraph_truncates() {
        let long = "A".repeat(200);
        let result = extract_first_paragraph(&long).unwrap();
        assert!(result.len() < 130);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_extract_first_paragraph_empty() {
        assert_eq!(extract_first_paragraph(""), None);
        assert_eq!(extract_first_paragraph("# Only header"), None);
    }
}
