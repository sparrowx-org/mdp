//! 交互式两栏选择器：左列表、右预览，方向键选择、回车立即应用。
//!
//! 只负责「选」，不负责「应用」——退出 raw mode 之后由 main 统一执行，
//! 这样应用失败时终端状态已经恢复干净。

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, queue, style, terminal};
use std::io::{self, Stdout, Write};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

use crate::apply::{self, Plan};
use crate::cg::DisplayInfo;
use crate::model::Solution;

const MIN_COLS: u16 = 58;
const MIN_ROWS: u16 = 12;
const LIST_W: u16 = 26;

/// 按显示宽度截断，超出部分用 … 代替。
fn fit(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

fn pad(s: &str, width: usize) -> String {
    let t = fit(s, width);
    let w = UnicodeWidthStr::width(t.as_str());
    format!("{t}{}", " ".repeat(width.saturating_sub(w)))
}

struct Item {
    solution: Solution,
    plan: Plan,
}

/// 打开选择器。返回被选中的方案名；用户取消则返回 `None`。
pub fn run<F>(live: &[DisplayInfo], config_path: &Path, reload: F) -> Result<Option<String>>
where
    F: Fn() -> Result<Vec<Solution>>,
{
    if !crossterm::tty::IsTty::is_tty(&io::stdout()) || !crossterm::tty::IsTty::is_tty(&io::stdin())
    {
        bail!("当前不是交互式终端。\n  用 `mdp list` 查看方案，`mdp apply <名字>` 直接应用。");
    }

    let mut items = build_items(reload()?);
    if items.is_empty() {
        bail!(
            "还没有任何方案。先用 `mdp store <名字> [描述]` 存一个当前布局。\n  \
             例如：mdp store work \"外接为主屏，笔记本放右边\""
        );
    }

    let mut out = io::stdout();
    terminal::enable_raw_mode().context("进入 raw 模式失败")?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide).context("进入备用屏幕失败")?;

    let result = event_loop(&mut out, &mut items, live, config_path, &reload);

    terminal::disable_raw_mode().ok();
    execute!(
        out,
        terminal::LeaveAlternateScreen,
        cursor::Show,
        terminal::Clear(terminal::ClearType::All)
    )
    .ok();

    result
}

fn build_items(solutions: Vec<Solution>) -> Vec<Item> {
    let live = cg_list();
    solutions
        .into_iter()
        .map(|solution| {
            let plan = apply::build_plan(&solution, &live);
            Item { solution, plan }
        })
        .collect()
}

/// TUI 里只需要布局关系，重读一次成本极低。
fn cg_list() -> Vec<DisplayInfo> {
    crate::cg::list_displays().unwrap_or_default()
}

fn event_loop<F>(
    out: &mut Stdout,
    items: &mut Vec<Item>,
    live: &[DisplayInfo],
    config_path: &Path,
    reload: &F,
) -> Result<Option<String>>
where
    F: Fn() -> Result<Vec<Solution>>,
{
    let mut sel: usize = 0;
    let mut top: usize = 0;
    // 只在状态变化时重绘，否则 250ms 轮询会把终端刷得发烫。
    let mut dirty = true;

    loop {
        if dirty {
            let (cols, rows) = terminal::size().context("读取终端尺寸失败")?;
            if cols < MIN_COLS || rows < MIN_ROWS {
                draw_fallback(out, items, sel, live, config_path, cols, rows)?;
            } else {
                draw(out, items, sel, top, live, config_path, cols, rows)?;
            }
            out.flush().ok();
            dirty = false;
        }
        let ev = event::poll(std::time::Duration::from_millis(250)).unwrap_or(false);
        if !ev {
            continue;
        }
        match event::read().context("读取终端事件失败")? {
            Event::Resize(_, _) => {
                dirty = true;
                continue;
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                dirty = true;
                let last = items.len().saturating_sub(1);
                let rows = terminal::size().map(|(_, r)| r).unwrap_or(24);
                let page = ((rows as usize).saturating_sub(6)).max(1);
                match key_action(key, items.len(), page, &mut sel, &mut top) {
                    Action::Quit => return Ok(None),
                    Action::Apply => return Ok(Some(items[sel].solution.name.clone())),
                    Action::Refresh => match reload() {
                        Ok(sols) => {
                            let keep = items.get(sel).map(|i| i.solution.name.clone());
                            *items = build_items(sols);
                            sel = keep
                                .and_then(|n| items.iter().position(|i| i.solution.name == n))
                                .unwrap_or(0)
                                .min(items.len().saturating_sub(1));
                            top = 0;
                        }
                        Err(e) => {
                            draw_error(out, items, sel, top, live, config_path, &e.to_string())?;
                            event::poll(std::time::Duration::from_secs(2)).ok();
                        }
                    },
                    Action::None => {}
                }
                if sel > last {
                    sel = last;
                }
            }
            _ => continue,
        }
    }
}

enum Action {
    None,
    Quit,
    Apply,
    Refresh,
}

fn key_action(key: KeyEvent, len: usize, page: usize, sel: &mut usize, top: &mut usize) -> Action {
    let last = len.saturating_sub(1);
    match key.code {
        KeyCode::Esc => return Action::Quit,
        KeyCode::Char('c') | KeyCode::Char('C')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            return Action::Quit
        }
        KeyCode::Char('q') => return Action::Quit,
        KeyCode::Char('r') => return Action::Refresh,
        KeyCode::Enter => return Action::Apply,
        KeyCode::Up | KeyCode::Char('k') => *sel = sel.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => *sel = (*sel + 1).min(last),
        // 左右与上下等效——四个方向都能选中，页跳转用 PageUp/PageDown
        KeyCode::Left | KeyCode::Char('h') => *sel = sel.saturating_sub(1),
        KeyCode::Right | KeyCode::Char('l') => *sel = (*sel + 1).min(last),
        KeyCode::PageUp => *sel = sel.saturating_sub(page),
        KeyCode::PageDown => *sel = (*sel + page).min(last),
        KeyCode::Home => *sel = 0,
        KeyCode::End => *sel = last,
        KeyCode::Char(c @ '1'..='9') => {
            let i = c as usize - '1' as usize;
            if i < len {
                *sel = i;
                return Action::Apply;
            }
        }
        _ => {}
    }
    let h = len;
    if *sel < *top {
        *top = *sel;
    } else if *sel >= *top + h {
        *top = *sel + 1 - h;
    }
    Action::None
}

#[allow(clippy::too_many_arguments)]
fn draw(
    out: &mut Stdout,
    items: &[Item],
    sel: usize,
    top: usize,
    live: &[DisplayInfo],
    config_path: &Path,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    use style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};

    let list_w = LIST_W.min(cols.saturating_sub(12));
    let body_h = rows.saturating_sub(3) as usize;
    let width = cols as usize;

    queue!(out, cursor::MoveTo(0, 0))?;

    // 标题行
    let title = pad(" mdp · 选择方案 ", width);
    queue!(
        out,
        SetForegroundColor(Color::Cyan),
        SetAttribute(Attribute::Bold),
        Print(title),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    line_end(out)?;

    // 列表 + 预览
    let visible = body_h.min(items.len().max(1));
    for row in 0..visible {
        let idx = top + row;
        queue!(out, cursor::MoveTo(0, (row + 1) as u16))?;
        let selected = idx < items.len() && idx == sel;
        if idx < items.len() {
            let it = &items[idx];
            let marker = if selected { "▌" } else { " " };
            let num = format!("{:<2}", idx + 1);
            let active = it.solution.is_active(live);
            let flag = if active { "*" } else { " " };
            let right = fit(
                &it.solution.summary(),
                width.saturating_sub(list_w as usize),
            );
            let plain = pad(
                &format!("{marker}{num}{flag} {}", it.solution.name),
                list_w as usize,
            );
            if selected {
                queue!(
                    out,
                    SetAttribute(Attribute::Reverse),
                    Print(plain),
                    SetAttribute(Attribute::Reset)
                )?;
                queue!(
                    out,
                    SetForegroundColor(Color::Cyan),
                    Print(right),
                    ResetColor
                )?;
            } else {
                queue!(out, Print(plain))?;
                queue!(
                    out,
                    SetForegroundColor(Color::DarkGrey),
                    Print(right),
                    ResetColor
                )?;
            }
            clear_line(out, width)?;
        } else {
            queue!(out, Print(pad("", width)))?;
            clear_line(out, width)?;
        }
    }

    // 预览栏
    let preview_x = list_w;
    let preview_w = width.saturating_sub(list_w as usize);
    let it = items.get(sel);
    let mut preview: Vec<String> = Vec::new();
    if let Some(it) = it {
        preview.push(fit(&it.solution.name, preview_w.saturating_sub(1)));
        if let Some(d) = &it.solution.description {
            preview.push(format!("  {}", fit(d, preview_w.saturating_sub(2))));
        }
        preview.push(String::new());
        let rows: Vec<String> = preview_rows(it, live, preview_w.saturating_sub(2));
        preview.extend(rows);
        let mut warn: Vec<String> = Vec::new();
        for n in &it.plan.notices {
            warn.push(format!(
                "  {}",
                fit(&n.render("⚠"), preview_w.saturating_sub(3))
            ));
        }
        if !warn.is_empty() {
            preview.push(String::new());
            preview.extend(warn);
        }
    }
    for row in 0..body_h {
        let y = (row + 1) as u16;
        queue!(out, cursor::MoveTo(preview_x, y))?;
        let text = preview.get(row).cloned().unwrap_or_default();
        let visible_len = if preview_x as usize + UnicodeWidthStr::width(text.as_str()) <= width {
            text
        } else {
            fit(&text, width.saturating_sub(preview_x as usize))
        };
        let cur_w = UnicodeWidthStr::width(visible_len.as_str());
        let raw = preview.get(row);
        let is_warn = raw.is_some_and(|t| t.starts_with("  ⚠"));
        let is_title = row == 0 && raw.is_some();
        if is_warn || is_title {
            let color = if is_warn { Color::Yellow } else { Color::Cyan };
            queue!(
                out,
                SetForegroundColor(color),
                SetAttribute(Attribute::Bold),
                Print(visible_len),
                SetAttribute(Attribute::Reset),
                crossterm::style::ResetColor
            )?;
        } else {
            queue!(out, Print(visible_len))?;
        }
        if cur_w < preview_w {
            queue!(out, Print(" ".repeat(preview_w - cur_w)))?;
        }
    }

    // 分隔线 + 底部提示
    queue!(out, cursor::MoveTo(0, rows - 2))?;
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print(pad(&"─".repeat(width), width)),
        ResetColor
    )?;
    line_end(out)?;

    queue!(out, cursor::MoveTo(0, rows - 1))?;
    let cfg = config_path.display().to_string();
    let hints = fit(" ↑↓←→ 选择  Enter 应用  1-9 直选  r 重载  q 退出 ", width);
    let hints_w = UnicodeWidthStr::width(hints.as_str());
    queue!(
        out,
        SetAttribute(Attribute::Reverse),
        Print(hints.clone()),
        SetAttribute(Attribute::Reset)
    )?;
    if hints_w < width {
        queue!(
            out,
            SetForegroundColor(Color::DarkGrey),
            Print(fit(&format!(" {cfg}"), width - hints_w)),
            ResetColor
        )?;
    }
    line_end(out)?;
    Ok(())
}

/// 预览栏内容：每块屏一行目标位置 + 与现状的位移方向。
fn preview_rows(it: &Item, live: &[DisplayInfo], width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for entry in &it.plan.entries {
        let target = if entry.target.primary {
            (0, 0)
        } else {
            (entry.target.x, entry.target.y)
        };
        let star = if entry.target.primary { "★ " } else { "  " };
        rows.push(fit(&format!("{star}{}", entry.live.label()), width));

        let now = live
            .iter()
            .find(|d| d.id == entry.live.id)
            .map(|d| (d.x, d.y))
            .unwrap_or((entry.live.x, entry.live.y));
        let arrow = apply::move_arrow_text(now, target);
        rows.push(format!(
            "  ({},{}) {} ({},{})",
            target.0, target.1, arrow, now.0, now.1
        ));
    }
    if it.plan.entries.is_empty() {
        rows.push("（方案中的显示器当前都不可用）".to_string());
    }
    rows
}

fn draw_error(
    out: &mut Stdout,
    items: &[Item],
    sel: usize,
    top: usize,
    live: &[DisplayInfo],
    config_path: &Path,
    msg: &str,
) -> io::Result<()> {
    let (cols, rows) = terminal::size()?;
    draw(out, items, sel, top, live, config_path, cols, rows)?;
    use crossterm::style::{Color, Print, SetForegroundColor};
    queue!(
        out,
        cursor::MoveTo(0, rows - 2),
        SetForegroundColor(Color::Red),
        Print(fit(&format!(" ⚠ {msg}"), cols as usize)),
        crossterm::style::ResetColor
    )?;
    Ok(())
}

/// 终端太小时退化为纯列表，避免把布局画乱。
fn draw_fallback(
    out: &mut Stdout,
    items: &[Item],
    sel: usize,
    live: &[DisplayInfo],
    config_path: &Path,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    queue!(out, cursor::MoveTo(0, 0))?;
    let head = " mdp （终端过小，用 ↑↓ + Enter） ";
    queue!(out, crossterm::style::Print(pad(head, cols as usize)))?;
    line_end(out)?;
    for row in 0..(rows.saturating_sub(2) as usize) {
        let idx = row;
        queue!(out, cursor::MoveTo(0, (row + 1) as u16))?;
        if let Some(it) = items.get(idx) {
            let active = if it.solution.is_active(live) {
                "*"
            } else {
                " "
            };
            let text = format!(
                "{} {} {}",
                if idx == sel { "▌" } else { " " },
                active,
                it.solution.name
            );
            queue!(out, crossterm::style::Print(pad(&text, cols as usize)))?;
        }
        line_end(out)?;
    }
    queue!(out, cursor::MoveTo(0, rows - 1))?;
    let cfg = format!(" {}", config_path.display());
    queue!(
        out,
        crossterm::style::Print(pad(&fit(&cfg, cols as usize), cols as usize))
    )?;
    line_end(out)?;
    Ok(())
}

fn line_end(out: &mut Stdout) -> io::Result<()> {
    queue!(out, terminal::Clear(terminal::ClearType::CurrentLine))
}

fn clear_line(out: &mut Stdout, _width: usize) -> io::Result<()> {
    queue!(out, terminal::Clear(terminal::ClearType::CurrentLine))
}
