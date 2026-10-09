//! 命令行接口定义。

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "mdp",
    version,
    about = "macOS 多显示器布局管理：存方案、切主屏、一键应用",
    long_about = "macOS 多显示器布局管理。\n\n\
无参数运行会打开交互选择器（方向键选择，回车立即应用）。\n\
方案保存在 ~/.config/mdp/solutions.toml，只记录每块屏的位置与主屏。"
)]
pub struct Cli {
    /// 方案文件路径（默认 ~/.config/mdp/solutions.toml，可用 $MDP_CONFIG 覆盖）
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// 以 JSON 输出，便于脚本处理
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// 把当前显示器布局存成一个方案
    Store {
        /// 方案名
        name: String,
        /// 方案描述（可省略，多个词会用空格连接）
        #[arg(trailing_var_arg = true, value_name = "描述")]
        desc: Vec<String>,
        /// 覆盖同名方案
        #[arg(short, long)]
        force: bool,
    },

    /// 应用一个方案
    #[command(visible_alias = "use")]
    Apply {
        name: String,
        /// 只显示将要发生的改动，不实际应用
        #[arg(long)]
        dry_run: bool,
        /// 只对本次登录会话生效，不写永久配置
        #[arg(long)]
        session: bool,
    },

    /// 列出所有方案（与当前布局一致的会打 *）
    #[command(visible_alias = "ls")]
    List,

    /// 显示当前显示器布局
    Current,

    /// 删除一个方案
    #[command(visible_alias = "rm")]
    Remove {
        name: String,
        #[arg(short, long)]
        yes: bool,
    },

    /// 重命名方案
    Rename { from: String, to: String },

    /// 查看单个方案的详情
    Show { name: String },

    /// 用 $EDITOR 打开方案文件
    Edit {
        /// 可选：先定位到某个方案
        name: Option<String>,
    },

    /// 打印方案文件路径
    Path,
}
