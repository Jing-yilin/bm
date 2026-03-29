# bm

CLI bookmark manager backed by a markdown file.

## Install

```bash
cargo build --release
alias bm="$(pwd)/target/release/bm"
```

## Config

Create `~/.config/bm/config.toml`:

```toml
bookmarks_file = "/path/to/bookmarks.md"
```

## Usage

```bash
bm <url>                  # Add a bookmark (auto-fetches page title)
bm add <url>              # Same as above
bm list                   # List all bookmarks
bm search <query>         # Search by keyword (case-insensitive)
bm remove <index|url>     # Remove by 1-based index or URL
```

## Bookmarks format

```markdown
# Saved URLs

- https://example.com - Example Domain
- https://rust-lang.org - Rust Programming Language
```

## License

MIT
