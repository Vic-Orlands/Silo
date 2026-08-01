use arboard::Clipboard;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use silo_core::{
    generate_totp, inspect_totp, load_vault, new_entry, save_vault, Entry, SecretString, Vault,
};
use std::{
    io::{self, stdout},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use time::OffsetDateTime;
use zeroize::Zeroizing;

const CANVAS: Color = Color::Rgb(12, 14, 12);
const SURFACE: Color = Color::Rgb(10, 10, 10);
const MODAL_SURFACE: Color = Color::Rgb(7, 8, 7);
const MODAL_BACKDROP: Color = Color::Rgb(3, 4, 3);
const SURFACE_2: Color = Color::Rgb(18, 18, 18);
const BORDER: Color = Color::Rgb(42, 42, 42);
const BORDER_FOCUS: Color = Color::Rgb(72, 72, 72);
const INK: Color = Color::Rgb(255, 255, 255);
const MUTED: Color = Color::Rgb(162, 162, 162);
const FAINT: Color = Color::Rgb(102, 102, 102);
const EMERALD: Color = Color::Rgb(46, 204, 113);
const CORAL: Color = Color::Rgb(255, 136, 102);
const TRACK: Color = Color::Rgb(37, 37, 37);

const FORM_FIELD_COUNT: usize = 6;
/// Lines per authentication row in the list (title + username + spacer).
const LIST_ITEM_LINES: u16 = 3;

pub fn run(path: &Path, timeout: u64) -> Result<(), Box<dyn std::error::Error>> {
    if timeout == 0 {
        return Err("shell timeout must be greater than zero".into());
    }
    enable_raw_mode()?;
    let mut output = stdout();
    execute!(
        output,
        EnterAlternateScreen,
        EnableMouseCapture,
        crossterm::event::EnableFocusChange
    )?;
    let backend = CrosstermBackend::new(output);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_loop(
        &mut terminal,
        path.to_path_buf(),
        Duration::from_secs(timeout),
    );

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        crossterm::event::DisableFocusChange,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
}

struct App {
    path: PathBuf,
    vault: Vault,
    master: Option<Zeroizing<String>>,
    screen: Screen,
    selected: usize,
    search: String,
    search_cursor: usize,
    search_mode: bool,
    detail_open: bool,
    /// When set, overview field-nav is active (`→`) and `c` copies that field.
    field_nav: Option<usize>,
    form: Option<FormState>,
    confirm_delete: bool,
    help_open: bool,
    help_scroll: usize,
    status: String,
    status_kind: StatusKind,
    last_activity: Instant,
    timeout: Duration,
    broker: Option<silo_broker::BrokerHandle>,
    started: Instant,
    hit: HitRegions,
}

#[derive(Default, Clone)]
struct HitRegions {
    list: Option<Rect>,
    search_input: Option<Rect>,
    detail_card: Option<Rect>,
    form_fields: Vec<Rect>,
    unlock_input: Option<Rect>,
}

enum Screen {
    Unlock {
        password: String,
        cursor: usize,
    },
    /// Premium step sequence after a successful unlock or create.
    Ceremony(Ceremony),
    Browse,
}

enum Ceremony {
    Unlocking {
        started: Instant,
        vault: Vault,
        master: Zeroizing<String>,
    },
    Created {
        started: Instant,
        name: String,
        username: String,
        email: String,
        url: String,
        has_totp: bool,
    },
}

#[derive(Clone, Copy)]
enum StatusKind {
    Info,
    Success,
    Error,
}

struct FormState {
    id: Option<uuid::Uuid>,
    title: String,
    url: String,
    username: String,
    email: String,
    password: Zeroizing<String>,
    totp: Zeroizing<String>,
    focus: usize,
    cursors: [usize; FORM_FIELD_COUNT],
    reveal_password: bool,
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    path: PathBuf,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App {
        path,
        vault: Vault::new(),
        master: None,
        screen: Screen::Unlock {
            password: String::new(),
            cursor: 0,
        },
        selected: 0,
        search: String::new(),
        search_cursor: 0,
        search_mode: false,
        detail_open: false,
        field_nav: None,
        form: None,
        confirm_delete: false,
        help_open: false,
        help_scroll: 0,
        status: "Type your master password to unlock.".into(),
        status_kind: StatusKind::Info,
        last_activity: Instant::now(),
        timeout,
        broker: None,
        started: Instant::now(),
        hit: HitRegions::default(),
    };

    loop {
        advance_ceremony(&mut app)?;
        terminal.draw(|frame| draw(frame, &mut app))?;
        if app.last_activity.elapsed() >= app.timeout && matches!(app.screen, Screen::Browse) {
            app.lock_for_inactivity();
        }
        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) => {
                    app.last_activity = Instant::now();
                    if handle_key(&mut app, key)? {
                        break;
                    }
                }
                Event::Mouse(mouse) => {
                    app.last_activity = Instant::now();
                    handle_mouse(&mut app, mouse);
                }
                Event::FocusGained if matches!(app.screen, Screen::Browse) => {
                    app.lock_for_inactivity();
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn advance_ceremony(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let done = match &app.screen {
        Screen::Ceremony(Ceremony::Unlocking { started, .. }) => {
            started.elapsed() >= Duration::from_millis(1650)
        }
        Screen::Ceremony(Ceremony::Created { started, .. }) => {
            started.elapsed() >= Duration::from_millis(2400)
        }
        _ => false,
    };
    if !done {
        return Ok(());
    }

    match std::mem::replace(&mut app.screen, Screen::Browse) {
        Screen::Ceremony(Ceremony::Unlocking { vault, master, .. }) => {
            let broker = silo_broker::start_with_vault(
                vault.clone(),
                app.path.clone(),
                master.clone(),
                app.timeout.as_secs(),
            )?;
            app.vault = vault;
            app.master = Some(master);
            app.broker = Some(broker);
            app.detail_open = false;
            app.set_status(
                "Silo is ready. Select a login and press enter.",
                StatusKind::Success,
            );
        }
        Screen::Ceremony(Ceremony::Created { .. }) => {
            app.set_status("Authentication secured in Silo.", StatusKind::Success);
        }
        other => app.screen = other,
    }
    Ok(())
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    app.hit = HitRegions::default();
    frame.render_widget(
        Block::default().style(Style::default().bg(CANVAS)),
        frame.area(),
    );
    match &app.screen {
        Screen::Unlock { .. } => draw_unlock(frame, app),
        Screen::Ceremony(ceremony) => {
            // Clone ceremony data for drawing without holding a borrow on app.screen
            let ceremony = match ceremony {
                Ceremony::Unlocking { started, .. } => {
                    CeremonyDraw::Unlocking { started: *started }
                }
                Ceremony::Created {
                    started,
                    name,
                    username,
                    email,
                    url,
                    has_totp,
                } => CeremonyDraw::Created {
                    started: *started,
                    name: name.clone(),
                    username: username.clone(),
                    email: email.clone(),
                    url: url.clone(),
                    has_totp: *has_totp,
                },
            };
            draw_ceremony(frame, app, &ceremony);
        }
        Screen::Browse => draw_workspace(frame, app),
    }
}

enum CeremonyDraw {
    Unlocking {
        started: Instant,
    },
    Created {
        started: Instant,
        name: String,
        username: String,
        email: String,
        url: String,
        has_totp: bool,
    },
}

// --- Unlock / login (borderless) ---------------------------------------------

fn draw_unlock(frame: &mut ratatui::Frame, app: &mut App) {
    let area = centered(frame.area(), 56, 20);
    frame.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);

    let (password, cursor) = match &app.screen {
        Screen::Unlock { password, cursor } => (password.clone(), *cursor),
        _ => unreachable!(),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "Silo",
                Style::default().fg(INK).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Your vault stays local.",
                Style::default().fg(MUTED),
            )),
        ]),
        chunks[0],
    );

    let masked = "•".repeat(password.chars().count());
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Master password",
            Style::default().fg(MUTED),
        ))),
        chunks[2],
    );
    app.hit.unlock_input = Some(chunks[3]);
    frame.render_widget(
        Paragraph::new(caret_line(
            &masked,
            cursor,
            cursor_on(app),
            Style::default().fg(INK),
        )),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            app.status.as_str(),
            status_style(app.status_kind),
        ))),
        chunks[4],
    );
    frame.render_widget(
        Paragraph::new(Line::from(hint_spans(&[
            ("enter", "unlock"),
            ("esc", "quit"),
            ("ctrl-u", "clear"),
        ]))),
        chunks[5],
    );
}

// --- Ceremonies (Daytona-style loaders) --------------------------------------

fn draw_ceremony(frame: &mut ratatui::Frame, app: &App, ceremony: &CeremonyDraw) {
    let area = centered(frame.area(), 62, 18);
    frame.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);

    match ceremony {
        CeremonyDraw::Unlocking { started } => {
            let ms = started.elapsed().as_millis();
            let mut lines = vec![
                Line::from(Span::styled(
                    "Opening Silo",
                    Style::default().fg(INK).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
            ];
            lines.push(check_line("Reading encrypted vault", ms >= 180));
            lines.push(Line::from(""));
            lines.push(check_line("Verifying master key", ms >= 420));
            lines.push(Line::from(""));
            lines.push(check_line("Decrypting authentications", ms >= 700));
            lines.push(Line::from(""));

            let progress = ((ms.saturating_sub(700) as f64) / 900.0).clamp(0.0, 1.0);
            let elapsed = format!("{:.1}s", ms as f64 / 1000.0);
            lines.push(Line::from(vec![
                Span::styled(
                    if progress >= 1.0 { "✓  " } else { "   " },
                    Style::default().fg(EMERALD),
                ),
                Span::styled("Opening private vault", Style::default().fg(INK)),
            ]));

            frame.render_widget(Paragraph::new(lines), area);

            let bar = Rect {
                x: area.x + 3,
                y: area.y + 11,
                width: 28,
                height: 1,
            };
            frame.render_widget(
                Gauge::default()
                    .gauge_style(Style::default().fg(EMERALD).bg(TRACK))
                    .ratio(progress)
                    .label(""),
                bar,
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("  {elapsed}"),
                    Style::default().fg(FAINT),
                ))),
                Rect {
                    x: bar.x + bar.width + 1,
                    y: bar.y,
                    width: 8,
                    height: 1,
                },
            );
        }
        CeremonyDraw::Created {
            started,
            name,
            username,
            email,
            url,
            has_totp,
        } => {
            let ms = started.elapsed().as_millis();
            let mut lines = vec![
                Line::from(Span::styled(
                    "Securing authentication",
                    Style::default().fg(INK).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
            ];
            lines.push(check_line("Encrypting credentials", ms >= 160));
            lines.push(Line::from(""));
            lines.push(check_line("Writing to local vault", ms >= 380));
            lines.push(Line::from(""));
            lines.push(check_line("Indexing authentication", ms >= 620));
            lines.push(Line::from(""));

            frame.render_widget(Paragraph::new(lines), area);

            if ms >= 820 {
                let box_area = Rect {
                    x: area.x,
                    y: area.y + 10,
                    width: area.width,
                    height: 9,
                };
                let login = display_title(name);
                let email_value = if email.is_empty() {
                    "—"
                } else {
                    email.as_str()
                };
                let otp_value = if *has_totp {
                    "configured"
                } else {
                    "not configured"
                };
                draw_kv_card(
                    frame,
                    box_area,
                    &[
                        ("Login", login.as_str(), false),
                        ("Username", username, false),
                        ("Email", email_value, false),
                        ("URL", url, false),
                        ("Password", "••••••••••••••••", false),
                        ("OTP", otp_value, *has_totp),
                    ],
                    None,
                );
            }

            let _ = app;
        }
    }
}

fn check_line(label: &str, done: bool) -> Line<'static> {
    if done {
        Line::from(vec![
            Span::styled("✓  ", Style::default().fg(EMERALD)),
            Span::styled(label.to_string(), Style::default().fg(INK)),
        ])
    } else {
        Line::from(vec![
            Span::styled("·  ", Style::default().fg(FAINT)),
            Span::styled(label.to_string(), Style::default().fg(FAINT)),
        ])
    }
}

// --- Workspace ---------------------------------------------------------------

fn draw_workspace(frame: &mut ratatui::Frame, app: &mut App) {
    let content_width = frame.area().width.saturating_sub(6).max(1);
    let search_h = search_wrapped_height(app, content_width);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),        // brand + total
            Constraint::Length(1),        // nav border
            Constraint::Length(1),        // margin under nav
            Constraint::Min(8),           // body
            Constraint::Length(1),        // margin above find label
            Constraint::Length(1),        // Find a login · status
            Constraint::Length(1),        // margin top on search input
            Constraint::Length(search_h), // wrapping search input
            Constraint::Length(1),        // margin above key bindings
            Constraint::Length(1),        // key bindings
        ])
        .split(inset(frame.area(), 3, 1));

    draw_header(frame, app, outer[0]);
    draw_nav_border(frame, outer[1]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(30),
            Constraint::Length(4),
            Constraint::Min(24),
        ])
        .split(outer[3]);

    draw_entry_list(frame, app, body[0]);
    draw_detail(frame, app, body[2]);
    draw_search_label(frame, app, outer[5]);
    draw_search_input(frame, app, outer[7]);
    draw_footer(frame, app, outer[9]);

    if app.form.is_some() || app.confirm_delete || app.help_open {
        dim_area(frame, frame.area());
    }
    if app.help_open {
        draw_help(frame, app);
    }
    if app.form.is_some() {
        draw_form(frame, app);
    }
    if app.confirm_delete {
        draw_delete_modal(frame, app);
    }
}

fn draw_header(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "SILO",
            Style::default().fg(INK).add_modifier(Modifier::BOLD),
        ))),
        left[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Private vault", Style::default().fg(FAINT)),
            Span::styled(" · ", Style::default().fg(FAINT)),
            Span::styled("Unlocked", Style::default().fg(EMERALD)),
        ])),
        left[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("Total logins in Silo: {}", app.vault.entries.len()),
            Style::default().fg(MUTED),
        )))
        .alignment(Alignment::Right),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );
}

fn draw_nav_border(frame: &mut ratatui::Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            Style::default().fg(BORDER),
        ))),
        area,
    );
}

fn draw_search_label(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Find a login", Style::default().fg(MUTED)),
            Span::styled(" · ", Style::default().fg(FAINT)),
            Span::styled(app.status.as_str(), status_style(app.status_kind)),
        ])),
        area,
    );
}

fn draw_search_input(frame: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.hit.search_input = Some(area);
    let style = Style::default()
        .fg(if app.search.is_empty() && !app.search_mode {
            FAINT
        } else {
            INK
        })
        .bg(SURFACE_2);

    // Full-bleed surface, then equal top/bottom + left padding for the text.
    frame.render_widget(Block::default().style(Style::default().bg(SURFACE_2)), area);
    let inner = pad(area, 1, 1);

    let text = if app.search.is_empty() && !app.search_mode {
        "type to filter…".to_string()
    } else {
        app.search.clone()
    };
    let line = if app.search_mode {
        caret_line(
            &text,
            app.search_cursor.min(text.chars().count()),
            cursor_on(app),
            style,
        )
    } else {
        Line::from(Span::styled(text, style))
    };

    frame.render_widget(
        Paragraph::new(line)
            .style(Style::default().bg(SURFACE_2))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_entry_list(frame: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(4)])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Authentications",
            Style::default().fg(INK).add_modifier(Modifier::BOLD),
        ))),
        chunks[0],
    );

    app.hit.list = Some(chunks[1]);
    let indices = visible_indices(app);
    let mut items = Vec::new();
    for (visible_i, index) in indices.iter().enumerate() {
        let entry = &app.vault.entries[*index];
        let selected = visible_i == app.selected.min(indices.len().saturating_sub(1));
        let title_style = if selected {
            Style::default().fg(EMERALD).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(INK)
        };
        items.push(ListItem::new(vec![
            Line::from(vec![
                selection_gutter(selected),
                Span::styled(display_title(&entry.name), title_style),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(entry.username.clone(), Style::default().fg(FAINT)),
            ]),
            Line::from(""),
        ]));
    }

    let list = List::new(if items.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  no logins yet",
            Style::default().fg(MUTED),
        )))]
    } else {
        items
    });
    let mut state = ListState::default();
    frame.render_stateful_widget(list, chunks[1], &mut state);
}

fn draw_detail(frame: &mut ratatui::Frame, app: &mut App, area: Rect) {
    if !app.detail_open {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Your private vault.",
                    Style::default().fg(INK).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Select a login, then press enter to open details.",
                    Style::default().fg(MUTED),
                )),
            ]),
            area,
        );
        return;
    }

    let detail = {
        let Some(entry) = selected_entry(app) else {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "Select a login.",
                    Style::default().fg(MUTED),
                ))),
                area,
            );
            return;
        };

        let timestamp = now();
        let otp = entry
            .totp_secret
            .as_ref()
            .and_then(|secret| generate_totp(secret.as_str(), timestamp).ok());
        let remaining = 30 - timestamp % 30;
        let title = display_title(&entry.name);
        let email = if entry.email.is_empty() {
            "—".to_string()
        } else {
            entry.email.clone()
        };
        let username = entry.username.clone();
        let url = display_url(&entry.url);
        let has_totp = entry.totp_secret.is_some();
        (title, username, email, url, otp, remaining, has_totp)
    };
    let (title, username, email, url, otp, remaining, has_totp) = detail;

    let header = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(8),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("Authentication / {title}"),
                Style::default().fg(FAINT),
            )),
            Line::from(""),
        ]),
        header[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                title.clone(),
                Style::default().fg(INK).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(""),
        ]),
        header[1],
    );

    let mut rows: Vec<(&str, String, bool)> = vec![
        ("Username", username, false),
        ("Email", email, false),
        ("Password", "••••••••••••••••••".to_string(), false),
        ("URL", url, false),
    ];
    let otp_display = otp
        .as_deref()
        .map(format_otp)
        .unwrap_or_else(|| "not configured".to_string());
    if otp.is_some() || has_totp {
        rows.push(("OTP code", otp_display, otp.is_some()));
    }
    if let Some(nav) = app.field_nav {
        if nav >= rows.len() {
            app.field_nav = Some(rows.len().saturating_sub(1));
        }
    }
    let field_nav = app.field_nav;
    app.hit.detail_card = Some(header[2]);
    draw_overview_fields(frame, header[2], &rows, field_nav, remaining, otp.is_some());
}

/// Landing-style overview rows with separators and an OTP ring timer.
fn draw_overview_fields(
    frame: &mut ratatui::Frame,
    area: Rect,
    rows: &[(&str, String, bool)],
    selected: Option<usize>,
    remaining: u64,
    otp_live: bool,
) {
    let mut lines = Vec::new();
    for (i, (label, value, live)) in rows.iter().enumerate() {
        let active = selected == Some(i);
        let label_style = if active {
            Style::default().fg(EMERALD).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(FAINT)
        };
        let mut row = vec![
            selection_gutter(active),
            Span::styled(format!("{label:<14}"), label_style),
        ];
        if *live {
            row.push(Span::styled(
                value.clone(),
                Style::default().fg(EMERALD).add_modifier(Modifier::BOLD),
            ));
            if otp_live && *label == "OTP code" {
                let progress = remaining as f64 / 30.0;
                row.push(Span::raw("  "));
                row.push(Span::styled(
                    otp_ring(progress),
                    Style::default().fg(EMERALD),
                ));
                row.push(Span::styled(
                    format!(" {remaining}s"),
                    Style::default().fg(MUTED),
                ));
            }
        } else {
            row.push(Span::styled(value.clone(), Style::default().fg(INK)));
        }
        lines.push(Line::from(row));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            Style::default().fg(BORDER),
        )));
        lines.push(Line::from(""));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

/// Daytona-style bordered key/value card (Workspace / State / Editor).
fn draw_kv_card(
    frame: &mut ratatui::Frame,
    area: Rect,
    rows: &[(&str, &str, bool)],
    selected: Option<usize>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(SURFACE));
    let inner = pad(block.inner(area), 2, 1);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    for (i, (label, value, live)) in rows.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        let active = selected == Some(i);
        let label_style = if active {
            Style::default().fg(EMERALD).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MUTED)
        };
        lines.push(Line::from(vec![
            selection_gutter(active),
            Span::styled(format!("{label:<12}"), label_style),
            Span::styled(
                (*value).to_string(),
                if *live {
                    Style::default().fg(EMERALD).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(INK)
                },
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn draw_footer(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let hints = if app.field_nav.is_some() {
        hint_spans(&[
            ("↑↓", "field"),
            ("c", "copy field"),
            ("←", "exit"),
            ("?", "help"),
            ("q", "quit"),
        ])
    } else if app.detail_open {
        hint_spans(&[
            ("↑↓", "select"),
            ("→", "fields"),
            ("n", "new"),
            ("/", "search"),
            ("c", "copy password"),
            ("?", "help"),
            ("q", "quit"),
        ])
    } else {
        hint_spans(&[
            ("↑↓", "select"),
            ("enter", "open"),
            ("n", "new"),
            ("/", "search"),
            ("c", "copy password"),
            ("?", "help"),
            ("q", "quit"),
        ])
    };
    frame.render_widget(Paragraph::new(Line::from(hints)), area);
}

fn draw_form(frame: &mut ratatui::Frame, app: &mut App) {
    let Some(form) = app.form.as_ref() else {
        return;
    };
    let focus = form.focus;
    let cursors = form.cursors;
    let reveal = form.reveal_password;
    let is_edit = form.id.is_some();
    let field_values = [
        form.title.clone(),
        form.url.clone(),
        form.username.clone(),
        form.email.clone(),
        if reveal {
            form.password.to_string()
        } else if form.password.is_empty() {
            String::new()
        } else {
            "•".repeat(form.password.chars().count())
        },
        form.totp.to_string(),
    ];
    let blink = cursor_on(app);
    let panel = Style::default().bg(MODAL_SURFACE);

    let area = centered(frame.area(), 70, 26);
    let title = if is_edit { "Edit login" } else { "New login" };
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER_FOCUS).bg(MODAL_SURFACE))
            .title(Line::from(Span::styled(
                format!(" {title} "),
                Style::default()
                    .fg(INK)
                    .bg(MODAL_SURFACE)
                    .add_modifier(Modifier::BOLD),
            )))
            .style(panel),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(1)])
        .split(pad(area, 2, 1));

    let labels = [
        "Name",
        "URL",
        "Username",
        "Email",
        "Password",
        "TOTP secret",
    ];
    let field_area = chunks[0];
    let mut field_rects = Vec::new();
    let mut lines = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        let active = focus == index;
        let value = &field_values[index];
        let empty = value.is_empty();
        let cursor = cursors[index];
        let shown = if empty && !active {
            "—".to_string()
        } else {
            value.clone()
        };
        let value_style = panel.fg(if empty {
            FAINT
        } else if active {
            INK
        } else {
            MUTED
        });
        let mut row = vec![
            selection_gutter_on(active, SURFACE),
            Span::styled(
                format!("{label:<12}"),
                panel.fg(if active { MUTED } else { FAINT }),
            ),
        ];
        if active {
            row.extend(
                caret_line(
                    &shown,
                    cursor.min(shown.chars().count()),
                    blink,
                    value_style,
                )
                .spans,
            );
        } else {
            row.push(Span::styled(shown, value_style));
        }
        lines.push(Line::from(row));
        lines.push(Line::from(""));

        let y = field_area
            .y
            .saturating_add((index as u16).saturating_mul(2));
        field_rects.push(Rect {
            x: field_area.x,
            y,
            width: field_area.width,
            height: 2,
        });
    }

    app.hit.form_fields = field_rects;
    frame.render_widget(
        Paragraph::new(lines)
            .style(panel)
            .wrap(Wrap { trim: false }),
        field_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(hint_spans_on(
            &[
                ("tab", "next"),
                ("←→", "move"),
                ("ctrl-s", "save"),
                ("ctrl-u", "clear"),
                ("x", "reveal"),
                ("esc", "cancel"),
            ],
            SURFACE,
        )))
        .style(panel),
        chunks[1],
    );
}

fn draw_help(frame: &mut ratatui::Frame, app: &App) {
    let area = centered(frame.area(), 72, 22);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER_FOCUS))
            .title(Line::from(Span::styled(
                " Keys & how to use Silo ",
                Style::default().fg(INK).add_modifier(Modifier::BOLD),
            )))
            .style(Style::default().bg(MODAL_SURFACE)),
        area,
    );

    let inner = pad(area, 2, 1);
    let lines = help_lines();
    let visible = inner.height.saturating_sub(1) as usize;
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = app.help_scroll.min(max_scroll);
    let page: Vec<Line> = lines.into_iter().skip(scroll).take(visible).collect();

    frame.render_widget(
        Paragraph::new(page).style(Style::default().fg(INK).bg(MODAL_SURFACE)),
        inner,
    );
    frame.render_widget(
        Paragraph::new(Line::from(hint_spans(&[
            ("↑↓", "scroll"),
            ("esc", "close"),
        ])))
        .style(Style::default().fg(MUTED).bg(MODAL_SURFACE))
        .alignment(Alignment::Right),
        Rect {
            x: area.x + 2,
            y: area.y + area.height.saturating_sub(2),
            width: area.width.saturating_sub(4),
            height: 1,
        },
    );
}

fn help_lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let push = |lines: &mut Vec<Line<'static>>, key: &str, label: &str| {
        lines.push(Line::from(vec![
            Span::styled(format!("{key:<10}"), Style::default().fg(INK)),
            Span::styled(
                label.to_string(),
                Style::default().fg(FAINT).add_modifier(Modifier::ITALIC),
            ),
        ]));
    };

    lines.push(Line::from(Span::styled(
        "Navigation",
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    )));
    push(&mut lines, "↑ ↓ j k", "Move selection");
    push(&mut lines, "enter", "Open login details");
    push(&mut lines, "esc", "Close details, form, help, or quit");
    push(&mut lines, "/", "Focus Find a login");
    push(&mut lines, "→", "Enter overview field copy mode");
    push(&mut lines, "← / esc", "Leave field copy mode");
    push(&mut lines, "c", "Copy password (or selected field)");
    push(&mut lines, "ctrl-u", "Clear the active input");
    push(&mut lines, "?", "Open this help");
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Vault",
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    )));
    push(&mut lines, "n", "Create a new login");
    push(&mut lines, "e", "Edit selected login");
    push(&mut lines, "d", "Delete selected login");
    push(&mut lines, "c", "Copy password (clears in 20s)");
    push(&mut lines, "o", "Copy OTP code (clears in 20s)");
    push(&mut lines, "q", "Quit and lock Silo");
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Forms",
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    )));
    push(&mut lines, "tab / ↑↓", "Move between fields");
    push(&mut lines, "ctrl-s", "Save to encrypted vault");
    push(&mut lines, "x", "Reveal or hide password");
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "CLI",
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "silo init                     Create an encrypted vault",
        Style::default().fg(FAINT),
    )));
    lines.push(Line::from(Span::styled(
        "silo add <name>               Add a login (prompts for details)",
        Style::default().fg(FAINT),
    )));
    lines.push(Line::from(Span::styled(
        "silo list / show / get / otp  Read vault data",
        Style::default().fg(FAINT),
    )));
    lines.push(Line::from(Span::styled(
        "silo shell                   Interactive workspace (this UI)",
        Style::default().fg(FAINT),
    )));
    lines.push(Line::from(Span::styled(
        "silo --help                  Full command reference",
        Style::default().fg(FAINT),
    )));
    lines
}

fn dim_area(frame: &mut ratatui::Frame, area: Rect) {
    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(
                    Style::default()
                        .fg(FAINT)
                        .bg(MODAL_BACKDROP)
                        .add_modifier(Modifier::DIM),
                );
            }
        }
    }
}

fn draw_delete_modal(frame: &mut ratatui::Frame, app: &App) {
    let area = centered(frame.area(), 52, 11);
    let panel = Style::default().bg(MODAL_SURFACE);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER_FOCUS).bg(SURFACE))
            .style(panel),
        area,
    );
    let name = selected_entry(app)
        .map(|entry| display_title(&entry.name))
        .unwrap_or_else(|| "this entry".into());

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Remove this login?",
                panel.fg(INK).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Login           ", panel.fg(MUTED)),
                Span::styled(name, panel.fg(CORAL)),
            ]),
            Line::from(""),
            Line::from(hint_spans_on(&[("y", "confirm"), ("n", "cancel")], SURFACE)),
        ])
        .style(panel),
        pad(area, 2, 1),
    );
}

// --- Input -------------------------------------------------------------------

fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool, Box<dyn std::error::Error>> {
    if matches!(app.screen, Screen::Ceremony(_)) {
        if let Screen::Ceremony(Ceremony::Created { .. }) = &app.screen {
            if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                app.screen = Screen::Browse;
                app.set_status("Authentication secured in Silo.", StatusKind::Success);
            }
        }
        return Ok(false);
    }

    match &mut app.screen {
        Screen::Unlock { password, cursor } => {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('u') | KeyCode::Char('U'))
            {
                password.clear();
                *cursor = 0;
                return Ok(false);
            }
            match key.code {
                KeyCode::Esc => return Ok(true),
                KeyCode::Enter => match load_vault(&app.path, password) {
                    Ok(vault) => {
                        let master = Zeroizing::new(std::mem::take(password));
                        app.screen = Screen::Ceremony(Ceremony::Unlocking {
                            started: Instant::now(),
                            vault,
                            master,
                        });
                    }
                    Err(_) => {
                        password.clear();
                        *cursor = 0;
                        app.set_status(
                            "Could not unlock vault. Check the password.",
                            StatusKind::Error,
                        );
                    }
                },
                KeyCode::Left => *cursor = cursor.saturating_sub(1),
                KeyCode::Right => *cursor = (*cursor + 1).min(password.chars().count()),
                KeyCode::Home => *cursor = 0,
                KeyCode::End => *cursor = password.chars().count(),
                KeyCode::Backspace => delete_before(password, cursor),
                KeyCode::Delete => delete_after(password, *cursor),
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    insert_char(password, cursor, character);
                }
                _ => {}
            }
            return Ok(false);
        }
        Screen::Ceremony(_) => return Ok(false),
        Screen::Browse => {}
    }

    if app.help_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => app.help_open = false,
            KeyCode::Up | KeyCode::Char('k') => {
                app.help_scroll = app.help_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.help_scroll = app.help_scroll.saturating_add(1);
            }
            _ => {}
        }
        return Ok(false);
    }

    if key.code == KeyCode::Char('?')
        && !app.search_mode
        && app.form.is_none()
        && !app.confirm_delete
    {
        app.help_open = true;
        app.help_scroll = 0;
        return Ok(false);
    }

    if app.confirm_delete {
        match key.code {
            KeyCode::Char('y') => {
                if let Some(index) = selected_index(app) {
                    app.vault.entries.remove(index);
                    app.selected = app.selected.saturating_sub(1);
                    app.detail_open = false;
                    app.field_nav = None;
                    app.save()?;
                    app.set_status("Entry deleted.", StatusKind::Success);
                }
                app.confirm_delete = false;
            }
            KeyCode::Char('n') | KeyCode::Esc => app.confirm_delete = false,
            _ => {}
        }
        return Ok(false);
    }

    if app.form.is_some() {
        return handle_form_key(app, key);
    }

    if app.search_mode {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('u') | KeyCode::Char('U'))
        {
            app.search.clear();
            app.search_cursor = 0;
            app.selected = 0;
            return Ok(false);
        }
        match key.code {
            KeyCode::Esc => {
                app.search_mode = false;
            }
            KeyCode::Enter => {
                app.search_mode = false;
            }
            KeyCode::Left => app.search_cursor = app.search_cursor.saturating_sub(1),
            KeyCode::Right => {
                app.search_cursor = (app.search_cursor + 1).min(app.search.chars().count());
            }
            KeyCode::Home => app.search_cursor = 0,
            KeyCode::End => app.search_cursor = app.search.chars().count(),
            KeyCode::Backspace => {
                delete_before(&mut app.search, &mut app.search_cursor);
                app.selected = 0;
            }
            KeyCode::Delete => {
                delete_after(&mut app.search, app.search_cursor);
                app.selected = 0;
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                insert_char(&mut app.search, &mut app.search_cursor, character);
                app.selected = 0;
            }
            _ => {}
        }
        return Ok(false);
    }

    // Overview field navigation
    if let Some(nav) = app.field_nav {
        let field_count = overview_field_count(app);
        match key.code {
            KeyCode::Left | KeyCode::Esc => {
                app.field_nav = None;
                app.set_status("Left overview fields.", StatusKind::Info);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.field_nav = Some(nav.saturating_sub(1));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.field_nav = Some((nav + 1).min(field_count.saturating_sub(1)));
            }
            KeyCode::Char('c') => app.copy_overview_field(),
            KeyCode::Char('q') => return Ok(true),
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Esc => {
            if app.detail_open {
                app.detail_open = false;
                app.field_nav = None;
                app.set_status("Closed entry details.", StatusKind::Info);
                return Ok(false);
            }
            return Ok(true);
        }
        KeyCode::Enter => {
            if selected_entry(app).is_some() {
                app.detail_open = true;
                app.field_nav = None;
                app.set_status("Entry open.", StatusKind::Info);
            }
        }
        KeyCode::Right => {
            if selected_entry(app).is_some() {
                app.detail_open = true;
                app.field_nav = Some(0);
                app.set_status("Navigate fields · c copies selection.", StatusKind::Info);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Char('/') => {
            app.search_mode = true;
            app.search_cursor = app.search.chars().count();
        }
        KeyCode::Char('n') => {
            app.form = Some(FormState::new(None));
        }
        KeyCode::Char('e') => {
            if let Some(entry) = selected_entry(app) {
                app.form = Some(FormState::new(Some(entry)));
            }
        }
        KeyCode::Char('d') => {
            if selected_entry(app).is_some() {
                app.confirm_delete = true;
            }
        }
        KeyCode::Char('c') => app.copy_selected(false),
        KeyCode::Char('o') => app.copy_selected(true),
        _ => {}
    }
    Ok(false)
}

fn handle_form_key(app: &mut App, key: KeyEvent) -> Result<bool, Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Esc => app.form = None,
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => app.save_form()?,
        KeyCode::Char('u') | KeyCode::Char('U')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if let Some(form) = app.form.as_mut() {
                form.active_value_mut().clear();
                form.cursors[form.focus] = 0;
            }
        }
        _ => {
            if let Some(form) = app.form.as_mut() {
                let focus = form.focus;
                match key.code {
                    KeyCode::Tab | KeyCode::Down => {
                        form.focus = (form.focus + 1) % FORM_FIELD_COUNT;
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        form.focus = (form.focus + FORM_FIELD_COUNT - 1) % FORM_FIELD_COUNT;
                    }
                    KeyCode::Left => {
                        form.cursors[focus] = form.cursors[focus].saturating_sub(1);
                    }
                    KeyCode::Right => {
                        let len = form.active_value().chars().count();
                        form.cursors[focus] = (form.cursors[focus] + 1).min(len);
                    }
                    KeyCode::Home => form.cursors[focus] = 0,
                    KeyCode::End => {
                        form.cursors[focus] = form.active_value().chars().count();
                    }
                    KeyCode::Char('x') if form.focus == 4 => {
                        form.reveal_password = !form.reveal_password;
                    }
                    KeyCode::Backspace => {
                        let mut cursor = form.cursors[focus];
                        delete_before(form.active_value_mut(), &mut cursor);
                        form.cursors[focus] = cursor;
                    }
                    KeyCode::Delete => {
                        let cursor = form.cursors[focus];
                        delete_after(form.active_value_mut(), cursor);
                    }
                    KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let mut cursor = form.cursors[focus];
                        insert_char(form.active_value_mut(), &mut cursor, character);
                        form.cursors[focus] = cursor;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(false)
}

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }
    if matches!(app.screen, Screen::Ceremony(_)) || app.help_open || app.confirm_delete {
        return;
    }

    let col = mouse.column;
    let row = mouse.row;

    if let Screen::Unlock { password, cursor } = &mut app.screen {
        if let Some(area) = app.hit.unlock_input {
            if contains(area, col, row) {
                let rel = col.saturating_sub(area.x) as usize;
                *cursor = rel.min(password.chars().count());
            }
        }
        return;
    }

    if app.form.is_some() {
        for (i, area) in app.hit.form_fields.iter().enumerate() {
            if contains(*area, col, row) {
                if let Some(form) = app.form.as_mut() {
                    form.focus = i;
                    let len = form.active_value().chars().count();
                    let rel = col.saturating_sub(area.x.saturating_add(14)) as usize;
                    form.cursors[i] = rel.min(len);
                }
                return;
            }
        }
        return;
    }

    if let Some(area) = app.hit.search_input {
        if contains(area, col, row) {
            app.search_mode = true;
            let rel = col.saturating_sub(area.x.saturating_add(1)) as usize;
            app.search_cursor = rel.min(app.search.chars().count());
            return;
        }
    }

    if let Some(area) = app.hit.list {
        if contains(area, col, row) {
            let rel_y = row.saturating_sub(area.y);
            let index = (rel_y / LIST_ITEM_LINES) as usize;
            let count = visible_indices(app).len();
            if count > 0 && index < count {
                app.selected = index;
                app.field_nav = None;
                app.search_mode = false;
            }
            return;
        }
    }

    if app.detail_open {
        if let Some(area) = app.hit.detail_card {
            if contains(area, col, row) {
                // Enter field-nav and pick nearest field from click row
                let inner_y = row.saturating_sub(area.y);
                let field = (inner_y / 4) as usize;
                let max = overview_field_count(app).saturating_sub(1);
                app.field_nav = Some(field.min(max));
            }
        }
    }
}

fn contains(area: Rect, col: u16, row: u16) -> bool {
    col >= area.x
        && row >= area.y
        && col < area.x.saturating_add(area.width)
        && row < area.y.saturating_add(area.height)
}

impl App {
    fn set_status(&mut self, message: impl Into<String>, kind: StatusKind) {
        self.status = message.into();
        self.status_kind = kind;
    }

    fn lock_for_inactivity(&mut self) {
        if let Some(broker) = self.broker.take() {
            broker.lock();
        }
        self.set_status("Silo locked due to inactivity.", StatusKind::Info);
        self.master = None;
        self.vault = Vault::new();
        self.screen = Screen::Unlock {
            password: String::new(),
            cursor: 0,
        };
        self.form = None;
        self.detail_open = false;
        self.field_nav = None;
        self.confirm_delete = false;
        self.help_open = false;
        self.search_mode = false;
    }

    fn save(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(master) = self.master.as_ref() else {
            return Err("vault is locked".into());
        };
        save_vault(&self.path, &self.vault, master)?;
        Ok(())
    }

    fn save_form(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(form) = self.form.take() else {
            return Ok(());
        };
        if form.title.trim().is_empty() || form.url.trim().is_empty() {
            self.set_status("Name and URL are required.", StatusKind::Error);
            self.form = Some(form);
            return Ok(());
        }
        let is_new = form.id.is_none();
        let name = form.title.clone();
        let username = form.username.clone();
        let email = form.email.clone();
        let url = form.url.clone();
        let has_totp = !form.totp.trim().is_empty();
        if has_totp {
            if let Err(error) = inspect_totp(form.totp.trim()) {
                self.set_status(error.to_string(), StatusKind::Error);
                self.form = Some(form);
                return Ok(());
            }
        }

        if let Some(id) = form.id {
            let Some(entry) = self.vault.entries.iter_mut().find(|entry| entry.id == id) else {
                self.set_status("Entry no longer exists.", StatusKind::Error);
                return Ok(());
            };
            entry.name = form.title;
            entry.url = form.url;
            entry.username = form.username;
            entry.email = form.email;
            entry.password = SecretString::new(form.password.to_string());
            entry.totp_secret =
                (!form.totp.trim().is_empty()).then_some(SecretString::new(form.totp.to_string()));
        } else {
            self.vault.add(new_entry(
                form.title,
                form.url,
                form.username,
                form.email,
                form.password.to_string(),
                (!form.totp.trim().is_empty()).then_some(form.totp.to_string()),
            ));
        }
        self.save()?;

        if is_new {
            self.screen = Screen::Ceremony(Ceremony::Created {
                started: Instant::now(),
                name,
                username,
                email,
                url,
                has_totp,
            });
        } else {
            self.set_status("Changes saved to the encrypted vault.", StatusKind::Success);
        }
        Ok(())
    }

    fn move_selection(&mut self, delta: i32) {
        let length = visible_indices(self).len();
        if length == 0 {
            self.selected = 0;
            return;
        }
        self.selected = if delta < 0 {
            self.selected.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            (self.selected + delta as usize).min(length - 1)
        };
        self.field_nav = None;
    }

    fn copy_selected(&mut self, otp: bool) {
        let Some(entry) = selected_entry(self) else {
            return;
        };
        let value = if otp {
            entry
                .totp_secret
                .as_ref()
                .and_then(|secret| generate_totp(secret.as_str(), now()).ok())
        } else {
            Some(entry.password.as_str().to_string())
        };
        let Some(value) = value else {
            self.set_status("No TOTP is configured for this entry.", StatusKind::Error);
            return;
        };
        clipboard_copy(value);
        self.set_status(
            if otp {
                "TOTP copied. Clipboard clears in 20 seconds."
            } else {
                "Password copied. Clipboard clears in 20 seconds."
            },
            StatusKind::Success,
        );
    }

    fn copy_overview_field(&mut self) {
        let Some(nav) = self.field_nav else {
            return;
        };
        let Some(entry) = selected_entry(self) else {
            return;
        };
        let otp = entry
            .totp_secret
            .as_ref()
            .and_then(|secret| generate_totp(secret.as_str(), now()).ok());
        let (label, value) = match nav {
            0 => ("Username", entry.username.clone()),
            1 => (
                "Email",
                if entry.email.is_empty() {
                    String::new()
                } else {
                    entry.email.clone()
                },
            ),
            2 => ("Password", entry.password.as_str().to_string()),
            3 => ("URL", entry.url.clone()),
            4 => ("OTP code", otp.unwrap_or_default()),
            _ => return,
        };
        if value.is_empty() {
            self.set_status(format!("{label} is empty."), StatusKind::Error);
            return;
        }
        clipboard_copy(value);
        self.set_status(
            format!("{label} copied. Clipboard clears in 20 seconds."),
            StatusKind::Success,
        );
    }
}

impl FormState {
    fn new(entry: Option<&Entry>) -> Self {
        match entry {
            Some(entry) => {
                let title = entry.name.clone();
                let url = entry.url.clone();
                let username = entry.username.clone();
                let email = entry.email.clone();
                let password = entry.password.as_str().to_string();
                let totp = entry
                    .totp_secret
                    .as_ref()
                    .map(|secret| secret.as_str().to_string())
                    .unwrap_or_default();
                let cursors = [
                    title.chars().count(),
                    url.chars().count(),
                    username.chars().count(),
                    email.chars().count(),
                    password.chars().count(),
                    totp.chars().count(),
                ];
                Self {
                    id: Some(entry.id),
                    title,
                    url,
                    username,
                    email,
                    password: Zeroizing::new(password),
                    totp: Zeroizing::new(totp),
                    focus: 0,
                    cursors,
                    reveal_password: false,
                }
            }
            None => Self {
                id: None,
                title: String::new(),
                url: String::new(),
                username: String::new(),
                email: String::new(),
                password: Zeroizing::new(String::new()),
                totp: Zeroizing::new(String::new()),
                focus: 0,
                cursors: [0; FORM_FIELD_COUNT],
                reveal_password: false,
            },
        }
    }

    fn active_value(&self) -> &str {
        match self.focus {
            0 => &self.title,
            1 => &self.url,
            2 => &self.username,
            3 => &self.email,
            4 => &self.password,
            _ => &self.totp,
        }
    }

    fn active_value_mut(&mut self) -> &mut String {
        match self.focus {
            0 => &mut self.title,
            1 => &mut self.url,
            2 => &mut self.username,
            3 => &mut self.email,
            4 => &mut self.password,
            _ => &mut self.totp,
        }
    }
}

fn visible_indices(app: &App) -> Vec<usize> {
    let query = app.search.to_lowercase();
    app.vault
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            query.is_empty()
                || entry.name.to_lowercase().contains(&query)
                || entry.username.to_lowercase().contains(&query)
                || entry.email.to_lowercase().contains(&query)
                || entry.url.to_lowercase().contains(&query)
        })
        .map(|(index, _)| index)
        .collect()
}

fn selected_index(app: &App) -> Option<usize> {
    visible_indices(app).get(app.selected).copied()
}

fn selected_entry(app: &App) -> Option<&Entry> {
    selected_index(app).and_then(|index| app.vault.entries.get(index))
}

fn cursor_on(app: &App) -> bool {
    (app.started.elapsed().as_millis() / 530).is_multiple_of(2)
}

fn overview_field_count(app: &App) -> usize {
    let Some(entry) = selected_entry(app) else {
        return 0;
    };
    if entry.totp_secret.is_some() {
        5
    } else {
        4
    }
}

fn search_wrapped_height(app: &App, width: u16) -> u16 {
    // Inner text width accounts for left/right pad(area, 1, 1).
    let width = width.saturating_sub(2).max(1);
    let len = if app.search.is_empty() && !app.search_mode {
        16u16
    } else {
        (app.search.chars().count() as u16).saturating_add(1)
    };
    // +2 for equal top/bottom padding inside the search surface.
    len.div_ceil(width).clamp(1, 8).saturating_add(2)
}

/// Fixed-width selection gutter so labels never shift when active.
fn selection_gutter(active: bool) -> Span<'static> {
    selection_gutter_on(active, CANVAS)
}

fn selection_gutter_on(active: bool, bg: Color) -> Span<'static> {
    if active {
        Span::styled("▌ ", Style::default().fg(EMERALD).bg(bg))
    } else {
        Span::styled("  ", Style::default().bg(bg))
    }
}

/// Stable caret: invert the character under the cursor, and always reserve a
/// trailing cell so blink and ←/→ never change the line width.
fn caret_line(text: &str, cursor: usize, show_caret: bool, style: Style) -> Line<'static> {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut spans = Vec::new();

    for (i, ch) in chars.iter().enumerate() {
        let on_cursor = i == cursor;
        let cell_style = if on_cursor && show_caret {
            Style::default()
                .fg(CANVAS)
                .bg(EMERALD)
                .add_modifier(Modifier::BOLD)
        } else {
            style
        };
        spans.push(Span::styled(ch.to_string(), cell_style));
    }

    // Trailing cell: caret when at EOL, invisible spacer otherwise — keeps width fixed.
    let end_on = cursor >= chars.len() && show_caret;
    let end_style = if end_on {
        Style::default().fg(CANVAS).bg(EMERALD)
    } else {
        style
    };
    spans.push(Span::styled(" ", end_style));

    Line::from(spans)
}

fn byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn insert_char(s: &mut String, cursor: &mut usize, ch: char) {
    let idx = byte_index(s, *cursor);
    s.insert(idx, ch);
    *cursor += 1;
}

fn delete_before(s: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    *cursor -= 1;
    let idx = byte_index(s, *cursor);
    s.remove(idx);
}

fn delete_after(s: &mut String, cursor: usize) {
    if cursor >= s.chars().count() {
        return;
    }
    let idx = byte_index(s, cursor);
    s.remove(idx);
}

fn clipboard_copy(value: String) {
    thread::spawn(move || {
        if let Ok(mut clipboard) = Clipboard::new() {
            let _ = clipboard.set_text(value.clone());
            thread::sleep(Duration::from_secs(20));
            if clipboard.get_text().ok().as_deref() == Some(value.as_str()) {
                let _ = clipboard.set_text("");
            }
        }
    });
}

fn display_title(name: &str) -> String {
    name.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_url(url: &str) -> String {
    url.trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))
        .unwrap_or(url)
        .to_string()
}

fn format_otp(code: &str) -> String {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 6 {
        format!("{} {}", &digits[..3], &digits[3..])
    } else {
        code.to_string()
    }
}

fn otp_ring(progress: f64) -> String {
    // Depleting ring: full when a period starts, empty near refresh.
    let filled = ((progress.clamp(0.0, 1.0) * 8.0).round() as u8).min(8);
    match filled {
        8 => "●",
        7 | 6 => "◕",
        5 | 4 => "◑",
        3 | 2 => "◔",
        _ => "○",
    }
    .to_string()
}

// --- Visual primitives -------------------------------------------------------

fn hint_spans(pairs: &[(&str, &str)]) -> Vec<Span<'static>> {
    hint_spans_on(pairs, CANVAS)
}

fn hint_spans_on(pairs: &[(&str, &str)], bg: Color) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default().bg(bg)));
        }
        spans.push(Span::styled(
            (*key).to_string(),
            Style::default().fg(INK).bg(bg),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default()
                .fg(FAINT)
                .bg(bg)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    spans
}

fn status_style(kind: StatusKind) -> Style {
    Style::default().fg(match kind {
        StatusKind::Info => MUTED,
        StatusKind::Success => EMERALD,
        StatusKind::Error => CORAL,
    })
}

fn pad(area: Rect, x: u16, y: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(x),
        y: area.y.saturating_add(y),
        width: area.width.saturating_sub(x.saturating_mul(2)),
        height: area.height.saturating_sub(y.saturating_mul(2)),
    }
}

fn inset(area: Rect, x: u16, y: u16) -> Rect {
    pad(area, x, y)
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn now() -> u64 {
    OffsetDateTime::now_utc().unix_timestamp().max(0) as u64
}
