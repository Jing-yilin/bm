mod tui;

use clap::{Parser, Subcommand};
use regex::Regex;
use serde::Deserialize;
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
                "No config found. Create {} with:\n\n  bookmarks_file = \"/path/to/bookmarks.md\"\n",
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

#[derive(Parser)]
#[command(name = "bm", about = "CLI bookmark manager backed by a markdown file")]
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
    if p.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            return home.join(&p[2..]);
        }
    }
    PathBuf::from(p)
}

pub fn read_bookmarks(path: &PathBuf) -> Vec<String> {
    let content = fs::read_to_string(path).unwrap_or_default();
    content.lines().map(String::from).collect()
}

pub fn bookmark_entries(lines: &[String]) -> Vec<(usize, &str)> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("- "))
        .map(|(i, l)| (i, l.as_str()))
        .collect()
}

pub fn extract_url(line: &str) -> &str {
    let stripped = line.strip_prefix("- ").unwrap_or(line);
    stripped.split_whitespace().next().unwrap_or(stripped)
}

fn extract_title_from_html(html: &str) -> Option<String> {
    let re = Regex::new(r"(?i)<title[^>]*>([\s\S]*?)</title>").ok()?;
    let caps = re.captures(html)?;
    let title = caps.get(1)?.as_str().trim().to_string();
    let title = decode_html_entities(&title);
    if title.is_empty() { None } else { Some(title) }
}

fn extract_description_from_html(html: &str) -> Option<String> {
    // Try og:description first, then meta description
    let og_re = Regex::new(r#"(?i)<meta\s[^>]*property\s*=\s*"og:description"[^>]*content\s*=\s*"([^"]*)"[^>]*/?\s*>"#).ok()?;
    if let Some(caps) = og_re.captures(html) {
        let desc = decode_html_entities(caps.get(1)?.as_str().trim());
        if !desc.is_empty() {
            return Some(desc);
        }
    }
    // Also try content before property order
    let og_re2 = Regex::new(r#"(?i)<meta\s[^>]*content\s*=\s*"([^"]*)"[^>]*property\s*=\s*"og:description"[^>]*/?\s*>"#).ok()?;
    if let Some(caps) = og_re2.captures(html) {
        let desc = decode_html_entities(caps.get(1)?.as_str().trim());
        if !desc.is_empty() {
            return Some(desc);
        }
    }
    // Fallback: meta name="description"
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

struct PageMeta {
    title: Option<String>,
    description: Option<String>,
}

fn extract_metadata_from_html(html: &str) -> PageMeta {
    PageMeta {
        title: extract_title_from_html(html),
        description: extract_description_from_html(html),
    }
}

fn format_bookmark_label(title: Option<String>, description: Option<String>) -> Option<String> {
    match (title, description) {
        (Some(t), Some(d)) => {
            let t_lower = t.to_lowercase();
            let d_lower = d.to_lowercase();
            if t_lower == d_lower {
                Some(t)
            } else if d_lower.starts_with(&format!("{} - ", t_lower))
                || d_lower.starts_with(&format!("{}: ", t_lower))
                || d_lower.starts_with(&format!("{} | ", t_lower))
            {
                // Description already has "Title: ..." or "Title - ..." pattern
                Some(d)
            } else if d_lower.starts_with(&format!("{} is ", t_lower))
                || d_lower.starts_with(&format!("{} — ", t_lower))
            {
                // "Expo is an open-source..." -> "Expo: Open-source..."
                let rest = &d[t.len()..].trim_start();
                let rest = rest.strip_prefix("is ").or_else(|| rest.strip_prefix("— ")).unwrap_or(rest);
                let rest = uppercase_first(rest);
                Some(format!("{}: {}", t, rest))
            } else {
                Some(format!("{}: {}", t, d))
            }
        }
        (Some(t), None) => Some(t),
        (None, Some(d)) => Some(d),
        (None, None) => None,
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
            let boundary = trimmed.char_indices()
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
    let t = t.strip_suffix(" / X")
        .or_else(|| t.strip_suffix(" / Twitter"))
        .unwrap_or(t);
    if t.is_empty() { None } else { Some(t.to_string()) }
}

fn fetch_metadata_via_jina(url: &str) -> Option<String> {
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
    let description = data.get("description").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("");
    let content = data.get("content").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("");

    let title = if is_x_url(url) {
        clean_x_title(raw_title)
    } else {
        let t = raw_title.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    };

    let desc = if description.trim().is_empty() {
        None
    } else {
        Some(description.trim().to_string())
    };

    let label = format_bookmark_label(title.clone(), desc);
    if label.is_some() {
        return label;
    }

    extract_first_paragraph(content)
}

fn fetch_metadata(url: &str) -> Option<String> {
    if is_x_url(url) {
        return fetch_metadata_via_jina(url);
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;

    let resp = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) bm/0.1",
        )
        .send()
        .ok()?;

    let body = resp.text().ok()?;
    let meta = extract_metadata_from_html(&body);
    let label = format_bookmark_label(meta.title, meta.description);
    if label.is_some() {
        return label;
    }

    // Fallback to Jina Reader for JS-heavy sites
    fetch_metadata_via_jina(url)
}

fn is_duplicate(url: &str, bm_path: &PathBuf) -> bool {
    let lines = read_bookmarks(bm_path);
    let entries = bookmark_entries(&lines);
    entries.iter().any(|(_, line)| extract_url(line) == url)
}

fn append_entry(entry: &str, bm_path: &PathBuf) -> Result<(), BmError> {
    let mut content = fs::read_to_string(bm_path)
        .map_err(|e| BmError::IoError(format!("Failed to read {}: {}", bm_path.display(), e)))?;
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(entry);
    content.push('\n');
    fs::write(bm_path, content)
        .map_err(|e| BmError::IoError(format!("Failed to write {}: {}", bm_path.display(), e)))
}

fn cmd_add(url: &str, bm_path: &PathBuf) -> Result<String, BmError> {
    if is_duplicate(url, bm_path) {
        return Ok(format!("Already bookmarked: {}", url));
    }

    print!("Fetching metadata... ");
    let label = fetch_metadata(url);
    match &label {
        Some(l) => println!("{}", l),
        None => println!("(not found)"),
    }

    add_entry(url, label, bm_path)
}

fn add_entry(url: &str, title: Option<String>, bm_path: &PathBuf) -> Result<String, BmError> {
    let entry = match title {
        Some(t) => format!("- {} - {}", url, t),
        None => format!("- {}", url),
    };

    append_entry(&entry, bm_path)?;
    Ok(format!(
        "Bookmarked: {}",
        entry.strip_prefix("- ").unwrap_or(&entry)
    ))
}

fn cmd_list(bm_path: &PathBuf) -> String {
    let lines = read_bookmarks(bm_path);
    let entries = bookmark_entries(&lines);
    if entries.is_empty() {
        return "No bookmarks found.".to_string();
    }
    let mut output = String::new();
    for (idx, (_, line)) in entries.iter().enumerate() {
        let content = line.strip_prefix("- ").unwrap_or(line);
        output.push_str(&format!("  {}. {}\n", idx + 1, content));
    }
    output.push_str(&format!("\n{} bookmark(s)", entries.len()));
    output
}

fn cmd_search(query: &str, bm_path: &PathBuf) -> String {
    let lines = read_bookmarks(bm_path);
    let entries = bookmark_entries(&lines);
    let q = query.to_lowercase();
    let matches: Vec<_> = entries
        .iter()
        .filter(|(_, line)| line.to_lowercase().contains(&q))
        .collect();

    if matches.is_empty() {
        return format!("No bookmarks matching \"{}\".", query);
    }
    let mut output = String::new();
    for (_, line) in &matches {
        let content = line.strip_prefix("- ").unwrap_or(line);
        output.push_str(&format!("  {}\n", content));
    }
    output.push_str(&format!("\n{} result(s)", matches.len()));
    output
}

pub fn cmd_remove(target: &str, bm_path: &PathBuf) -> Result<String, BmError> {
    let lines = read_bookmarks(bm_path);
    let entries = bookmark_entries(&lines);

    let remove_line_idx = if let Ok(idx) = target.parse::<usize>() {
        if idx == 0 || idx > entries.len() {
            return Err(BmError::IndexOutOfRange(idx, entries.len()));
        }
        entries[idx - 1].0
    } else {
        match entries.iter().find(|(_, line)| extract_url(line) == target) {
            Some((line_idx, _)) => *line_idx,
            None => return Err(BmError::BookmarkNotFound(target.to_string())),
        }
    };

    let removed = &lines[remove_line_idx];
    let msg = format!(
        "Removed: {}",
        removed.strip_prefix("- ").unwrap_or(removed)
    );

    let new_lines: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != remove_line_idx)
        .map(|(_, l)| l.as_str())
        .collect();

    let mut content = new_lines.join("\n");
    if !content.ends_with('\n') {
        content.push('\n');
    }
    fs::write(bm_path, content)
        .map_err(|e| BmError::IoError(format!("Failed to write {}: {}", bm_path.display(), e)))?;

    Ok(msg)
}

fn run() -> Result<(), BmError> {
    let cli = Cli::parse();
    let config = load_config_from(&config_path())?;
    let bm_path = resolve_path(&config.bookmarks_file);

    if !bm_path.exists() {
        return Err(BmError::FileNotFound(bm_path));
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

    fn create_temp_bookmarks(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bookmarks.md");
        fs::write(&path, content).unwrap();
        (dir, path)
    }

    const SAMPLE: &str = "\
# Saved URLs

- https://example.com - Example Domain
- https://rust-lang.org - Rust Programming Language
- https://github.com - GitHub
";

    // -- extract_url --

    #[test]
    fn test_extract_url_with_title() {
        assert_eq!(
            extract_url("- https://example.com - Example"),
            "https://example.com"
        );
    }

    #[test]
    fn test_extract_url_bare() {
        assert_eq!(extract_url("- https://bare.com"), "https://bare.com");
    }

    #[test]
    fn test_extract_url_no_prefix() {
        assert_eq!(
            extract_url("https://no-prefix.com - Title"),
            "https://no-prefix.com"
        );
    }

    // -- extract_title_from_html --

    #[test]
    fn test_extract_title_basic() {
        let html = "<html><head><title>Hello World</title></head></html>";
        assert_eq!(extract_title_from_html(html), Some("Hello World".into()));
    }

    #[test]
    fn test_extract_title_with_entities() {
        let html = "<title>Foo &amp; Bar &ndash; Baz</title>";
        assert_eq!(
            extract_title_from_html(html),
            Some("Foo & Bar \u{2013} Baz".into())
        );
    }

    #[test]
    fn test_extract_title_multiline() {
        let html = "<title>\n  Multi\n  Line\n</title>";
        assert_eq!(extract_title_from_html(html), Some("Multi\n  Line".into()));
    }

    #[test]
    fn test_extract_title_empty() {
        let html = "<title>  </title>";
        assert_eq!(extract_title_from_html(html), None);
    }

    #[test]
    fn test_extract_title_missing() {
        let html = "<html><body>No title here</body></html>";
        assert_eq!(extract_title_from_html(html), None);
    }

    #[test]
    fn test_extract_title_case_insensitive() {
        let html = "<TITLE>Upper Case</TITLE>";
        assert_eq!(
            extract_title_from_html(html),
            Some("Upper Case".into())
        );
    }

    #[test]
    fn test_extract_title_with_attributes() {
        let html = "<title lang=\"en\">With Attrs</title>";
        assert_eq!(
            extract_title_from_html(html),
            Some("With Attrs".into())
        );
    }

    // -- extract_description_from_html --

    #[test]
    fn test_extract_description_meta() {
        let html = r#"<html><head><meta name="description" content="A great site about Rust"></head></html>"#;
        assert_eq!(
            extract_description_from_html(html),
            Some("A great site about Rust".into())
        );
    }

    #[test]
    fn test_extract_description_og() {
        let html = r#"<meta property="og:description" content="OG description here">"#;
        assert_eq!(
            extract_description_from_html(html),
            Some("OG description here".into())
        );
    }

    #[test]
    fn test_extract_description_og_preferred_over_meta() {
        let html = r#"<meta property="og:description" content="OG wins"><meta name="description" content="Meta loses">"#;
        assert_eq!(
            extract_description_from_html(html),
            Some("OG wins".into())
        );
    }

    #[test]
    fn test_extract_description_content_before_name() {
        let html = r#"<meta content="Reversed order" name="description">"#;
        assert_eq!(
            extract_description_from_html(html),
            Some("Reversed order".into())
        );
    }

    #[test]
    fn test_extract_description_content_before_og() {
        let html = r#"<meta content="Reversed OG" property="og:description">"#;
        assert_eq!(
            extract_description_from_html(html),
            Some("Reversed OG".into())
        );
    }

    #[test]
    fn test_extract_description_empty() {
        let html = r#"<meta name="description" content="">"#;
        assert_eq!(extract_description_from_html(html), None);
    }

    #[test]
    fn test_extract_description_missing() {
        let html = "<html><head><title>No desc</title></head></html>";
        assert_eq!(extract_description_from_html(html), None);
    }

    #[test]
    fn test_extract_description_with_entities() {
        let html = r#"<meta name="description" content="Foo &amp; Bar &ndash; Baz">"#;
        assert_eq!(
            extract_description_from_html(html),
            Some("Foo & Bar \u{2013} Baz".into())
        );
    }

    // -- format_bookmark_label --

    #[test]
    fn test_format_label_title_and_description() {
        let result = format_bookmark_label(Some("Expo".into()), Some("Build cross-platform apps".into()));
        assert_eq!(result, Some("Expo: Build cross-platform apps".into()));
    }

    #[test]
    fn test_format_label_title_only() {
        let result = format_bookmark_label(Some("Expo".into()), None);
        assert_eq!(result, Some("Expo".into()));
    }

    #[test]
    fn test_format_label_description_only() {
        let result = format_bookmark_label(None, Some("A great site".into()));
        assert_eq!(result, Some("A great site".into()));
    }

    #[test]
    fn test_format_label_neither() {
        let result = format_bookmark_label(None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_label_description_has_title_prefix_separator() {
        let result = format_bookmark_label(
            Some("GitHub".into()),
            Some("GitHub: Where people build software".into()),
        );
        assert_eq!(result, Some("GitHub: Where people build software".into()));
    }

    #[test]
    fn test_format_label_description_starts_with_title_is() {
        let result = format_bookmark_label(
            Some("Expo".into()),
            Some("Expo is an open-source platform".into()),
        );
        assert_eq!(result, Some("Expo: An open-source platform".into()));
    }

    #[test]
    fn test_format_label_no_special_prefix() {
        let result = format_bookmark_label(
            Some("MyApp".into()),
            Some("Build amazing things".into()),
        );
        assert_eq!(result, Some("MyApp: Build amazing things".into()));
    }

    #[test]
    fn test_format_label_identical() {
        let result = format_bookmark_label(Some("Same".into()), Some("Same".into()));
        assert_eq!(result, Some("Same".into()));
    }

    // -- extract_metadata_from_html --

    #[test]
    fn test_extract_metadata_full() {
        let html = r#"<html><head><title>My Site</title><meta name="description" content="Best site ever"></head></html>"#;
        let meta = extract_metadata_from_html(html);
        assert_eq!(meta.title, Some("My Site".into()));
        assert_eq!(meta.description, Some("Best site ever".into()));
    }

    #[test]
    fn test_extract_metadata_title_only() {
        let html = "<html><head><title>Just Title</title></head></html>";
        let meta = extract_metadata_from_html(html);
        assert_eq!(meta.title, Some("Just Title".into()));
        assert_eq!(meta.description, None);
    }

    // -- decode_html_entities --

    #[test]
    fn test_decode_all_entities() {
        let s = "&amp; &lt; &gt; &#39; &quot; &#x27; &ndash; &mdash;";
        assert_eq!(
            decode_html_entities(s),
            "& < > ' \" ' \u{2013} \u{2014}"
        );
    }

    // -- bookmark_entries --

    #[test]
    fn test_bookmark_entries_filters_header() {
        let lines: Vec<String> = SAMPLE.lines().map(String::from).collect();
        let entries = bookmark_entries(&lines);
        assert_eq!(entries.len(), 3);
        assert!(entries[0].1.starts_with("- https://example.com"));
    }

    #[test]
    fn test_bookmark_entries_empty() {
        let lines: Vec<String> = vec!["# Header".into(), "".into()];
        assert_eq!(bookmark_entries(&lines).len(), 0);
    }

    // -- resolve_path --

    #[test]
    fn test_resolve_path_absolute() {
        let p = resolve_path("/tmp/bookmarks.md");
        assert_eq!(p, PathBuf::from("/tmp/bookmarks.md"));
    }

    #[test]
    fn test_resolve_path_tilde() {
        let p = resolve_path("~/test.md");
        assert!(p.to_string_lossy().contains("test.md"));
        assert!(!p.to_string_lossy().starts_with("~"));
    }

    // -- is_duplicate --

    #[test]
    fn test_is_duplicate_true() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        assert!(is_duplicate("https://example.com", &path));
    }

    #[test]
    fn test_is_duplicate_false() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        assert!(!is_duplicate("https://new-site.com", &path));
    }

    // -- add_entry (testable version without network) --

    #[test]
    fn test_add_new_bookmark_with_title() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = add_entry("https://new.com", Some("New Site".into()), &path).unwrap();
        assert!(result.contains("Bookmarked:"));
        assert!(result.contains("https://new.com"));
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("- https://new.com - New Site"));
    }

    #[test]
    fn test_add_new_bookmark_no_title() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = add_entry("https://notitle.com", None, &path).unwrap();
        assert!(result.contains("Bookmarked:"));
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("- https://notitle.com\n"));
    }

    #[test]
    fn test_add_duplicate_detected() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        assert!(is_duplicate("https://example.com", &path));
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches("https://example.com").count(), 1);
    }

    #[test]
    fn test_add_entry_does_not_dedup() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = add_entry("https://example.com", Some("Dup".into()), &path).unwrap();
        assert!(result.contains("Bookmarked:"));
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches("https://example.com").count(), 2);
    }

    // -- cmd_list --

    #[test]
    fn test_list_shows_all() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let output = cmd_list(&path);
        assert!(output.contains("1. https://example.com"));
        assert!(output.contains("2. https://rust-lang.org"));
        assert!(output.contains("3. https://github.com"));
        assert!(output.contains("3 bookmark(s)"));
    }

    #[test]
    fn test_list_empty() {
        let (_dir, path) = create_temp_bookmarks("# Saved URLs\n");
        let output = cmd_list(&path);
        assert_eq!(output, "No bookmarks found.");
    }

    // -- cmd_search --

    #[test]
    fn test_search_found() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let output = cmd_search("rust", &path);
        assert!(output.contains("https://rust-lang.org"));
        assert!(output.contains("1 result(s)"));
    }

    #[test]
    fn test_search_case_insensitive() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let output = cmd_search("GITHUB", &path);
        assert!(output.contains("https://github.com"));
    }

    #[test]
    fn test_search_multiple_results() {
        let content = "\
# Saved URLs

- https://a.com - Rust Book
- https://b.com - Rust By Example
- https://c.com - Python Docs
";
        let (_dir, path) = create_temp_bookmarks(content);
        let output = cmd_search("rust", &path);
        assert!(output.contains("2 result(s)"));
    }

    #[test]
    fn test_search_no_match() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let output = cmd_search("xyznonexistent", &path);
        assert!(output.contains("No bookmarks matching"));
    }

    // -- cmd_remove --

    #[test]
    fn test_remove_by_index() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = cmd_remove("2", &path).unwrap();
        assert!(result.contains("Removed:"));
        assert!(result.contains("rust-lang.org"));
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("rust-lang.org"));
        assert!(content.contains("example.com"));
        assert!(content.contains("github.com"));
    }

    #[test]
    fn test_remove_by_url() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = cmd_remove("https://github.com", &path).unwrap();
        assert!(result.contains("Removed:"));
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("github.com"));
    }

    #[test]
    fn test_remove_first_entry() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = cmd_remove("1", &path).unwrap();
        assert!(result.contains("example.com"));
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("example.com"));
        assert!(content.contains("rust-lang.org"));
    }

    #[test]
    fn test_remove_last_entry() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = cmd_remove("3", &path).unwrap();
        assert!(result.contains("github.com"));
    }

    #[test]
    fn test_remove_index_zero() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = cmd_remove("0", &path);
        assert!(result.is_err());
        match result.unwrap_err() {
            BmError::IndexOutOfRange(0, 3) => {}
            other => panic!("Expected IndexOutOfRange(0, 3), got: {}", other),
        }
    }

    #[test]
    fn test_remove_index_too_large() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = cmd_remove("99", &path);
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_url_not_found() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let result = cmd_remove("https://nothere.com", &path);
        assert!(result.is_err());
        match result.unwrap_err() {
            BmError::BookmarkNotFound(u) => assert_eq!(u, "https://nothere.com"),
            other => panic!("Expected BookmarkNotFound, got: {}", other),
        }
    }

    // -- load_config_from --

    #[test]
    fn test_load_config_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "bookmarks_file = \"/tmp/bm.md\"\n").unwrap();
        let cfg = load_config_from(&path).unwrap();
        assert_eq!(cfg.bookmarks_file, "/tmp/bm.md");
    }

    #[test]
    fn test_load_config_missing() {
        let path = PathBuf::from("/nonexistent/config.toml");
        let result = load_config_from(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_config_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "not valid toml [[[").unwrap();
        let result = load_config_from(&path);
        assert!(result.is_err());
    }

    // -- BmError Display --

    #[test]
    fn test_error_display() {
        let e = BmError::IndexOutOfRange(5, 3);
        assert_eq!(format!("{}", e), "Index 5 out of range (1-3).");

        let e = BmError::BookmarkNotFound("https://x.com".into());
        assert_eq!(format!("{}", e), "Bookmark not found: https://x.com");
    }

    // -- is_x_url --

    #[test]
    fn test_is_x_url_https() {
        assert!(is_x_url("https://x.com/user/status/123"));
        assert!(is_x_url("https://twitter.com/user/status/123"));
    }

    #[test]
    fn test_is_x_url_www() {
        assert!(is_x_url("https://www.x.com/user/status/123"));
        assert!(is_x_url("https://www.twitter.com/user/status/123"));
    }

    #[test]
    fn test_is_x_url_http() {
        assert!(is_x_url("http://x.com/user/status/123"));
        assert!(is_x_url("http://twitter.com/user/status/123"));
    }

    #[test]
    fn test_is_x_url_false() {
        assert!(!is_x_url("https://example.com"));
        assert!(!is_x_url("https://github.com"));
        assert!(!is_x_url("https://notx.com/path"));
        assert!(!is_x_url("https://xtwitter.com/path"));
    }

    // -- clean_x_title --

    #[test]
    fn test_clean_x_title_with_suffix() {
        assert_eq!(
            clean_x_title("YQ on X: \"Agents Don't Click Ads.\" / X"),
            Some("YQ on X: \"Agents Don't Click Ads.\"".into())
        );
    }

    #[test]
    fn test_clean_x_title_twitter_suffix() {
        assert_eq!(
            clean_x_title("User on X: \"Hello\" / Twitter"),
            Some("User on X: \"Hello\"".into())
        );
    }

    #[test]
    fn test_clean_x_title_generic() {
        assert_eq!(clean_x_title("X"), None);
        assert_eq!(clean_x_title("Twitter"), None);
        assert_eq!(clean_x_title(""), None);
    }

    // -- extract_first_paragraph --

    #[test]
    fn test_extract_first_paragraph_skips_images() {
        let content = "[![Image 1](https://img.png)](https://link)\n\nThe quick brown fox jumps over the lazy dog.";
        assert_eq!(
            extract_first_paragraph(content),
            Some("The quick brown fox jumps over the lazy dog.".into())
        );
    }

    #[test]
    fn test_extract_first_paragraph_skips_headers() {
        let content = "# Title\n\n## Subtitle\n\nThis is the first real paragraph of content here.";
        assert_eq!(
            extract_first_paragraph(content),
            Some("This is the first real paragraph of content here.".into())
        );
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

    #[test]
    fn test_is_x_url_case_insensitive() {
        assert!(is_x_url("https://X.COM/user/status/123"));
        assert!(is_x_url("HTTPS://x.com/user/status/123"));
        assert!(is_x_url("https://Twitter.com/user/status/456"));
    }

    #[test]
    fn test_is_x_url_article_path() {
        assert!(is_x_url("https://x.com/yq_acc/article/2037579506429657198"));
    }

    #[test]
    fn test_is_x_url_edge_cases() {
        assert!(!is_x_url("https://x.com"));
        assert!(!is_x_url(""));
        assert!(!is_x_url("x.com/user/status/123"));
    }

    #[test]
    fn test_clean_x_title_no_suffix() {
        assert_eq!(
            clean_x_title("YQ on X: \"Some text\""),
            Some("YQ on X: \"Some text\"".into())
        );
    }

    #[test]
    fn test_clean_x_title_whitespace() {
        assert_eq!(
            clean_x_title("  User on X: \"Hello\" / X  "),
            Some("User on X: \"Hello\"".into())
        );
        assert_eq!(clean_x_title("   "), None);
    }

    #[test]
    fn test_extract_first_paragraph_skips_links() {
        let content = "[Log in](https://x.com/login)\n[Sign up](https://x.com/signup)\n\nThis is meaningful text after some links.";
        assert_eq!(
            extract_first_paragraph(content),
            Some("This is meaningful text after some links.".into())
        );
    }

    #[test]
    fn test_extract_first_paragraph_skips_inline_images() {
        let content = "![Image 1: alt text](https://img.png)\n\nThe actual paragraph starts here with real text.";
        assert_eq!(
            extract_first_paragraph(content),
            Some("The actual paragraph starts here with real text.".into())
        );
    }

    #[test]
    fn test_extract_first_paragraph_skips_short_lines() {
        let content = "OK\nNot enough\nShort line here!!!\n\nThis line is long enough to be a real paragraph for the reader.";
        assert_eq!(
            extract_first_paragraph(content),
            Some("This line is long enough to be a real paragraph for the reader.".into())
        );
    }

    #[test]
    fn test_extract_first_paragraph_unicode_truncation() {
        let content = "这是一段很长的中文内容，用来测试Unicode字符的截断功能。我们需要确保截断不会发生在多字节字符的中间位置，否则会导致无效的UTF-8字符串。这段文字应该足够长以触发截断逻辑。";
        let result = extract_first_paragraph(content).unwrap();
        assert!(result.ends_with("..."));
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn test_extract_first_paragraph_trims_whitespace() {
        let content = "   \n   \n   The first real paragraph with leading spaces.   ";
        assert_eq!(
            extract_first_paragraph(content),
            Some("The first real paragraph with leading spaces.".into())
        );
    }

    #[test]
    fn test_extract_first_paragraph_realistic_jina_x_output() {
        let content = r#"[![Image 1: Image](https://pbs.twimg.com/media/abc.jpg)](https://x.com/user/article/123/media/456)

The internet's business model is advertising. For thirty years, that has been the default: show humans content, harvest attention, convert clicks into revenue.

AI agents break this model. An agent calling an API does not have attention to harvest."#;
        let result = extract_first_paragraph(content).unwrap();
        assert!(result.starts_with("The internet's business model is advertising."));
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_extract_first_paragraph_only_non_text() {
        let content = "# Header\n## Subheader\n[![img](url)](link)\n![alt](url)\n[link](url)\nshort";
        assert_eq!(extract_first_paragraph(content), None);
    }

    // -- integration: add then remove preserves file --

    #[test]
    fn test_add_then_remove_roundtrip() {
        let (_dir, path) = create_temp_bookmarks(SAMPLE);
        let original = fs::read_to_string(&path).unwrap();
        let original_count = original.matches("- https://").count();

        add_entry("https://temp.com", Some("Temp".into()), &path).unwrap();
        let after_add = fs::read_to_string(&path).unwrap();
        assert_eq!(after_add.matches("- https://").count(), original_count + 1);

        cmd_remove("https://temp.com", &path).unwrap();
        let after_remove = fs::read_to_string(&path).unwrap();
        assert_eq!(
            after_remove.matches("- https://").count(),
            original_count
        );
        assert!(!after_remove.contains("temp.com"));
    }
}
