//! 方案 → 实际布局：匹配显示器、下发原点、回读校验。

use crate::cg::{self, DisplayInfo};
use crate::model::{Placement, Solution};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MatchBy {
    /// UUID 精确命中——正常路径。
    Uuid,
    /// UUID 对不上，靠「像素尺寸 + 内建/外接」猜出来的。可靠性较低，会给警告。
    Fallback,
}

pub struct Entry {
    pub live: DisplayInfo,
    pub target: Placement,
    pub via: MatchBy,
}

/// 不阻断应用的提示信息。
pub enum Notice {
    Missing {
        uuid: String,
        label: String,
    },
    Ambiguous {
        uuid: String,
        label: String,
        candidates: Vec<String>,
    },
    Extra {
        uuid: String,
        label: String,
    },
    Fallback {
        uuid: String,
        label: String,
        matched: String,
    },
    Drifted {
        label: String,
        target: (i32, i32),
        actual: (i32, i32),
    },
}

impl Notice {
    pub fn render(&self, bullet: &str) -> String {
        match self {
            Notice::Missing { uuid, label } => format!(
                "{bullet} {label} ({}) from this solution is not connected — skipped",
                cg::uuid_head(uuid)
            ),
            Notice::Ambiguous {
                uuid,
                label,
                candidates,
            } => format!(
                "{bullet} Which display is {label} ({})? {} all have the same size — skipped. Re-store the solution to fix it",
                cg::uuid_head(uuid),
                candidates.join(" / ")
            ),
            Notice::Extra { uuid, label } => format!(
                "{bullet} {label} ({}) is not part of this solution — left where it is",
                cg::uuid_head(uuid)
            ),
            Notice::Fallback {
                uuid,
                label,
                matched,
            } => format!(
                "{bullet} {label} ({}) was matched by size to {matched} — different cable or dock? Re-store the solution to fix it",
                cg::uuid_head(uuid)
            ),
            Notice::Drifted {
                label,
                target,
                actual,
            } => format!(
                "{bullet} {label} wanted ({},{}) but ended up at ({},{}) — macOS removes gaps and overlaps on its own",
                target.0, target.1, actual.0, actual.1
            ),
        }
    }
}

pub struct Plan {
    pub entries: Vec<Entry>,
    pub notices: Vec<Notice>,
}

/// 当前布局对应的目标原点：主屏第一条且强制 (0,0)。
///
/// macOS 用「原点为 (0,0) 的屏」定义主屏，所以主屏必须第一个下发，
/// 否则改坐标时全局坐标系还没以新主屏为原点，结果会被偏移。
pub fn target_origins(entries: &[Entry]) -> Vec<(cg::CGDirectDisplayID, i32, i32)> {
    let mut out = Vec::with_capacity(entries.len());
    if let Some(e) = entries.iter().find(|e| e.target.primary) {
        out.push((e.live.id, 0, 0));
    }
    for e in entries.iter().filter(|e| !e.target.primary) {
        out.push((e.live.id, e.target.x, e.target.y));
    }
    out
}

/// 把方案和真实显示器配对。UUID 优先；UUID 对不上时按「像素尺寸 + 内建/外接」回退，
/// 只有唯一候选才敢用，多个候选就报歧义并跳过（宁可不动也不要错位）。
pub fn build_plan(solution: &Solution, live: &[DisplayInfo]) -> Plan {
    let mut taken = vec![false; live.len()];
    let mut entries: Vec<Entry> = Vec::new();
    let mut notices = Vec::new();
    let mut matched_any: Vec<bool> = vec![false; solution.displays.len()];

    // 第一轮：UUID 精确匹配。
    for (pi, p) in solution.displays.iter().enumerate() {
        if let Some(i) =
            (0..live.len()).find(|&i| !taken[i] && live[i].uuid.eq_ignore_ascii_case(&p.uuid))
        {
            taken[i] = true;
            matched_any[pi] = true;
            entries.push(Entry {
                live: live[i].clone(),
                target: p.clone(),
                via: MatchBy::Uuid,
            });
        }
    }

    // 第二轮：尺寸回退。
    for (pi, p) in solution.displays.iter().enumerate() {
        if matched_any[pi] {
            continue;
        }
        let same_geometry: Vec<usize> = (0..live.len())
            .filter(|&i| {
                !taken[i]
                    && live[i].pixel_width == p.pixel_width
                    && live[i].pixel_height == p.pixel_height
            })
            .collect();
        let same_kind: Vec<usize> = same_geometry
            .iter()
            .copied()
            .filter(|&i| live[i].builtin == p.builtin)
            .collect();

        let chosen = match (same_kind.len(), same_geometry.len()) {
            (1, _) => Some(same_kind[0]),
            (0, 1) => Some(same_geometry[0]),
            _ => None,
        };

        match chosen {
            Some(i) => {
                taken[i] = true;
                matched_any[pi] = true;
                notices.push(Notice::Fallback {
                    uuid: p.uuid.clone(),
                    label: placement_label(p),
                    matched: live[i].label(),
                });
                entries.push(Entry {
                    live: live[i].clone(),
                    target: p.clone(),
                    via: MatchBy::Fallback,
                });
            }
            None if same_geometry.len() > 1 => notices.push(Notice::Ambiguous {
                uuid: p.uuid.clone(),
                label: placement_label(p),
                candidates: same_geometry.iter().map(|&i| live[i].label()).collect(),
            }),
            None => notices.push(Notice::Missing {
                uuid: p.uuid.clone(),
                label: placement_label(p),
            }),
        }
    }

    for (i, d) in live.iter().enumerate() {
        if !taken[i] {
            notices.push(Notice::Extra {
                uuid: d.uuid.clone(),
                label: d.label(),
            });
        }
    }

    entries.sort_by_key(|e| (!e.target.primary, e.target.x, e.target.y));

    Plan { entries, notices }
}

pub fn placement_label(p: &Placement) -> String {
    if p.name.is_empty() {
        format!("{} · {}", cg::uuid_head(&p.uuid), p.size_label())
    } else {
        format!("{} · {}", p.name, p.size_label())
    }
}

pub struct AppliedRow {
    pub label: String,
    pub primary: bool,
    pub target: (i32, i32),
    pub actual: (i32, i32),
    pub mirrored: bool,
}

impl AppliedRow {
    pub fn drifted(&self) -> bool {
        self.target != self.actual
    }
}

pub struct ApplyResult {
    pub rows: Vec<AppliedRow>,
    pub notices: Vec<Notice>,
    pub skipped: bool,
}

/// 下发布局。`persist = true` 用 `SCOPE_PERMANENT`，重启后仍生效。
///
/// 失败时不留半截布局：一次事务内任何一条配置出错都会回滚。
pub fn execute(plan: &Plan, persist: bool) -> anyhow::Result<ApplyResult> {
    let mut notices = Vec::new();
    for e in &plan.entries {
        if e.live.mirroring {
            notices.push(Notice::Drifted {
                label: e.live.label(),
                target: (e.target.x, e.target.y),
                actual: (e.live.x, e.live.y),
            });
        }
        if e.live.asleep {
            notices.push(Notice::Missing {
                uuid: e.live.uuid.clone(),
                label: format!("{}（已休眠，未配置）", e.live.label()),
            });
        }
    }

    let targets: Vec<(cg::CGDirectDisplayID, i32, i32)> = plan
        .entries
        .iter()
        .filter(|e| !e.live.asleep)
        .flat_map(|e| target_origins(std::slice::from_ref(e)))
        .collect();

    if targets.is_empty() {
        return Ok(ApplyResult {
            rows: Vec::new(),
            notices,
            skipped: true,
        });
    }

    let scope = if persist {
        cg::SCOPE_PERMANENT
    } else {
        cg::SCOPE_SESSION
    };
    cg::apply_origins(&targets, scope)
        .map_err(|e| anyhow::anyhow!("设置显示布局失败：{}", cg::cg_error_message(e)))?;

    // 窗口服务器落地有延迟，回读实际位置而不是假设成功。
    std::thread::sleep(std::time::Duration::from_millis(120));
    let after = cg::settle_and_read(1000);

    let mut rows = Vec::new();
    for e in &plan.entries {
        let actual = after
            .iter()
            .find(|d| d.id == e.live.id)
            .map(|d| (d.x, d.y))
            .unwrap_or((e.live.x, e.live.y));
        let target = if e.target.primary {
            (0, 0)
        } else {
            (e.target.x, e.target.y)
        };
        let row = AppliedRow {
            label: e.live.label(),
            primary: e.target.primary,
            target,
            actual,
            mirrored: after
                .iter()
                .find(|d| d.id == e.live.id)
                .map(|d| d.mirroring)
                .unwrap_or(false),
        };
        if row.drifted() && !row.mirrored {
            notices.push(Notice::Drifted {
                label: row.label.clone(),
                target,
                actual,
            });
        }
        rows.push(row);
    }

    Ok(ApplyResult {
        rows,
        notices,
        skipped: false,
    })
}

/// 当前实际布局的纯展示文本（`mdp current` 用）。
pub fn describe_live(live: &[DisplayInfo]) -> String {
    let mut s = String::new();
    let mut sorted: Vec<&DisplayInfo> = live.iter().collect();
    sorted.sort_by_key(|d| (d.y, d.x));
    for d in sorted {
        s.push_str(&format!(
            "{}  ({},{})  {}  {}{}\n",
            d.label(),
            d.x,
            d.y,
            size_label(d.pixel_width, d.pixel_height),
            if d.refresh > 0.0 {
                format!("@{:.0}Hz", d.refresh)
            } else {
                String::new()
            },
            if d.primary { "  ★ primary" } else { "" }
        ));
    }
    if live.is_empty() {
        s.push_str("(no active display)\n");
    }
    s
}

/// 现状 → 目标的移动方向，供预览和 --dry-run 展示。
pub fn move_arrow_text(from: (i32, i32), to: (i32, i32)) -> &'static str {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if dx == 0 && dy == 0 {
        "="
    } else if dx.abs() >= dy.abs() {
        if dx > 0 {
            "→"
        } else {
            "←"
        }
    } else if dy > 0 {
        "↓"
    } else {
        "↑"
    }
}

fn size_label(w: u32, h: u32) -> String {
    if w == 0 && h == 0 {
        "-".to_string()
    } else {
        format!("{w}x{h}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(
        uuid: &str,
        x: i32,
        y: i32,
        pw: u32,
        ph: u32,
        builtin: bool,
        primary: bool,
    ) -> DisplayInfo {
        DisplayInfo {
            id: uuid.bytes().map(|b| b as u32).sum::<u32>() % 90 + 1,
            uuid: uuid.into(),
            name: "Ext".into(),
            builtin,
            asleep: false,
            mirroring: false,
            primary,
            x,
            y,
            width: pw as i32,
            height: ph as i32,
            pixel_width: pw,
            pixel_height: ph,
            refresh: 60.0,
        }
    }

    fn sol(displays: Vec<Placement>) -> Solution {
        Solution {
            name: "s".into(),
            description: None,
            created_at: String::new(),
            updated_at: String::new(),
            displays,
        }
    }

    fn place(
        uuid: &str,
        x: i32,
        y: i32,
        pw: u32,
        ph: u32,
        builtin: bool,
        primary: bool,
    ) -> Placement {
        Placement {
            uuid: uuid.into(),
            name: String::new(),
            x,
            y,
            width: pw as i32,
            height: ph as i32,
            pixel_width: pw,
            pixel_height: ph,
            builtin,
            primary,
        }
    }

    #[test]
    fn uuid_match_wins_over_fallback() {
        let s = sol(vec![
            place("A", 0, 0, 1920, 1080, false, true),
            place("B", 1920, 0, 3024, 1964, true, false),
        ]);
        let plan = build_plan(
            &s,
            &[
                live("A", 0, 0, 1920, 1080, false, true),
                live("B", 1920, 0, 3024, 1964, true, false),
            ],
        );
        assert_eq!(plan.entries.len(), 2);
        assert!(plan.entries.iter().all(|e| e.via == MatchBy::Uuid));
        assert!(
            plan.notices.is_empty(),
            "不该有警告: {:?}",
            plan.notices.len()
        );
    }

    #[test]
    fn uuid_mismatch_falls_back_when_size_is_unique() {
        let s = sol(vec![place("OLD", 0, 0, 1920, 1080, false, true)]);
        let plan = build_plan(&s, &[live("NEW", 5, 5, 1920, 1080, false, false)]);
        assert_eq!(plan.entries.len(), 1);
        assert_eq!(plan.entries[0].via, MatchBy::Fallback);
        assert_eq!(plan.entries[0].live.uuid, "NEW");
        assert_eq!(plan.notices.len(), 1, "回退必须警告");
    }

    #[test]
    fn two_identical_displays_are_ambiguous_not_guessed() {
        let s = sol(vec![place("GONE", 0, 0, 1920, 1080, false, true)]);
        let plan = build_plan(
            &s,
            &[
                live("X", 0, 0, 1920, 1080, false, false),
                live("Y", 1920, 0, 1920, 1080, false, false),
            ],
        );
        assert!(plan.entries.is_empty(), "歧义时不能瞎猜");
        assert!(matches!(plan.notices[0], Notice::Ambiguous { .. }));
    }

    #[test]
    fn missing_and_extra_are_reported_separately() {
        let s = sol(vec![place("A", 0, 0, 1920, 1080, false, true)]);
        let plan = build_plan(
            &s,
            &[
                live("A", 0, 0, 1920, 1080, false, true),
                live("Z", 0, 0, 3840, 2160, false, false),
            ],
        );
        assert!(plan.entries.len() == 1);
        assert!(matches!(plan.notices[0], Notice::Extra { .. }));
    }

    #[test]
    fn primary_is_applied_first_at_origin() {
        let s = sol(vec![
            place("A", 0, 0, 1920, 1080, false, true),
            place("B", 1920, 0, 3024, 1964, true, false),
        ]);
        let plan = build_plan(
            &s,
            &[
                live("B", 0, 0, 3024, 1964, true, true),
                live("A", 1920, 0, 1920, 1080, false, false),
            ],
        );
        let origins = target_origins(&plan.entries);
        assert_eq!(origins.len(), 2);
        let (first_id, fx, fy) = origins[0];
        assert_eq!((fx, fy), (0, 0));
        assert_eq!(first_id, plan.entries[0].live.id);
        assert!(plan.entries[0].target.primary);
    }
}
