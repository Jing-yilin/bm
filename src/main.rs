use clap::{Parser, Subcommand};
use regex::Regex;
use serde::Deserialize;
use std::fmt;
use std::fs;
use std::path::PathBuf;

#[derive(Debug)]
enum BmError {
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

fn read_bookmarks(path: &PathBuf) -> Vec<String> {
    let content = fs::read_to_string(path).unwrap_or_default();
    content.lines().map(String::from).collect()
}

fn bookmark_entries(lines: &[String]) -> Vec<(usize, &str)> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("- "))
        .map(|(i, l)| (i, l.as_str()))
        .collect()
}

fn extract_url(line: &str) -> &str {
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

fn fetch_metadata(url: &str) -> Option<String> {
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
    format_bookmark_label(meta.title, meta.description)
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

fn cmd_remove(target: &str, bm_path: &PathBuf) -> Result<String, BmError> {
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
        None => {
            if let Some(url) = cli.url {
                println!("{}", cmd_add(&url, &bm_path)?);
            } else {
                println!("{}", cmd_list(&bm_path));
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
