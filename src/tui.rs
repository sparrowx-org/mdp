//! 交互式两栏选择器。
//!
//! **内联渲染**：不进备用屏幕、不清屏，界面就画在当前 shell 提示符下面，
//! 退出后内容留在终端历史里可以往上翻。每一帧用绝对行号就地重绘，
//! 起点行在启动时通过 `cursor::position()` 探测；空间不够时先把屏幕往上顶几行。

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::{cursor, execute, queue, terminal};
use std::io::{self, Stdout, Write};
use unicode_width::UnicodeWidthStr;

use crate::apply::{self, Plan};
use crate::cg::DisplayInfo;
use crate::model::Solution;

/// 标题行 + 按键提示行。
const CHROME_TOP: u16 = 2;
/// 左栏文字宽度。
const LIST_COLS: u16 = 26;
/// 分隔符占 1 列，再留 1 列空隙。
const PREVIEW_GAP: u16 = 2;
/// 预览区至少要的宽度，不够就退化成单列。
const MIN_PREVIEW_W: u16 = 20;
/// 列表最少显示几行，不够说明终端太小。
const MIN_BODY: u16 = 3;
/// 正文最多几行。
const MAX_BODY: u16 = 16;

const HINT: &str = "Use ← → ↑ ↓ h j k l to navigate, Enter applies, d deletes, r reloads, q quits";

// ---------------------------------------------------------------- 文本与样式

#[derive(Clone, Copy, Default, PartialEq)]
struct Style {
    fg: Option<Color>,
    bold: bool,
    reverse: bool,
}

impl Style {
    const PLAIN: Style = Style {
        fg: None,
        bold: false,
        reverse: false,
    };

    const fn fg(fg: Color) -> Self {
        Style {
            fg: Some(fg),
            bold: false,
            reverse: false,
        }
    }

    const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
}

struct Span {
    text: String,
    style: Style,
}

impl Clone for Span {
    fn clone(&self) -> Self {
        Span {
            text: self.text.clone(),
            style: self.style,
        }
    }
}

fn plain(text: impl Into<String>) -> Span {
    Span {
        text: text.into(),
        style: Style::PLAIN,
    }
}

fn styled(text: impl Into<String>, style: Style) -> Span {
    Span {
        text: text.into(),
        style,
    }
}

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

/// 用空格把已写入的内容补到指定显示宽度。
fn pad_spans(spans: &mut Vec<Span>, width: usize) {
    let used = UnicodeWidthStr::width(spans_text(spans).as_str());
    if used < width {
        spans.push(plain(" ".repeat(width - used)));
    }
}

fn spans_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// 在指定绝对行号画一行（画完清到行尾，避免上帧残留）。
fn paint(out: &mut Stdout, row: u16, spans: &[Span], cols: usize) -> io::Result<()> {
    queue!(out, cursor::MoveTo(0, row))?;
    let mut used = 0usize;
    for span in spans {
        if used >= cols {
            break;
        }
        let room = cols - used;
        let w = UnicodeWidthStr::width(span.text.as_str());
        let text = if w > room {
            fit(&span.text, room)
        } else {
            span.text.clone()
        };
        used += UnicodeWidthStr::width(text.as_str());
        let s = span.style;
        if let Some(c) = s.fg {
            queue!(out, SetForegroundColor(c))?;
        }
        if s.bold {
            queue!(out, SetAttribute(Attribute::Bold))?;
        }
        if s.reverse {
            queue!(out, SetAttribute(Attribute::Reverse))?;
        }
        queue!(out, Print(text))?;
        if s.bold || s.reverse {
            queue!(out, SetAttribute(Attribute::Reset))?;
        }
        if s.fg.is_some() {
            queue!(out, ResetColor)?;
        }
    }
    // UntilNewLine（ESC[0K），不是 CurrentLine（ESC[2K）——后者会抹掉刚写的字
    queue!(out, terminal::Clear(terminal::ClearType::UntilNewLine))
}

// ---------------------------------------------------------------- 帧位置

/// 帧在屏幕上的位置。启动时定好，之后只按绝对行号重绘。
struct Frame {
    /// 帧首行的绝对行号。
    base: u16,
    /// 正文区最多能占多少行（屏幕剩余空间）。
    max_body: u16,
    /// 上一帧画到的总高度，用来清理残留。
    height: u16,
    cols: u16,
    rows: u16,
}

impl Frame {
    /// 从当前光标位置开始铺帧。下方空间不够时先把屏幕内容往上顶。
    fn start(out: &mut Stdout, want_body: u16) -> io::Result<Self> {
        let (cols, rows) = terminal::size()?;
        let need = CHROME_TOP + want_body;

        // 按下回车后 shell 已经把光标放到新一行的行首，帧就从那一行开始画；
        // 只有光标不在行首（提示符后面还有残留）时才往下挪一行，否则会覆盖别人的东西。
        let (col, row) = cursor::position().unwrap_or((0, rows.saturating_sub(1)));
        let mut base = if col == 0 { row } else { row + 1 };

        if base + need > rows {
            // 多打几个换行把屏幕往上顶，滚动多少行 base 就减多少行。
            let push = (base + need).saturating_sub(rows);
            for _ in 0..push {
                queue!(out, Print("\n"))?;
            }
            out.flush()?;
            base = base.saturating_sub(push);
        }

        let max_body = rows
            .saturating_sub(base + CHROME_TOP)
            .clamp(MIN_BODY, want_body.max(MIN_BODY));
        Ok(Self {
            base,
            max_body,
            height: CHROME_TOP + max_body,
            cols,
            rows,
        })
    }

    /// 终端尺寸变了就重新算，但帧首行尽量保持不动（否则会跳）。
    fn relayout(&mut self, want_body: u16) {
        let (cols, rows) = terminal::size().unwrap_or((self.cols, self.rows));
        self.cols = cols;
        self.rows = rows;
        if self.base >= rows {
            self.base = rows.saturating_sub(1);
        }
        self.max_body = rows
            .saturating_sub(self.base + CHROME_TOP)
            .clamp(MIN_BODY, want_body.max(MIN_BODY));
        self.height = CHROME_TOP + self.max_body;
    }

    /// 退出后光标该停在哪：帧下一行的行首。
    /// 返回的列固定是 0；行号超出屏幕时返回 None，交给调用方先打个换行让屏幕滚。
    fn cursor_after(&self) -> Option<u16> {
        let r = self.base.saturating_add(self.height);
        if r >= self.rows {
            None
        } else {
            Some(r)
        }
    }
}

// ---------------------------------------------------------------- 条目

struct Item {
    solution: Solution,
    plan: Plan,
    /// 预渲染的预览行，行数决定正文区高度。
    preview: Vec<Vec<Span>>,
}

fn build_items(solutions: Vec<Solution>, live: &[DisplayInfo]) -> Vec<Item> {
    solutions
        .into_iter()
        .map(|solution| {
            let plan = apply::build_plan(&solution, live);
            let mut item = Item {
                solution,
                plan,
                preview: Vec::new(),
            };
            item.preview = preview_for(&item, live, 100);
            item
        })
        .collect()
}

/// 正文区需要多少行。两栏时取大者，单列时预览接在列表下面，所以相加。
fn body_lines(items: &[Item], cols: u16) -> u16 {
    let list = items.len() as u16;
    let preview = items
        .iter()
        .map(|i| i.preview.len() as u16)
        .max()
        .unwrap_or(0);
    let n = if two_col_fits(cols) {
        list.max(preview)
    } else {
        list.saturating_add(preview)
    };
    n.clamp(MIN_BODY, MAX_BODY)
}

fn two_col_fits(cols: u16) -> bool {
    cols as usize >= LIST_COLS as usize + PREVIEW_GAP as usize + MIN_PREVIEW_W as usize
}

// ---------------------------------------------------------------- 入口

/// 打开选择器。返回被选中的方案名；用户取消则返回 `None`。
///
/// `reload` 重新读取方案，`delete` 删掉一个方案并返回删完之后的列表。
pub fn run<R, D>(live: &[DisplayInfo], reload: R, delete: D) -> Result<Option<String>>
where
    R: Fn() -> Result<Vec<Solution>>,
    D: Fn(&str) -> Result<Vec<Solution>>,
{
    if !crossterm::tty::IsTty::is_tty(&io::stdout()) || !crossterm::tty::IsTty::is_tty(&io::stdin())
    {
        bail!(
            "Not an interactive terminal.\n  \
             Use `mdp list` to see solutions and `mdp apply <name>` to apply one."
        );
    }

    let mut items = build_items(reload()?, live);
    if items.is_empty() {
        bail!(
            "No solutions saved yet. Store the current layout first:\n  \
             mdp store <name> [description]"
        );
    }

    let mut out = io::stdout();
    terminal::enable_raw_mode().context("failed to enter raw mode")?;
    execute!(out, cursor::Hide).context("failed to hide cursor")?;

    let want_body = body_lines(&items, terminal::size()?.0);
    let mut frame = match Frame::start(&mut out, want_body) {
        Ok(f) => f,
        Err(e) => {
            terminal::disable_raw_mode().ok();
            execute!(out, cursor::Show).ok();
            return Err(e.into());
        }
    };

    let result = event_loop(&mut out, &mut items, live, &reload, &delete, &mut frame);

    terminal::disable_raw_mode().ok();
    // 帧下面已经有空行就直接停那；屏幕没地方了就打一个换行让它滚上去。
    // 不再多打一个 \n，否则会在帧和下一条命令之间多留一个空行。
    match frame.cursor_after() {
        Some(row) => execute!(out, cursor::MoveTo(0, row)).ok(),
        None => execute!(out, Print("\n")).ok(),
    };
    execute!(out, cursor::Show).ok();

    result
}

fn event_loop<R, D>(
    out: &mut Stdout,
    items: &mut Vec<Item>,
    live: &[DisplayInfo],
    reload: &R,
    delete: &D,
    frame: &mut Frame,
) -> Result<Option<String>>
where
    R: Fn() -> Result<Vec<Solution>>,
    D: Fn(&str) -> Result<Vec<Solution>>,
{
    let mut sel: usize = 0;
    let mut top: usize = 0;
    // 待确认删除的方案名。再按一次 d 才真删。
    let mut pending_delete: Option<String> = None;
    let mut notice: Option<String> = None;
    // 只在状态变化时重绘，否则 250ms 轮询会把终端刷得发烫。
    let mut dirty = true;

    loop {
        if dirty {
            let overlay = Overlay {
                pending_delete: pending_delete.as_deref(),
                notice: notice.as_deref(),
            };
            let h = draw(out, frame, items, sel, top, live, &overlay)?;
            frame.height = h;
            out.flush().ok();
            dirty = false;
        }
        if !event::poll(std::time::Duration::from_millis(250)).unwrap_or(false) {
            continue;
        }
        match event::read().context("failed to read terminal event")? {
            Event::Resize(_, _) => {
                frame.relayout(body_lines(items, frame.cols));
                dirty = true;
                continue;
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                dirty = true;
                notice = None;

                // 待确认状态下，只有再按 d 才执行删除
                if let Some(name) = pending_delete.clone() {
                    if key.code == KeyCode::Char('d') {
                        pending_delete = None;
                        match delete(&name) {
                            Ok(sols) => {
                                let was = sel;
                                *items = build_items(sols, live);
                                frame.relayout(body_lines(items, frame.cols));
                                if items.is_empty() {
                                    return Ok(None);
                                }
                                sel = was.min(items.len() - 1);
                                top = 0;
                                notice = Some(format!("Deleted \"{name}\""));
                            }
                            Err(e) => notice = Some(format!("Delete failed: {e}")),
                        }
                        continue;
                    }
                    pending_delete = None;
                    continue;
                }

                let page = frame.max_body as usize;
                match key_action(
                    key,
                    items.len(),
                    page,
                    &mut sel,
                    &mut top,
                    frame.max_body as usize,
                    items,
                ) {
                    Action::Quit => return Ok(None),
                    Action::Apply => return Ok(Some(items[sel].solution.name.clone())),
                    Action::Reload => match reload() {
                        Ok(sols) => {
                            let keep = items.get(sel).map(|i| i.solution.name.clone());
                            *items = build_items(sols, live);
                            frame.relayout(body_lines(items, frame.cols));
                            sel = keep
                                .and_then(|n| items.iter().position(|i| i.solution.name == n))
                                .unwrap_or(0)
                                .min(items.len().saturating_sub(1));
                            top = 0;
                        }
                        Err(e) => notice = Some(format!("Reload failed: {e}")),
                    },
                    Action::Delete => {
                        pending_delete = Some(items[sel].solution.name.clone());
                    }
                    Action::None => {}
                }
                let last = items.len().saturating_sub(1);
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
    Reload,
    Delete,
}

fn key_action(
    key: KeyEvent,
    len: usize,
    page: usize,
    sel: &mut usize,
    top: &mut usize,
    view: usize,
    items: &[Item],
) -> Action {
    let last = len.saturating_sub(1);
    match key.code {
        KeyCode::Esc => return Action::Quit,
        KeyCode::Char('c') | KeyCode::Char('C')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            return Action::Quit
        }
        KeyCode::Char('q') => return Action::Quit,
        KeyCode::Char('r') => return Action::Reload,
        KeyCode::Char('d') | KeyCode::Delete => {
            if let Some(it) = items.get(*sel) {
                return if it.solution.name.is_empty() {
                    Action::None
                } else {
                    Action::Delete
                };
            }
            return Action::None;
        }
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
    if *sel < *top {
        *top = *sel;
    } else if *sel >= *top + view {
        *top = *sel + 1 - view;
    }
    Action::None
}

// ---------------------------------------------------------------- 绘制

/// 画一帧，返回帧总高度。
fn draw(
    out: &mut Stdout,
    frame: &Frame,
    items: &[Item],
    sel: usize,
    top: usize,
    live: &[DisplayInfo],
    overlay: &Overlay,
) -> io::Result<u16> {
    let cols = frame.cols as usize;
    let base = frame.base;

    paint(out, base, &header_spans(), cols)?;
    paint(out, base + 1, &status_line(cols, overlay), cols)?;

    let preview_x = LIST_COLS as usize + PREVIEW_GAP as usize;
    let two_col = two_col_fits(frame.cols);
    let preview = items.get(sel).map(|i| &i.preview);

    // 行数随选中项变化（描述长短、屏幕块数不同），多出来的行往下长；
    // 单列模式下预览接在列表下面，所以要把两者加起来。
    let list_len = items.len().saturating_sub(top) as u16;
    let preview_len = preview.map(|p| p.len() as u16).unwrap_or(0);
    let wanted = if two_col {
        body_lines(items, frame.cols)
    } else {
        list_len
            .saturating_add(preview_len)
            .clamp(MIN_BODY, MAX_BODY)
    };
    let body_h = (wanted as usize).min(frame.max_body as usize);

    let mut row = CHROME_TOP;
    for i in 0..body_h {
        let idx = top + i;
        let mut spans: Vec<Span> = Vec::new();

        if two_col {
            if let Some(it) = items.get(idx) {
                spans.extend(list_line(
                    it,
                    idx,
                    idx == sel,
                    it.solution.is_active(live),
                    LIST_COLS as usize,
                ));
            }
            pad_spans(&mut spans, LIST_COLS as usize);
            spans.push(styled(
                "│",
                if idx == sel {
                    Style::fg(Color::Cyan)
                } else {
                    Style::fg(Color::DarkGrey)
                },
            ));
            pad_spans(&mut spans, preview_x);
            if let Some(p) = preview {
                spans.extend(p.get(i).cloned().unwrap_or_default());
            }
        } else if i < list_len as usize {
            // 单列：先列完方案，再把选中项的预览接在下面
            if let Some(it) = items.get(top + i) {
                spans.extend(list_line(
                    it,
                    top + i,
                    top + i == sel,
                    it.solution.is_active(live),
                    cols,
                ));
            }
        } else if let Some(p) = preview {
            spans.extend(p.get(i - list_len as usize).cloned().unwrap_or_default());
        }

        paint(out, base + row, &spans, cols)?;
        row += 1;
    }

    // 清掉可能存在的上帧残留
    for extra in row..frame.height {
        queue!(
            out,
            cursor::MoveTo(0, base + extra),
            terminal::Clear(terminal::ClearType::CurrentLine)
        )?;
    }
    Ok(row)
}

/// 第二行上盖住按键提示的东西：删除确认、或一条操作结果。
#[derive(Default)]
struct Overlay<'a> {
    pending_delete: Option<&'a str>,
    notice: Option<&'a str>,
}

/// 第二行：默认是按键提示，删除确认 / 操作结果会顶掉它。
fn status_line(cols: usize, o: &Overlay) -> Vec<Span> {
    if let Some(name) = o.pending_delete {
        return vec![styled(
            fit(
                &format!("Delete \"{name}\"? press d again, any other key to cancel"),
                cols,
            ),
            Style::fg(Color::Red).bold(),
        )];
    }
    if let Some(msg) = o.notice {
        return vec![styled(fit(msg, cols), Style::fg(Color::Green))];
    }
    vec![styled(fit(HINT, cols), Style::fg(Color::DarkGrey))]
}

fn header_spans() -> Vec<Span> {
    let mut v = Vec::new();
    if let Some(name) = crate::cg::computer_name() {
        v.push(styled(name, Style::fg(Color::White).bold()));
        v.push(styled(" : ", Style::fg(Color::DarkGrey)));
    }
    v.push(styled(short_cwd(), Style::fg(Color::Cyan)));
    v.push(styled(" ❯ ", Style::fg(Color::Green).bold()));
    v.push(styled("mdp", Style::fg(Color::Green).bold()));
    v
}

/// 当前目录：家目录下写 `~`，其余只显示最后一段（同 shell 提示符的习惯）。
fn short_cwd() -> String {
    let Ok(cwd) = std::env::current_dir() else {
        return "~".to_string();
    };
    if let Some(home) = dirs::home_dir() {
        if cwd == home {
            return "~".to_string();
        }
        if let Ok(rest) = cwd.strip_prefix(&home) {
            let rest = rest.to_string_lossy().trim_start_matches('/').to_string();
            if let Some(last) = rest.rsplit('/').next().filter(|s| !s.is_empty()) {
                return last.to_string();
            }
        }
    }
    cwd.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "~".to_string())
}

/// 左栏一行：`❯ 1 * name`（`*` 表示该方案就是当前布局）
fn list_line(it: &Item, idx: usize, selected: bool, is_active: bool, width: usize) -> Vec<Span> {
    let marker_style = if selected {
        Style::fg(Color::Green).bold()
    } else {
        Style::fg(Color::DarkGrey)
    };
    let name_style = if selected {
        Style::fg(Color::White).bold()
    } else {
        Style::fg(Color::Grey)
    };
    let w = width.saturating_sub(6);
    vec![
        styled(if selected { "❯ " } else { "  " }, marker_style),
        styled(format!("{:<2}", idx + 1), Style::fg(Color::DarkGrey)),
        styled(
            if is_active { "* " } else { "  " },
            Style::fg(Color::Yellow),
        ),
        styled(fit(&it.solution.name, w), name_style),
    ]
}

/// 右栏内容：方案名 + 每块屏的目标位置，位置有变化时写成 `现在 → 目标`。
fn preview_for(it: &Item, live: &[DisplayInfo], width: usize) -> Vec<Vec<Span>> {
    let mut rows: Vec<Vec<Span>> = Vec::new();
    let w = width.saturating_sub(1);

    let mut title = vec![styled(
        fit(&it.solution.name, w),
        Style::fg(Color::Cyan).bold(),
    )];
    if it.solution.is_active(live) {
        title.push(styled("  ★", Style::fg(Color::Green)));
    }
    rows.push(title);
    if let Some(d) = &it.solution.description {
        rows.push(vec![styled(fit(d, w), Style::fg(Color::DarkGrey))]);
    }
    rows.push(Vec::new());

    for entry in &it.plan.entries {
        let target = if entry.target.primary {
            (0, 0)
        } else {
            (entry.target.x, entry.target.y)
        };
        let star_style = if entry.target.primary {
            Style::fg(Color::Yellow).bold()
        } else {
            Style::fg(Color::DarkGrey)
        };
        rows.push(vec![
            styled(if entry.target.primary { "★ " } else { "  " }, star_style),
            styled(fit(&entry.live.label(), w), Style::fg(Color::White)),
        ]);

        let now = live
            .iter()
            .find(|d| d.id == entry.live.id)
            .map(|d| (d.x, d.y))
            .unwrap_or((entry.live.x, entry.live.y));
        if now == target {
            rows.push(vec![
                plain("    "),
                styled(
                    format!("({:>5},{:>5})", target.0, target.1),
                    Style::fg(Color::DarkGrey),
                ),
            ]);
        } else {
            rows.push(vec![
                plain("    "),
                styled(
                    format!("({:>5},{:>5})", now.0, now.1),
                    Style::fg(Color::DarkGrey),
                ),
                styled(" → ", Style::fg(Color::Green)),
                styled(
                    format!("({:>5},{:>5})", target.0, target.1),
                    Style::fg(Color::Green).bold(),
                ),
            ]);
        }
    }

    if it.plan.entries.is_empty() {
        rows.push(vec![styled(
            fit("(no display in this solution is connected)", w),
            Style::fg(Color::DarkGrey),
        )]);
    }

    if !it.plan.notices.is_empty() {
        rows.push(Vec::new());
        for n in &it.plan.notices {
            rows.push(vec![styled(
                fit(&n.render("!"), w),
                Style::fg(Color::Yellow),
            )]);
        }
    }
    rows
}
