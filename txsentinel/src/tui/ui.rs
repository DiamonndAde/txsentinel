use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Cell, Gauge, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::lifecycle::{BundleEntry, CommitmentStage};
use crate::tui::app::App;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Root layout
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // title bar
            Constraint::Length(7),  // top panels (slot / tips / leader)
            Constraint::Length(7),  // active bundles
            Constraint::Length(10), // AI reasoning
            Constraint::Min(8),     // lifecycle log
            Constraint::Length(1),  // status bar
        ])
        .split(area);

    draw_title(frame, root[0], app);
    draw_top_panels(frame, root[1], app);
    draw_active_bundles(frame, root[2], app);
    draw_ai_panel(frame, root[3], app);
    draw_lifecycle_log(frame, root[4], app);
    draw_status_bar(frame, root[5], app);
}

fn draw_title(frame: &mut Frame, area: Rect, app: &App) {
    let net_label = if app.is_devnet { "devnet" } else { "mainnet-beta" };
    let net_color = if app.is_devnet { Color::Rgb(100, 255, 100) } else { Color::Rgb(255, 180, 50) };
    let line = Line::from(vec![
        Span::styled(
            " ⚡ TxSentinel — Smart Solana Transaction Stack  ",
            Style::default().fg(Color::Rgb(100, 200, 255)).bold(),
        ),
        Span::styled(format!("[{net_label}]"), Style::default().fg(net_color).bold()),
        Span::styled(
            "  [q] quit  [s] submit  [f] fault  [j/k] scroll AI",
            Style::default().fg(Color::Rgb(160, 160, 200)),
        ),
    ]);
    let title = Paragraph::new(line)
        .style(Style::default().bg(Color::Rgb(15, 15, 35)));
    frame.render_widget(title, area);
}

fn draw_top_panels(frame: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(area);

    // Slot stream panel
    let tps = app.slot_state.current_tps;
    let slot_lines = vec![
        Line::from(vec![
            Span::raw("  Slot  "),
            Span::styled(
                if app.slot_state.current_slot > 0 {
                    format!("{:>13}", app.slot_state.current_slot)
                } else {
                    format!("{:>13}", "connecting...")
                },
                Style::default().fg(Color::Rgb(100, 255, 150)).bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("  TPS   "),
            Span::styled(
                if tps > 0 { format!("{tps:>13}") } else { format!("{:>13}", "—") },
                tps_color(tps),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Load  "),
            Span::styled(
                format!("{:>13}", tps_label(tps)),
                tps_color(tps),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Bundles "),
            Span::styled(
                format!("{:>11}", app.submission_count),
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    let slot_panel = Paragraph::new(slot_lines)
        .block(styled_block("  SLOT STREAM  "))
        .style(Style::default().fg(Color::White));
    frame.render_widget(slot_panel, cols[0]);

    // Tip market panel
    let p = &app.tip_percentiles;
    let tip_lines = vec![
        tip_line("p25", p.p25),
        tip_line("p50", p.p50),
        tip_line("p75", p.p75),
        tip_line("p95", p.p95),
        tip_line("p99", p.p99),
    ];
    let tip_panel = Paragraph::new(tip_lines)
        .block(styled_block("  TIP MARKET (lamports)  "));
    frame.render_widget(tip_panel, cols[1]);

    // Leader window panel
    let jito_ms = app.slots_until_jito * 400;
    let leader_lines = if app.is_devnet {
        vec![
            Line::from(vec![
                Span::raw("  Mode    "),
                Span::styled("RPC Fallback", Style::default().fg(Color::Rgb(100, 255, 100)).bold()),
            ]),
            Line::from(vec![
                Span::raw("  Jito    "),
                Span::styled("mainnet-only", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("  "),
                Span::styled("▶ READY (devnet)", Style::default().fg(Color::Rgb(100, 255, 100)).bold()),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::raw("  Next Jito  "),
                Span::styled(
                    format!("in {} slots", app.slots_until_jito),
                    Style::default().fg(Color::Rgb(255, 200, 50)).bold(),
                ),
            ]),
            Line::from(vec![
                Span::raw("  Est. wait  "),
                Span::styled(
                    format!("{:.1}s", jito_ms as f64 / 1000.0),
                    Style::default().fg(Color::Rgb(255, 200, 50)),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    if app.slots_until_jito == 0 { "▶ SUBMIT NOW" } else { "◷ WAITING..." },
                    if app.slots_until_jito == 0 {
                        Style::default().fg(Color::Green).bold()
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
            ]),
        ]
    };
    let leader_panel = Paragraph::new(leader_lines)
        .block(styled_block("  LEADER WINDOW  "));
    frame.render_widget(leader_panel, cols[2]);
}

fn draw_active_bundles(frame: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec!["#", "Signature", "Stage", "Tip (L)", "Slot", "Latency", "AI Tip", "Baseline"])
        .style(Style::default().fg(Color::Rgb(150, 150, 255)).bold())
        .height(1);

    // Show 5 most recent bundles (in-flight or just completed)
    let recent: Vec<&BundleEntry> = app.recent_log.iter().take(5).collect();

    let rows: Vec<Row> = recent
        .iter()
        .enumerate()
        .map(|(i, &&ref e)| {
            let (stage_str, stage_color) = stage_display(&e.stage);
            let sig_short = &e.signature[..8.min(e.signature.len())];
            let latency = e.total_latency_ms()
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "—".to_string());
            let ai_tip = e.ai_tip_decision
                .map(|t| format!("{t}"))
                .unwrap_or_else(|| "—".to_string());
            let baseline = e.baseline_tip
                .map(|t| format!("{t}"))
                .unwrap_or_else(|| "—".to_string());

            Row::new(vec![
                Cell::from(format!("#{}", i + 1)),
                Cell::from(format!("{sig_short}...")),
                Cell::from(stage_str).style(Style::default().fg(stage_color)),
                Cell::from(format!("{}", e.tip_lamports)),
                Cell::from(format!("{}", e.submitted_slot)),
                Cell::from(latency),
                Cell::from(ai_tip).style(Style::default().fg(Color::Cyan)),
                Cell::from(baseline).style(Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(12),
            Constraint::Length(20),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(styled_block("  ACTIVE BUNDLES  "))
    .column_spacing(1);

    frame.render_widget(table, area);
}

fn draw_ai_panel(frame: &mut Frame, area: Rect, app: &App) {
    let inner_height = area.height.saturating_sub(2) as usize; // subtract borders

    let lines: Vec<Line> = if app.ai_reasoning_lines.is_empty() {
        vec![Line::from(Span::styled(
            "  Waiting for first bundle submission...",
            Style::default().fg(Color::DarkGray).italic(),
        ))]
    } else {
        // Show a window of lines around the scroll position
        let total = app.ai_reasoning_lines.len();
        let start = app.ai_scroll.saturating_sub(inner_height.saturating_sub(1));
        let end = (start + inner_height).min(total);
        app.ai_reasoning_lines[start..end]
            .iter()
            .map(|l| {
                Line::from(Span::styled(
                    format!("  {l}"),
                    Style::default().fg(Color::Rgb(180, 255, 180)),
                ))
            })
            .collect()
    };

    let total = app.ai_reasoning_lines.len();
    let scroll_hint = if total > inner_height {
        format!("  AI AGENT REASONING (DeepSeek)  [line {}/{}  j/k scroll]  ", app.ai_scroll + 1, total)
    } else {
        "  AI AGENT REASONING (DeepSeek)  ".to_string()
    };

    let panel = Paragraph::new(lines)
        .block(styled_block_owned(scroll_hint))
        .wrap(Wrap { trim: false });
    frame.render_widget(panel, area);
}

fn draw_lifecycle_log(frame: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec!["#", "Stage", "Signature", "Slot", "Tip", "→Proc", "→Conf", "→Fin", "Fault"])
        .style(Style::default().fg(Color::Rgb(150, 150, 255)).bold())
        .height(1);

    let rows: Vec<Row> = app
        .recent_log
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (stage_str, stage_color) = stage_display(&e.stage);
            let sig_short = &e.signature[..8.min(e.signature.len())];
            let sub_to_proc = e.submitted_to_processed_ms.map(|ms| format!("{ms}ms")).unwrap_or_else(|| "—".to_string());
            let proc_to_conf = e.processed_to_confirmed_ms.map(|ms| format!("{ms}ms")).unwrap_or_else(|| "—".to_string());
            let conf_to_fin = e.confirmed_to_finalized_ms.map(|ms| format!("{ms}ms")).unwrap_or_else(|| "—".to_string());
            let fault = e.injected_fault.clone().unwrap_or_else(|| "—".to_string());

            Row::new(vec![
                Cell::from(format!("#{}", i + 1)),
                Cell::from(stage_str).style(Style::default().fg(stage_color)),
                Cell::from(format!("{sig_short}...")),
                Cell::from(format!("{}", e.submitted_slot)),
                Cell::from(format!("{}L", e.tip_lamports)),
                Cell::from(sub_to_proc),
                Cell::from(proc_to_conf),
                Cell::from(conf_to_fin),
                Cell::from(fault).style(Style::default().fg(Color::Rgb(255, 100, 100))),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(16),
        ],
    )
    .header(header)
    .block(styled_block("  LIFECYCLE LOG  "))
    .column_spacing(1);

    frame.render_widget(table, area);
}

fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let bar = Paragraph::new(format!(" {}", app.status_message))
        .style(Style::default().bg(Color::Rgb(20, 20, 40)).fg(Color::Gray));
    frame.render_widget(bar, area);
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn styled_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 100)))
        .title(Span::styled(
            title,
            Style::default().fg(Color::Rgb(120, 120, 200)).bold(),
        ))
        .style(Style::default().bg(Color::Rgb(8, 8, 20)))
}

fn styled_block_owned(title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 100)))
        .title(Span::styled(
            title,
            Style::default().fg(Color::Rgb(120, 120, 200)).bold(),
        ))
        .style(Style::default().bg(Color::Rgb(8, 8, 20)))
}

fn tip_line(label: &str, lamports: u64) -> Line<'static> {
    let color = match label {
        "p25" => Color::Rgb(100, 200, 100),
        "p50" => Color::Rgb(150, 220, 100),
        "p75" => Color::Rgb(220, 200, 80),
        "p95" => Color::Rgb(255, 150, 50),
        "p99" => Color::Rgb(255, 80, 80),
        _ => Color::White,
    };
    Line::from(vec![
        Span::raw(format!("  {label:<4} ")),
        Span::styled(
            format!("{lamports:>10} L"),
            Style::default().fg(color).bold(),
        ),
    ])
}

fn stage_display(stage: &CommitmentStage) -> (&'static str, Color) {
    match stage {
        CommitmentStage::Submitted => ("SUBMITTED ", Color::Gray),
        CommitmentStage::Processed => ("PROCESSED ", Color::Rgb(100, 180, 255)),
        CommitmentStage::Confirmed => ("CONFIRMED ", Color::Rgb(100, 255, 150)),
        CommitmentStage::Finalized => ("FINALIZED ✓", Color::Green),
        CommitmentStage::Failed(_) => ("FAILED ✗   ", Color::Red),
    }
}

fn tps_color(tps: u64) -> Style {
    let color = match tps {
        0..=1500 => Color::Rgb(100, 255, 100),
        1501..=3000 => Color::Rgb(255, 220, 50),
        3001..=5000 => Color::Rgb(255, 140, 30),
        _ => Color::Rgb(255, 60, 60),
    };
    Style::default().fg(color).bold()
}

fn tps_label(tps: u64) -> &'static str {
    match tps {
        0..=1500 => "LOW",
        1501..=3000 => "MODERATE",
        3001..=5000 => "HIGH",
        _ => "VERY HIGH",
    }
}
