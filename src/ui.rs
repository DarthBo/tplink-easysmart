//! Rendering. Colours stay within the terminal's 16-colour palette so the app
//! inherits whatever theme the user already runs.

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table, TableState, Wrap,
};

use crate::app::{App, Field, Mode, SettingsForm};
use crate::model::SwitchInfo;
use crate::proto::iface::Interface;

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Min(3),    // table
        Constraint::Length(1), // status
        Constraint::Length(1), // keys
    ])
    .split(frame.area());

    draw_title(frame, chunks[0], app);
    draw_table(
        frame,
        chunks[1],
        &app.switches,
        app.selected,
        app.client.interface(),
        app.busy.is_some(),
    );
    draw_status(frame, chunks[2], app);
    draw_keys(frame, chunks[3], app);

    match &app.mode {
        Mode::Settings(form) => draw_settings(frame, form),
        Mode::Confirm(form) => {
            draw_settings(frame, form);
            draw_confirm(frame, form);
        }
        Mode::Help => draw_help(frame),
        Mode::List => {}
    }
}

fn draw_title(frame: &mut Frame, area: Rect, app: &App) {
    let iface = app.client.interface();
    let left = Span::styled(
        " TP-Link Easy Smart ",
        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
    );
    let right = Span::styled(
        format!(
            " {} {}  {} switch{} ",
            iface.name,
            iface.addr,
            app.switches.len(),
            if app.switches.len() == 1 { "" } else { "es" }
        ),
        Style::default().fg(Color::DarkGray),
    );
    let line = Line::from(vec![left, Span::raw("  "), right]);
    frame.render_widget(Paragraph::new(line), area);
}

pub fn draw_table(
    frame: &mut Frame,
    area: Rect,
    switches: &[SwitchInfo],
    selected: usize,
    iface: &Interface,
    busy: bool,
) {
    if switches.is_empty() {
        let text = if busy {
            "Scanning..."
        } else {
            "No switches found.  Press r to scan again."
        };
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }

    let header = Row::new(
        ["Model", "MAC address", "IP address", "Description"]
            .into_iter()
            .map(|h| Cell::from(h).style(Style::default().add_modifier(Modifier::BOLD))),
    )
    .height(1);

    let rows: Vec<Row> = switches
        .iter()
        .map(|s| {
            // Dim switches we cannot reach over IP -- typically still on the
            // 192.168.0.1 factory default.
            let base = if s.reachable_from(iface) {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Row::new(vec![
                Cell::from(s.model.clone()),
                Cell::from(s.mac_string()),
                Cell::from(s.ip.to_string()),
                Cell::from(s.description.clone()),
            ])
            .style(base)
        })
        .collect();

    // Description is last so it can absorb whatever width is left over; the
    // three identifying columns keep their full width at any terminal size.
    let widths = [
        Constraint::Length(10),
        Constraint::Length(17),
        Constraint::Length(15),
        Constraint::Min(11),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(4)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title(" Discovered switches "),
        )
        .row_highlight_style(
            Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("");

    let mut state = TableState::default().with_selected(Some(selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let line = match (&app.busy, &app.status) {
        (Some(busy), _) => Line::from(Span::styled(
            format!(" {busy}"),
            Style::default().fg(Color::Yellow),
        )),
        (None, Some(status)) => Line::from(Span::styled(
            format!(" {}", status.text),
            Style::default().fg(if status.error { Color::Red } else { Color::Green }),
        )),
        (None, None) => Line::from(""),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_keys(frame: &mut Frame, area: Rect, app: &App) {
    let keys: &[(&str, &str)] = match app.mode {
        Mode::List => &[
            ("↑↓", "select"),
            ("enter", "settings"),
            ("w", "web UI"),
            ("r", "rescan"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Mode::Settings(_) => &[
            ("tab", "next field"),
            ("space", "toggle"),
            ("enter", "apply"),
            ("esc", "cancel"),
        ],
        Mode::Confirm(_) => &[("y", "confirm"), ("n/esc", "cancel")],
        Mode::Help => &[("esc", "close")],
    };

    let mut spans = Vec::new();
    for (key, what) in keys {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::default().fg(Color::Black).bg(Color::DarkGray),
        ));
        spans.push(Span::styled(
            format!(" {what}  "),
            Style::default().fg(Color::DarkGray),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A centred box `width` x `height`, clamped to the frame.
fn centered(frame: &Frame, width: u16, height: u16) -> Rect {
    let area = frame.area();
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

pub fn draw_settings(frame: &mut Frame, form: &SettingsForm) {
    // Three read-only rows, a blank, the editable fields, a blank, the
    // buttons, then one line of note.
    const INFO_ROWS: usize = 3;
    let height = (INFO_ROWS + Field::ROWS) as u16 + 6;
    let area = centered(frame, 66, height);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", form.title))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical(
        std::iter::repeat_n(Constraint::Length(1), INFO_ROWS + 1 + Field::ROWS).chain([
            Constraint::Length(1), // spacer
            Constraint::Length(1), // buttons
            Constraint::Min(0),    // note
        ]),
    )
    .split(inner);

    let label_width = 20;
    let hardware = match form.port_count {
        Some(ports) if !form.hardware.is_empty() => format!("{}  ({ports} ports)", form.hardware),
        _ => form.hardware.clone(),
    };
    for (i, (label, value)) in [
        ("MAC Address", form.mac_display.as_str()),
        ("Hardware Version", hardware.as_str()),
        ("Firmware Version", form.firmware.as_str()),
    ]
    .into_iter()
    .enumerate()
    {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {label:<label_width$}"),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(value.to_string()),
            ])),
            rows[i],
        );
    }

    // Editable fields start after the read-only block and a blank line.
    let field_top = INFO_ROWS + 1;

    for (i, field) in Field::ORDER.iter().take(Field::ROWS).enumerate() {
        let row = rows[field_top + i];
        let focused = form.focus == i;
        let disabled = form.is_disabled(*field);

        let label_style = if disabled {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Gray)
        };
        let value_style = if disabled {
            Style::default().fg(Color::DarkGray)
        } else if focused {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        };

        let value = match form.input(*field) {
            Some(input) => input.display(),
            None => {
                let on = form.toggle(*field);
                format!("[{}] {}", if on { "x" } else { " " }, if on { "enabled" } else { "disabled" })
            }
        };

        let line = Line::from(vec![
            Span::styled(format!(" {:<label_width$}", field.label()), label_style),
            Span::styled(format!("{value:<38}"), value_style),
        ]);
        frame.render_widget(Paragraph::new(line), row);

        // Put the real cursor where typing will land.
        if focused && !disabled {
            if let Some(input) = form.input(*field) {
                let x = row.x + 1 + label_width as u16 + input.cursor as u16;
                if x < row.x + row.width {
                    frame.set_cursor_position((x, row.y));
                }
            }
        }
    }

    // Buttons, so the way to commit the form is visible in the form itself.
    let button = |field: Field| {
        let focused = form.field() == field;
        let style = if focused {
            Style::default()
                .fg(Color::Black)
                .bg(if field == Field::Apply { Color::Green } else { Color::Gray })
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        Span::styled(format!("  {}  ", field.label()), style)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            button(Field::Apply),
            Span::raw("  "),
            button(Field::Cancel),
        ])),
        rows[field_top + Field::ROWS + 1],
    );

    let note = if form.dhcp {
        " DHCP is on, so the address fields are ignored.".to_string()
    } else {
        " Credentials are the switch's own web login.".to_string()
    };
    frame.render_widget(
        Paragraph::new(note).style(Style::default().fg(Color::DarkGray)),
        rows[field_top + Field::ROWS + 2],
    );
}

fn draw_confirm(frame: &mut Frame, form: &SettingsForm) {
    let area = centered(frame, 54, 7);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Apply changes? ")
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let summary = if form.dhcp {
        "DHCP enabled (address assigned by the server)".to_string()
    } else {
        format!("{} / {}", form.ip.value, form.netmask.value)
    };
    let text = vec![
        Line::from(format!("Switch:  {}", form.title)),
        Line::from(format!("Address: {summary}")),
        Line::from("Flash:   saved"),
        Line::from(""),
        Line::from(Span::styled(
            "The switch will apply this immediately.  y to confirm.",
            Style::default().fg(Color::Yellow),
        )),
    ];
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
}

fn draw_help(frame: &mut Frame) {
    let area = centered(frame, 64, 18);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = vec![
        Line::from(Span::styled("Discovery", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  Requests go out as broadcasts, so switches on a foreign"),
        Line::from("  subnet (the 192.168.0.1 factory default) still answer."),
        Line::from("  Those rows are dimmed: reach them by giving them an"),
        Line::from("  address on this subnet first."),
        Line::from(""),
        Line::from(Span::styled("Keys", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  ↑ ↓ / j k   move          g / G    first / last"),
        Line::from("  enter, s    settings      w        open web UI"),
        Line::from("  r           rescan        q        quit"),
        Line::from(""),
        Line::from(Span::styled("Settings", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  Only description and IP settings live here; everything"),
        Line::from("  else is on the switch's web interface (w)."),
        Line::from("  Applying needs the switch login (default admin/admin)."),
    ];
    frame.render_widget(Paragraph::new(text), inner);
}
