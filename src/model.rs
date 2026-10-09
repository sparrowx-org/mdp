//! 方案（solution）的数据模型。

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::cg::{self, DisplayInfo};

/// 一个方案 = 每块屏的位置 + 哪块是主屏。
///
/// 只存位置，不存分辨率 / 刷新率：分辨率切换的风险远大于收益，改分辨率用系统设置更合适。
/// 存分辨率的唯一用途是**回退匹配**（UUID 找不到时用来认出同一块屏）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Solution {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub displays: Vec<Placement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Placement {
    /// 主键：`CGDisplayCreateUUIDFromDisplayID`，跨重启稳定。
    pub uuid: String,
    /// 仅用于回退匹配与显示，不参与主键。
    #[serde(default)]
    pub name: String,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default)]
    pub pixel_width: u32,
    #[serde(default)]
    pub pixel_height: u32,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub primary: bool,
}

impl Placement {
    pub fn size_label(&self) -> String {
        let w = if self.pixel_width > 0 {
            self.pixel_width
        } else {
            self.width as u32
        };
        let h = if self.pixel_height > 0 {
            self.pixel_height
        } else {
            self.height as u32
        };
        if w == 0 && h == 0 {
            "-".to_string()
        } else {
            format!("{w}x{h}")
        }
    }

    /// 从当前真实布局抓取一条记录。
    pub fn capture(d: &DisplayInfo) -> Self {
        Self {
            uuid: d.uuid.clone(),
            name: d.name.clone(),
            x: d.x,
            y: d.y,
            width: d.width,
            height: d.height,
            pixel_width: d.pixel_width,
            pixel_height: d.pixel_height,
            builtin: d.builtin,
            primary: d.primary,
        }
    }
}

impl Solution {
    /// 把当前显示器布局存成一个方案。
    pub fn capture(name: &str, description: Option<String>, live: &[DisplayInfo]) -> Self {
        let now = now_iso();
        Self {
            name: name.to_string(),
            description,
            created_at: now.clone(),
            updated_at: now,
            displays: live.iter().map(Placement::capture).collect(),
        }
    }

    pub fn primary(&self) -> Option<&Placement> {
        self.displays.iter().find(|p| p.primary)
    }

    /// 判断当前实际布局是否与本方案完全一致。
    pub fn is_active(&self, live: &[DisplayInfo]) -> bool {
        if self.displays.len() != live.len() {
            return false;
        }
        live.iter().all(|d| {
            self.displays
                .iter()
                .any(|p| p.uuid.eq_ignore_ascii_case(&d.uuid) && p.x == d.x && p.y == d.y)
        })
    }

    pub fn summary(&self) -> String {
        let primary = self
            .primary()
            .map(|p| {
                if p.name.is_empty() {
                    cg::uuid_head(&p.uuid)
                } else {
                    p.name.clone()
                }
            })
            .unwrap_or_else(|| "无".to_string());
        format!("{} 屏 · 主屏 {}", self.displays.len(), primary)
    }
}

/// 方案名：允许中英文、数字、- _ .，禁止空白与 TOML / shell 敏感字符。
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("方案名不能为空");
    }
    if name.chars().count() > 64 {
        bail!("方案名过长（最多 64 字符）");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| c.is_whitespace() || "[]\"'\\=#.".contains(*c))
    {
        bail!("方案名不能包含空白或字符 {bad:?}（允许中英文、数字、- _ / :）");
    }
    Ok(())
}

pub fn now_iso() -> String {
    use time::macros::format_description;
    use time::OffsetDateTime;
    let t = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    // 不带小数秒：秒级精度足够，文件也更干净
    let fmt = format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
    );
    t.format(&fmt).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(uuid: &str, x: i32, y: i32, primary: bool) -> Placement {
        Placement {
            uuid: uuid.into(),
            name: String::new(),
            x,
            y,
            width: 1920,
            height: 1080,
            pixel_width: 1920,
            pixel_height: 1080,
            builtin: false,
            primary,
        }
    }

    #[test]
    fn is_active_requires_same_layout() {
        let s = Solution {
            name: "w".into(),
            description: None,
            created_at: String::new(),
            updated_at: String::new(),
            displays: vec![p("A", 0, 0, true), p("B", 1920, 0, false)],
        };
        assert!(!s.is_active(&[]), "屏数不同不算命中");
        assert_eq!(s.primary().unwrap().uuid, "A");
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("work").is_ok());
        assert!(validate_name("工作-1").is_ok());
        assert!(validate_name("two words").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("a=b").is_err());
    }
}
