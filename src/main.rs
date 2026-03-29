use clap::{Parser, Subcommand};
use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process;

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

fn load_config() -> Config {
    let path = config_path();
    if path.exists() {
        let content = fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_else(|e| {
            eprintln!("Invalid config {}: {}", path.display(), e);
            process::exit(1);
        })
    } else {
        eprintln!(
            "No config found. Create {} with:\n\n  bookmarks_file = \"/path/to/bookmarks.md\"\n",
            path.display()
        );
        process::exit(1);
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

fn fetch_title(url: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;

    let resp = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) bm/0.1")
        .send()
        .ok()?;

    let body = resp.text().ok()?;
    let re = Regex::new(r"(?i)<title[^>]*>([\s\S]*?)</title>").ok()?;
    let caps = re.captures(&body)?;
    let title = caps.get(1)?.as_str().trim().to_string();
    let title = title
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&ndash;", "\u{2013}")
        .replace("&mdash;", "\u{2014}");

    if title.is_empty() { None } else { Some(title) }
}

fn cmd_add(url: &str, bm_path: &PathBuf) {
    let lines = read_bookmarks(bm_path);
    let entries = bookmark_entries(&lines);
    for (_, line) in &entries {
        if extract_url(line) == url {
            println!("Already bookmarked: {}", url);
            return;
        }
    }

    print!("Fetching title... ");
    let entry = match fetch_title(url) {
        Some(title) => {
            println!("{}", title);
            format!("- {} - {}", url, title)
        }
        None => {
            println!("(not found)");
            format!("- {}", url)
        }
    };

    let mut content = fs::read_to_string(bm_path).unwrap_or_default();
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&entry);
    content.push('\n');
    fs::write(bm_path, content).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {}", bm_path.display(), e);
        process::exit(1);
    });

    println!("Bookmarked: {}", entry.strip_prefix("- ").unwrap_or(&entry));
}

fn cmd_list(bm_path: &PathBuf) {
    let lines = read_bookmarks(bm_path);
    let entries = bookmark_entries(&lines);
    if entries.is_empty() {
        println!("No bookmarks found.");
        return;
    }
    for (idx, (_, line)) in entries.iter().enumerate() {
        let content = line.strip_prefix("- ").unwrap_or(line);
        println!("  {}. {}", idx + 1, content);
    }
    println!("\n{} bookmark(s)", entries.len());
}

fn cmd_search(query: &str, bm_path: &PathBuf) {
    let lines = read_bookmarks(bm_path);
    let entries = bookmark_entries(&lines);
    let q = query.to_lowercase();
    let matches: Vec<_> = entries
        .iter()
        .enumerate()
        .filter(|(_, (_, line))| line.to_lowercase().contains(&q))
        .collect();

    if matches.is_empty() {
        println!("No bookmarks matching \"{}\".", query);
        return;
    }
    for (_, (_, line)) in &matches {
        let content = line.strip_prefix("- ").unwrap_or(line);
        println!("  {}", content);
    }
    println!("\n{} result(s)", matches.len());
}

fn cmd_remove(target: &str, bm_path: &PathBuf) {
    let lines = read_bookmarks(bm_path);
    let entries = bookmark_entries(&lines);

    let remove_line_idx = if let Ok(idx) = target.parse::<usize>() {
        if idx == 0 || idx > entries.len() {
            eprintln!("Index {} out of range (1-{}).", idx, entries.len());
            process::exit(1);
        }
        entries[idx - 1].0
    } else {
        match entries.iter().find(|(_, line)| extract_url(line) == target) {
            Some((line_idx, _)) => *line_idx,
            None => {
                eprintln!("Bookmark not found: {}", target);
                process::exit(1);
            }
        }
    };

    let removed = &lines[remove_line_idx];
    println!(
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
    fs::write(bm_path, content).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {}", bm_path.display(), e);
        process::exit(1);
    });
}

fn main() {
    let cli = Cli::parse();
    let config = load_config();
    let bm_path = resolve_path(&config.bookmarks_file);

    if !bm_path.exists() {
        eprintln!("Bookmarks file not found: {}", bm_path.display());
        process::exit(1);
    }

    match cli.command {
        Some(Commands::Add { url }) => cmd_add(&url, &bm_path),
        Some(Commands::List) => cmd_list(&bm_path),
        Some(Commands::Search { query }) => cmd_search(&query, &bm_path),
        Some(Commands::Remove { target }) => cmd_remove(&target, &bm_path),
        None => {
            if let Some(url) = cli.url {
                cmd_add(&url, &bm_path);
            } else {
                cmd_list(&bm_path);
            }
        }
    }
}
