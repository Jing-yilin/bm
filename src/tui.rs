use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};
use std::path::PathBuf;
use std::time::Duration;

use crate::{bookmark_entries, cmd_remove, extract_url, read_bookmarks, BmError};

#[derive(PartialEq)]
enum Mode {
    Normal,
    Search,
    ConfirmDelete,
}

struct Bookmark {
    url: String,
    label: String,
    #[allow(dead_code)]
    line_idx: usize,
}

pub struct App {
    bm_path: PathBuf,
    bookmarks: Vec<Bookmark>,
    filtered: Vec<usize>,
    table_state: TableState,
    mode: Mode,
    search_query: String,
    should_quit: bool,
    status_msg: String,
}

impl App {
    pub fn new(bm_path: PathBuf) -> Self {
        let mut app = App {
            bm_path,
            bookmarks: Vec::new(),
            filtered: Vec::new(),
            table_state: TableState::default(),
            mode: Mode::Normal,
            search_query: String::new(),
            should_quit: false,
            status_msg: String::new(),
        };
        app.reload();
        if !app.filtered.is_empty() {
            app.table_state.select(Some(0));
        }
        app
    }

    fn reload(&mut self) {
        let lines = read_bookmarks(&self.bm_path);
        let entries = bookmark_entries(&lines);
        self.bookmarks = entries
            .iter()
            .map(|(line_idx, line)| {
                let url = extract_url(line).to_string();
                let stripped = line.strip_prefix("- ").unwrap_or(line);
                let label = stripped
                    .strip_prefix(&url)
                    .and_then(|s| s.strip_prefix(" - "))
                    .unwrap_or("")
                    .to_string();
                Bookmark {
                    url,
                    label,
                    line_idx: *line_idx,
                }
            })
            .collect();
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered = if q.is_empty() {
            (0..self.bookmarks.len()).collect()
        } else {
            self.bookmarks
                .iter()
                .enumerate()
                .filter(|(_, b)| {
                    b.url.to_lowercase().contains(&q) || b.label.to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect()
        };
        if self.filtered.is_empty() {
            self.table_state.select(None);
        } else {
            let sel = self.table_state.selected().unwrap_or(0);
            if sel >= self.filtered.len() {
                self.table_state.select(Some(self.filtered.len() - 1));
            }
        }
    }

    fn selected_bookmark(&self) -> Option<&Bookmark> {
        self.table_state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .map(|&idx| &self.bookmarks[idx])
    }

    fn next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i >= self.filtered.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.filtered.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn open_selected(&mut self) {
        if let Some(bm) = self.selected_bookmark() {
            let url = bm.url.clone();
            let _ = std::process::Command::new("open").arg(&url).spawn();
            self.status_msg = format!("Opened: {}", url);
        }
    }

    fn delete_selected(&mut self) {
        if let Some(bm) = self.selected_bookmark() {
            let url = bm.url.clone();
            match cmd_remove(&url, &self.bm_path) {
                Ok(msg) => {
                    self.status_msg = msg;
                    self.reload();
                    if self.filtered.is_empty() {
                        self.table_state.select(None);
                    } else {
                        let sel = self.table_state.selected().unwrap_or(0);
                        if sel >= self.filtered.len() {
                            self.table_state
                                .select(Some(self.filtered.len().saturating_sub(1)));
                        }
                    }
                }
                Err(e) => {
                    self.status_msg = format!("Error: {}", e);
                }
            }
        }
        self.mode = Mode::Normal;
    }

    fn handle_event(&mut self) -> Result<(), BmError> {
        if event::poll(Duration::from_millis(100))
            .map_err(|e| BmError::IoError(e.to_string()))?
        {
            if let Event::Key(key) = event::read().map_err(|e| BmError::IoError(e.to_string()))? {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    self.should_quit = true;
                    return Ok(());
                }
                match self.mode {
                    Mode::Normal => self.handle_normal_key(key),
                    Mode::Search => self.handle_search_key(key),
                    Mode::ConfirmDelete => self.handle_confirm_key(key),
                }
            }
        }
        Ok(())
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.next(),
            KeyCode::Char('k') | KeyCode::Up => self.previous(),
            KeyCode::Enter => self.open_selected(),
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.status_msg.clear();
            }
            KeyCode::Char('d') => {
                if self.selected_bookmark().is_some() {
                    self.mode = Mode::ConfirmDelete;
                    if let Some(bm) = self.selected_bookmark() {
                        self.status_msg = format!("Delete {}? (y/n)", bm.url);
                    }
                }
            }
            KeyCode::Char('g') => {
                if !self.filtered.is_empty() {
                    self.table_state.select(Some(0));
                }
            }
            KeyCode::Char('G') => {
                if !self.filtered.is_empty() {
                    self.table_state
                        .select(Some(self.filtered.len().saturating_sub(1)));
                }
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.search_query.clear();
                self.apply_filter();
                self.mode = Mode::Normal;
                if !self.filtered.is_empty() {
                    self.table_state.select(Some(0));
                }
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.apply_filter();
                if !self.filtered.is_empty() {
                    self.table_state.select(Some(0));
                }
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.apply_filter();
                if !self.filtered.is_empty() {
                    self.table_state.select(Some(0));
                }
            }
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.delete_selected(),
            _ => {
                self.mode = Mode::Normal;
                self.status_msg.clear();
            }
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::vertical([
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(frame.area());

        self.render_table(frame, chunks[0]);
        self.render_footer(frame, chunks[1]);
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect) {
        let header = Row::new(vec![
            Cell::from(" # "),
            Cell::from("URL"),
            Cell::from("Description"),
        ])
        .style(Style::default().bold().fg(Color::Cyan))
        .bottom_margin(0);

        let rows: Vec<Row> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(display_idx, &bm_idx)| {
                let bm = &self.bookmarks[bm_idx];
                Row::new(vec![
                    Cell::from(format!("{:>3}", display_idx + 1)),
                    Cell::from(bm.url.as_str()),
                    Cell::from(bm.label.as_str()),
                ])
            })
            .collect();

        let count = self.filtered.len();
        let total = self.bookmarks.len();
        let title = if self.search_query.is_empty() {
            format!(" Bookmarks ({}) ", total)
        } else {
            format!(" Bookmarks ({}/{}) ", count, total)
        };

        let widths = [
            Constraint::Length(4),
            Constraint::Percentage(40),
            Constraint::Percentage(55),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .row_highlight_style(
                Style::default()
                    .add_modifier(Modifier::REVERSED)
                    .fg(Color::Yellow),
            )
            .highlight_symbol(">> ");

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let mut spans = Vec::new();

        if self.mode == Mode::Search {
            spans.push(Span::styled(" /", Style::default().fg(Color::Yellow)));
            spans.push(Span::raw(&self.search_query));
            spans.push(Span::styled("█", Style::default().fg(Color::Yellow)));
            spans.push(Span::raw("  "));
        } else if !self.search_query.is_empty() {
            spans.push(Span::styled(
                format!(" filter: {} ", self.search_query),
                Style::default().fg(Color::Yellow),
            ));
        }

        if !self.status_msg.is_empty() {
            spans.push(Span::styled(
                format!(" {} ", self.status_msg),
                Style::default().fg(Color::Green),
            ));
        }

        if self.mode == Mode::Normal && self.status_msg.is_empty() {
            spans.push(Span::styled(
                " j/k",
                Style::default().fg(Color::Cyan),
            ));
            spans.push(Span::raw(":nav "));
            spans.push(Span::styled("Enter", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw(":open "));
            spans.push(Span::styled("/", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw(":search "));
            spans.push(Span::styled("d", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw(":del "));
            spans.push(Span::styled("g/G", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw(":top/bottom "));
            spans.push(Span::styled("q", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw(":quit"));
        }

        let footer = Paragraph::new(Line::from(spans));
        frame.render_widget(footer, area);
    }
}

pub fn run_tui(bm_path: PathBuf) -> Result<(), BmError> {
    let mut terminal =
        ratatui::init();
    let mut app = App::new(bm_path);

    let result = (|| -> Result<(), BmError> {
        loop {
            terminal
                .draw(|frame| app.render(frame))
                .map_err(|e| BmError::IoError(e.to_string()))?;
            app.handle_event()?;
            if app.should_quit {
                break;
            }
        }
        Ok(())
    })();

    ratatui::restore();
    result
}
