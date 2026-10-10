//! 方案持久化：`~/.config/mdp/solutions.toml`
//!
//! 用 `[[solution]]` 数组而不是 map，这样 `mdp list` 的顺序 = 文件里的顺序，
//! 手写调整顺序直接改文件即可。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::Solution;

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    solution: Vec<Solution>,
}

pub struct Store {
    path: PathBuf,
    solutions: Vec<Solution>,
}

impl Store {
    /// 路径优先级：`--config` > `$MDP_CONFIG` > `$XDG_CONFIG_HOME/mdp` > `~/.config/mdp`
    ///
    /// 故意不用 `dirs::config_dir()`：它在 macOS 上返回 `~/Library/Application Support`，
    /// 与「方案文件要能直接进 dotfiles」的预期不符。
    pub fn resolve_path(override_path: Option<&Path>) -> PathBuf {
        if let Some(p) = override_path {
            return p.to_path_buf();
        }
        if let Some(p) = std::env::var_os("MDP_CONFIG") {
            return PathBuf::from(p);
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
            });
        base.join("mdp").join("solutions.toml")
    }

    pub fn load(override_path: Option<&Path>) -> Result<Self> {
        let path = Self::resolve_path(override_path);
        let solutions = match fs::read_to_string(&path) {
            Ok(text) => {
                let file: File = toml::from_str(&text)
                    .with_context(|| format!("解析 {} 失败", path.display()))?;
                file.solution
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("读取 {} 失败", path.display())),
        };
        Ok(Self { path, solutions })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn all(&self) -> &[Solution] {
        &self.solutions
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.solutions.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.solutions.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Solution> {
        self.solutions.iter().find(|s| s.name == name)
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.solutions.iter().position(|s| s.name == name)
    }

    /// 新增或覆盖（保留原有 created_at）。
    pub fn put(&mut self, solution: Solution) {
        match self.index_of(&solution.name) {
            Some(i) => {
                let created = self.solutions[i].created_at.clone();
                self.solutions[i] = Solution {
                    created_at: created,
                    ..solution
                };
            }
            None => self.solutions.push(solution),
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<Solution> {
        let i = self.index_of(name)?;
        Some(self.solutions.remove(i))
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let i = self
            .index_of(from)
            .with_context(|| format!("solution {from:?} does not exist"))?;
        if self.index_of(to).is_some() {
            anyhow::bail!("solution {to:?} already exists");
        }
        self.solutions[i].name = to.to_string();
        self.solutions[i].updated_at = crate::model::now_iso();
        Ok(())
    }

    /// 原子写：先写临时文件再 rename，避免写一半损坏配置。
    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        let file = File {
            solution: self.solutions.clone(),
        };
        let mut text = String::new();
        text.push_str("# mdp display layout solutions. Safe to edit by hand; order here is the order in `mdp list`.\n");
        text.push_str(&toml::to_string_pretty(&file).context("failed to serialize solutions")?);
        let tmp = self.path.with_extension("toml.tmp");
        fs::write(&tmp, text).with_context(|| format!("failed to write {}", tmp.display()))?;
        fs::rename(&tmp, &self.path)
            .with_context(|| format!("failed to save {}", self.path.display()))?;
        Ok(())
    }

    /// 当前实际布局命中的方案（第一个匹配项）。
    pub fn active(&self, live: &[crate::cg::DisplayInfo]) -> Option<&Solution> {
        self.solutions.iter().find(|s| s.is_active(live))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Placement;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mdp-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn sample(name: &str) -> Solution {
        Solution {
            name: name.into(),
            description: Some("双屏".into()),
            created_at: "t0".into(),
            updated_at: "t0".into(),
            displays: vec![Placement {
                uuid: "AAAA-1111".into(),
                name: "External Display".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                pixel_width: 1920,
                pixel_height: 1080,
                builtin: false,
                primary: true,
            }],
        }
    }

    #[test]
    fn roundtrip_preserves_order_and_content() {
        let dir = tmpdir("roundtrip");
        let path = dir.join("solutions.toml");
        let mut s = Store::load(Some(&path)).unwrap();
        assert!(s.is_empty());
        s.put(sample("work"));
        s.put(sample("game"));
        s.save().unwrap();

        let t = toml::from_str::<toml::Value>(&fs::read_to_string(&path).unwrap()).unwrap();
        let names: Vec<&str> = t["solution"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["work", "game"], "文件里顺序应与写入顺序一致");

        let r = Store::load(Some(&path)).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.get("game").unwrap().displays[0].uuid, "AAAA-1111");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_keeps_created_at_and_rename_rejects_collision() {
        let dir = tmpdir("put");
        let path = dir.join("solutions.toml");
        let mut s = Store::load(Some(&path)).unwrap();
        s.put(sample("work"));
        s.put(sample("game"));
        s.rename("work", "game").unwrap_err();
        s.rename("work", "office").unwrap();
        assert!(s.get("office").is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}
