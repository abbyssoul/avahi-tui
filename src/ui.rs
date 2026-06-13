use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::{
    app::{App, AppMode},
    service::{GroupingMode, ServiceRecord},
};

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);

    render_header(frame, app, chunks[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);
    render_services(frame, app, body[0]);
    render_details(frame, app, body[1]);
    render_status(frame, app, chunks[2]);

    match app.mode {
        AppMode::TypeFilter => render_type_filter(frame, app),
        AppMode::Grouping => render_grouping(frame, app),
        AppMode::ActionPicker => render_action_picker(frame, app),
        AppMode::InstancePicker => render_instance_picker(frame, app),
        AppMode::Help => render_help(frame),
        AppMode::Browse | AppMode::Search => {}
    }
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let search = if app.filter.text_query.is_empty() {
        "<none>".to_string()
    } else {
        app.filter.text_query.clone()
    };
    let mode = match app.mode {
        AppMode::Search => "search",
        AppMode::Browse => "browse",
        AppMode::TypeFilter => "type filter",
        AppMode::Grouping => "grouping",
        AppMode::ActionPicker => "actions",
        AppMode::InstancePicker => "instances",
        AppMode::Help => "help",
    };
    let title = format!(
        "domain: {} | mode: {mode} | group: {} | search: {search}",
        app.cli.domain, app.filter.grouping
    );
    frame.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("avahi-tui")),
        area,
    );
}

fn render_services(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items = if app.visible_groups.is_empty() {
        vec![ListItem::new("No services match the active filters")]
    } else {
        app.visible_groups
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let marker = if index == app.selected { "> " } else { "  " };
                let line = format!(
                    "{marker}{}  [{}]  {}",
                    group.label,
                    group.service_type,
                    group.count_label()
                );
                let style = if index == app.selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(line).style(style)
            })
            .collect()
    };

    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title("services")),
        area,
    );
}

fn render_details(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let lines = if let Some(group) = app.visible_groups.get(app.selected) {
        let raw_name = group
            .instances
            .first()
            .map(|record| record.name.as_str())
            .unwrap_or(group.name.as_str());
        let display_name = group
            .instances
            .first()
            .map(ServiceRecord::display_name)
            .unwrap_or_else(|| group.name.clone());
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Group", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(": {}", group.label)),
            ]),
            Line::from(format!("Id: {}", group.id.0)),
            Line::from(format!("Mode: {}", group.mode)),
            Line::from(format!("Name: {display_name}")),
            Line::from(format!("Type: {}", group.service_type)),
            Line::from(format!("Domain: {}", group.domain)),
            Line::from(format!(
                "Host: {}",
                group.hostname.as_deref().unwrap_or("<pending>")
            )),
            Line::from(format!(
                "Port: {}",
                group
                    .port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "<pending>".to_string())
            )),
            Line::from(format!("Instances: {}", group.instances.len())),
            Line::from(format!(
                "Last seen: {}s ago",
                group.last_seen.elapsed().as_secs()
            )),
            Line::from(""),
        ];
        if raw_name != display_name {
            lines.insert(4, Line::from(format!("Raw name: {raw_name}")));
        }
        if !group.txt.is_empty() {
            lines.push(Line::from("TXT:"));
            for (key, value) in &group.txt {
                lines.push(Line::from(format!("  {key}={value}")));
            }
            lines.push(Line::from(""));
        }
        for record in group.instances.iter().take(8) {
            lines.push(Line::from(instance_line(record)));
        }
        if group.instances.len() > 8 {
            lines.push(Line::from(format!(
                "... {} more",
                group.instances.len() - 8
            )));
        }
        lines
    } else {
        vec![Line::from("No service selected")]
    };

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("details")),
        area,
    );
}

fn render_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let type_count = app.filter.enabled_service_types.len();
    let text = format!(
        "{} | {} records | {} rows | {} type filters | / search | t types | g group | enter actions | ? help",
        app.status,
        app.records.len(),
        app.visible_groups.len(),
        type_count
    );
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("status")),
        area,
    );
}

fn render_type_filter(frame: &mut Frame<'_>, app: &App) {
    let service_types = app.service_types();
    let items = if service_types.is_empty() {
        vec![ListItem::new("No service types discovered yet")]
    } else {
        service_types
            .iter()
            .enumerate()
            .map(|(index, service_type)| {
                let enabled = app.filter.enabled_service_types.contains(service_type);
                let marker = if index == app.type_filter_index {
                    "> "
                } else {
                    "  "
                };
                let check = if enabled { "[x]" } else { "[ ]" };
                let style = if index == app.type_filter_index {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(format!("{marker}{check} {service_type}")).style(style)
            })
            .collect()
    };
    render_popup(
        frame,
        "service types",
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("space toggles, esc closes"),
        ),
        60,
        60,
    );
}

fn render_grouping(frame: &mut Frame<'_>, app: &App) {
    let items = GroupingMode::ALL
        .iter()
        .enumerate()
        .map(|(index, mode)| {
            let marker = if index == app.grouping_index {
                "> "
            } else {
                "  "
            };
            let active = if *mode == app.filter.grouping {
                " *"
            } else {
                ""
            };
            let style = if index == app.grouping_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("{marker}{mode}{active}")).style(style)
        })
        .collect::<Vec<_>>();
    render_popup(
        frame,
        "group by",
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("enter selects, esc closes"),
        ),
        50,
        45,
    );
}

fn render_action_picker(frame: &mut Frame<'_>, app: &App) {
    let items = app
        .action_matches
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let marker = if index == app.action_index {
                "> "
            } else {
                "  "
            };
            let needs = if action.needs_instance && action.matching_records.len() > 1 {
                " | choose instance"
            } else {
                ""
            };
            let description = action
                .command
                .action
                .description
                .as_deref()
                .or(action.command.description.as_deref())
                .unwrap_or("");
            let style = if index == app.action_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!(
                "{marker}{} - {}{needs}",
                action.command.name, description
            ))
            .style(style)
        })
        .collect::<Vec<_>>();
    render_popup(
        frame,
        "actions",
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("enter runs, esc closes"),
        ),
        70,
        50,
    );
}

fn render_instance_picker(frame: &mut Frame<'_>, app: &App) {
    let items = app
        .pending_action
        .as_ref()
        .map(|action| {
            action
                .matching_records
                .iter()
                .enumerate()
                .map(|(index, record)| {
                    let marker = if index == app.instance_index {
                        "> "
                    } else {
                        "  "
                    };
                    let style = if index == app.instance_index {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(format!("{marker}{}", instance_line(record))).style(style)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    render_popup(
        frame,
        "select instance",
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("enter runs, esc closes"),
        ),
        75,
        55,
    );
}

fn render_help(frame: &mut Frame<'_>) {
    let lines = vec![
        Line::from("j/down, k/up: move selection"),
        Line::from("enter: run matching action"),
        Line::from("/: fuzzy filter by text"),
        Line::from("t: service type checklist"),
        Line::from("g: grouping selector"),
        Line::from("q: quit"),
        Line::from("esc: close modal or search"),
    ];
    render_popup(
        frame,
        "help",
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL)),
        55,
        45,
    );
}

fn render_popup<W>(frame: &mut Frame<'_>, title: &str, widget: W, width: u16, height: u16)
where
    W: ratatui::widgets::Widget,
{
    let area = centered_rect(width, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().borders(Borders::ALL).title(title), area);
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    frame.render_widget(widget, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn instance_line(record: &ServiceRecord) -> String {
    format!(
        "{} {} host={} addr={} port={}",
        record.display_name(),
        record.service_type,
        record.hostname.as_deref().unwrap_or("<pending>"),
        record
            .address
            .map(|a| a.to_string())
            .unwrap_or_else(|| "<pending>".to_string()),
        record
            .port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "<pending>".to_string())
    )
}
