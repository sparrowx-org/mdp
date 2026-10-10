//! mdp —— macOS 多显示器布局管理。
//!
//! 所有面向用户的文案都是英文，方便在 issue 和终端里直接复制粘贴。

mod apply;
mod cg;
mod cli;
mod model;
mod store;
mod tui;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use serde_json::json;

use cg::DisplayInfo;
use cli::{Cli, Command};
use model::{validate_name, Solution};
use store::Store;

fn main() {
    if let Err(e) = run() {
        eprintln!("mdp: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let ctx = Ctx {
        config: cli.config.clone(),
        json: cli.json,
    };
    // 先 clone 命令，避免 partial move 之后再借用 cli。
    let command = cli.command.clone();
    match command {
        None => cmd_pick(&ctx),
        Some(Command::Store { name, desc, force }) => cmd_store(&ctx, name, desc, force),
        Some(Command::Apply {
            name,
            dry_run,
            session,
        }) => cmd_apply(&ctx, &name, dry_run, session),
        Some(Command::List) => cmd_list(&ctx),
        Some(Command::Current) => cmd_current(&ctx),
        Some(Command::Delete { name }) => cmd_delete(&ctx, &name),
        Some(Command::Rename { from, to }) => cmd_rename(&ctx, &from, &to),
        Some(Command::Show { name }) => cmd_show(&ctx, &name),
        Some(Command::Edit { name }) => cmd_edit(&ctx, name.as_deref()),
        Some(Command::Path) => cmd_path(&ctx),
    }
}

/// 全局选项（与子命令无关的那部分）。
struct Ctx {
    config: Option<std::path::PathBuf>,
    json: bool,
}

/// 读取当前显示器。读不到说明系统层面出了问题，直接报错而不是当作空列表。
fn live() -> Result<Vec<DisplayInfo>> {
    let displays = cg::list_displays()
        .map_err(|e| anyhow!("failed to read display list: {}", cg::cg_error_message(e)))?;
    if displays.is_empty() {
        bail!("no active display found");
    }
    Ok(displays)
}

fn join_desc(parts: &[String]) -> Option<String> {
    let s = parts.join(" ").trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// ---------------------------------------------------------------- 交互选择器

fn cmd_pick(ctx: &Ctx) -> Result<()> {
    let config_path = ctx.config.clone();
    let live = live()?;

    let chosen = tui::run(
        &live,
        || Ok(Store::load(config_path.as_deref())?.all().to_vec()),
        |name| {
            let mut s = Store::load(config_path.as_deref())?;
            s.remove(name);
            s.save()?;
            Ok(Store::load(config_path.as_deref())?.all().to_vec())
        },
    )?;

    let Some(name) = chosen else {
        println!("Cancelled, nothing changed.");
        return Ok(());
    };

    let store = Store::load(ctx.config.as_deref())?;
    let sol = store
        .get(&name)
        .ok_or_else(|| anyhow!("solution {name:?} was deleted while you were choosing"))
        .with_context(|| format!("solution file: {}", store.path().display()))?
        .clone();
    // dry_run = false：选择器里按回车就是要真正应用
    do_apply(ctx, &sol, &live, false, false)
}

// ---------------------------------------------------------------- store

fn cmd_store(ctx: &Ctx, name: String, desc: Vec<String>, force: bool) -> Result<()> {
    validate_name(&name)?;
    let mut store = Store::load(ctx.config.as_deref())?;
    if store.get(&name).is_some() && !force {
        bail!(
            "solution {name:?} already exists, pass --force to overwrite: mdp store {name} --force"
        );
    }
    let live = live()?;
    let solution = Solution::capture(&name, join_desc(&desc), &live);
    let n = solution.displays.len();
    store.put(solution);
    store.save()?;

    if ctx.json {
        let json = json!({
            "action": "store",
            "name": name,
            "path": store.path().display().to_string(),
            "displays": n,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }
    println!(
        "Saved \"{name}\" ({n} displays) → {}",
        store.path().display()
    );
    for d in &live {
        println!(
            "  {}{} ({},{}) {}",
            if d.primary { "★ " } else { "  " },
            d.label(),
            d.x,
            d.y,
            if d.primary { "primary" } else { "" }
        );
    }
    println!("\nRun `mdp` any time to switch back, or `mdp apply {name}`.");
    Ok(())
}

// ---------------------------------------------------------------- apply

fn cmd_apply(ctx: &Ctx, name: &str, dry_run: bool, session: bool) -> Result<()> {
    let store = Store::load(ctx.config.as_deref())?;
    let sol = store
        .get(name)
        .ok_or_else(|| anyhow!("solution {name:?} does not exist. {}", found_hint(&store)))?
        .clone();
    let live = live()?;
    do_apply(ctx, &sol, &live, dry_run, session)
}

fn found_hint(store: &Store) -> String {
    if store.is_empty() {
        "No solutions yet — store one with `mdp store <name>`.".to_string()
    } else {
        format!(
            "Available: {}",
            store
                .all()
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(" / ")
        )
    }
}

fn do_apply(
    ctx: &Ctx,
    sol: &Solution,
    live: &[DisplayInfo],
    dry_run: bool,
    session: bool,
) -> Result<()> {
    let plan = apply::build_plan(sol, live);

    if dry_run {
        if ctx.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "action": "dry-run",
                    "solution": sol.name,
                    "entries": plan.entries.iter().map(|e| json!({
                        "display": e.live.label(),
                        "uuid": e.live.uuid,
                        "match": match e.via { apply::MatchBy::Uuid => "uuid", apply::MatchBy::Fallback => "fallback" },
                        "from": [e.live.x, e.live.y],
                        "to": if e.target.primary { json!([0, 0]) } else { json!([e.target.x, e.target.y]) },
                        "primary": e.target.primary,
                    })).collect::<Vec<_>>(),
                    "notices": plan.notices.iter().map(|n| n.render("!")).collect::<Vec<_>>(),
                }))?
            );
            return Ok(());
        }
        println!(
            "What \"{}\" would change (--dry-run, nothing applied):",
            sol.name
        );
        for e in &plan.entries {
            let to = if e.target.primary {
                (0, 0)
            } else {
                (e.target.x, e.target.y)
            };
            println!(
                "  {}{} {} ({},{}) → ({},{})",
                if e.target.primary { "★ " } else { "  " },
                e.live.label(),
                apply::move_arrow_text((e.live.x, e.live.y), to),
                e.live.x,
                e.live.y,
                to.0,
                to.1
            );
        }
        for n in &plan.notices {
            println!("  {}", n.render("!"));
        }
        return Ok(());
    }

    let result = apply::execute(&plan, !session)?;

    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "action": "apply",
                "solution": sol.name,
                "persisted": !session,
                "skipped": result.skipped,
                "displays": result.rows.iter().map(|r| json!({
                    "display": r.label,
                    "primary": r.primary,
                    "target": [r.target.0, r.target.1],
                    "actual": [r.actual.0, r.actual.1],
                    "drifted": r.drifted(),
                })).collect::<Vec<_>>(),
                "notices": result.notices.iter().map(|n| n.render("!")).collect::<Vec<_>>(),
            }))?
        );
        return Ok(());
    }

    if result.skipped {
        println!("Nothing to configure, layout unchanged.");
        for n in &result.notices {
            println!("  {}", n.render("!"));
        }
        return Ok(());
    }

    println!(
        "Applied \"{}\"{}",
        sol.name,
        if session { " (session only)" } else { "" }
    );
    for r in &result.rows {
        println!(
            "  {} {} ({},{}){}",
            if r.primary { "★ " } else { "  " },
            r.label,
            r.actual.0,
            r.actual.1,
            if r.drifted() {
                format!("  ← wanted ({},{})", r.target.0, r.target.1)
            } else {
                String::new()
            }
        );
    }
    for n in &result.notices {
        println!("  {}", n.render("!"));
    }
    Ok(())
}

// ---------------------------------------------------------------- list / current / show

fn cmd_list(ctx: &Ctx) -> Result<()> {
    let store = Store::load(ctx.config.as_deref())?;
    let live = if store.is_empty() {
        Vec::new()
    } else {
        live().unwrap_or_default()
    };

    if ctx.json {
        let arr: Vec<_> = store
            .all()
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": s.description,
                    "displays": s.displays.len(),
                    "active": s.is_active(&live),
                    "updated_at": s.updated_at,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if store.is_empty() {
        println!("No solutions yet ({})", store.path().display());
        println!("Store one: mdp store <name> [description]");
        return Ok(());
    }

    println!(
        "Solutions (* = currently in use)  {}",
        store.path().display()
    );
    println!();
    for s in store.all() {
        println!(
            "  {}{:<16} {}",
            if s.is_active(&live) { "*" } else { " " },
            s.name,
            s.summary()
        );
        if let Some(d) = &s.description {
            println!("  {:<18}{}", "", d);
        }
    }
    println!("\n`mdp` opens the picker, `mdp apply <name>` applies directly.");
    Ok(())
}

fn cmd_current(ctx: &Ctx) -> Result<()> {
    let live = live()?;
    let store = Store::load(ctx.config.as_deref())?;

    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "displays": live.iter().map(|d| json!({
                    "name": d.name,
                    "uuid": d.uuid,
                    "x": d.x, "y": d.y,
                    "width": d.width, "height": d.height,
                    "pixel_width": d.pixel_width, "pixel_height": d.pixel_height,
                    "refresh": d.refresh,
                    "primary": d.primary,
                    "builtin": d.builtin,
                    "mirroring": d.mirroring,
                })).collect::<Vec<_>>(),
                "active_solution": store.active(&live).map(|s| s.name.clone()),
            }))?
        );
        return Ok(());
    }

    print!("{}", apply::describe_live(&live));
    match store.active(&live) {
        Some(s) => println!("\nCurrent layout matches solution \"{}\".", s.name),
        None => println!("\nCurrent layout does not match any saved solution."),
    }
    Ok(())
}

fn cmd_show(ctx: &Ctx, name: &str) -> Result<()> {
    let store = Store::load(ctx.config.as_deref())?;
    let sol = store
        .get(name)
        .ok_or_else(|| anyhow!("solution {name:?} does not exist. {}", found_hint(&store)))?
        .clone();
    let live = live().unwrap_or_default();

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&sol)?);
        return Ok(());
    }
    println!("Solution \"{}\"", sol.name);
    if let Some(d) = &sol.description {
        println!("Description  {d}");
    }
    println!("Created      {}", sol.created_at);
    println!("Updated      {}", sol.updated_at);
    println!("File         {}", store.path().display());
    println!(
        "Status       {}",
        if sol.is_active(&live) {
            "currently in use"
        } else {
            "not in use"
        }
    );
    println!();
    let mut rows: Vec<_> = sol.displays.iter().collect();
    rows.sort_by_key(|p| (p.y, p.x));
    for p in rows {
        let now = live.iter().find(|d| d.uuid.eq_ignore_ascii_case(&p.uuid));
        println!(
            "  {}{:<26} ({:>5},{:>5})  {}",
            if p.primary { "★ " } else { "  " },
            apply::placement_label(p),
            p.x,
            p.y,
            match now {
                Some(d) => format!("now ({},{})", d.x, d.y),
                None => "not connected".to_string(),
            }
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- 增删改

fn cmd_delete(ctx: &Ctx, name: &str) -> Result<()> {
    let mut store = Store::load(ctx.config.as_deref())?;
    if store.get(name).is_none() {
        bail!("solution {name:?} does not exist. {}", found_hint(&store));
    }
    store.remove(name);
    store.save()?;
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"action": "delete", "name": name}))?
        );
        return Ok(());
    }
    println!("Deleted \"{name}\"");
    Ok(())
}

fn cmd_rename(ctx: &Ctx, from: &str, to: &str) -> Result<()> {
    validate_name(to)?;
    let mut store = Store::load(ctx.config.as_deref())?;
    store
        .rename(from, to)
        .map_err(|e| anyhow!("{e}. {}", found_hint(&store)))?;
    store.save()?;
    println!("{from:?} → {to:?}");
    Ok(())
}

fn cmd_edit(ctx: &Ctx, name: Option<&str>) -> Result<()> {
    let store = Store::load(ctx.config.as_deref())?;
    if !store.path().exists() {
        store.save()?;
    }
    let path = store.path().to_path_buf();
    if let Some(n) = name {
        if store.get(n).is_none() {
            bail!("solution {n:?} does not exist. {}", found_hint(&store));
        }
    }
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("failed to launch editor {editor}"))?;
    if !status.success() {
        bail!("editor exited with status {}", status.code().unwrap_or(-1));
    }
    // 编辑完立刻校验，避免下次运行时才发现 TOML 写坏了
    Store::load(ctx.config.as_deref()).context("the edited file does not parse")?;
    Ok(())
}

fn cmd_path(ctx: &Ctx) -> Result<()> {
    let store = Store::load(ctx.config.as_deref())?;
    println!("{}", store.path().display());
    Ok(())
}
