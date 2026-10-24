//! 托盘图标解析。
//!
//! 配置里的图标名允许两种写法：
//!   ① 自己放进图标主题的名字（例如 `devtray`，文件在
//!      `~/.local/share/icons/hicolor/*/apps/devtray.svg|png`）—— 按路径能找到才算数；
//!   ② 系统主题里本来就有的图标名（例如 `process-stop`）—— 只能在实测白名单里才敢用。
//!
//! 为什么不干脆都信任？因为图标名写错时宿主不会报错，而是画成一个**白圆点**
//! （实测 `dialog-error`/`dialog-information` 在 noctalia 就是这样），
//! 你会以为"托盘坏了"。所以解析不出来就退回实测可辨的兜底图标。

use std::path::PathBuf;

const SIZES: &[&str] = &[
    "scalable", "16x16", "22x22", "24x24", "32x32", "48x48", "64x64", "128x128", "256x256",
];
const EXTS: &[&str] = &["svg", "png"];

pub const DEFAULT_OK: &str = "devtray";
pub const DEFAULT_STARTING: &str = "devtray-starting";
pub const DEFAULT_ERROR: &str = "devtray-error";

/// 兜底：这三个在本机（elementary 主题 + noctalia）截图实测过，一定渲染色且可辨
pub const FALLBACK_OK: &str = "utilities-terminal";
pub const FALLBACK_STARTING: &str = "view-refresh";
pub const FALLBACK_ERROR: &str = "process-stop";

/// 本机实测可辨、可以直接信任的系统主题图标名
pub const KNOWN_THEME: &[&str] = &[
    "utilities-terminal",
    "view-refresh",
    "process-stop",
    "media-playback-start",
    "media-playback-pause",
    "emblem-important",
    "emblem-synchronizing",
    "face-smile",
    "dialog-warning",
];

fn bases() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        PathBuf::from(&home).join(".local/share/icons/hicolor"),
        PathBuf::from("/usr/share/icons/hicolor"),
        PathBuf::from(&home).join(".icons"),
        PathBuf::from("/usr/share/pixmaps"),
    ]
}

/// 图标名是否有对应的真实文件（用户自建图标走这条路）
pub fn exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    for base in bases() {
        for size in SIZES {
            for ext in EXTS {
                if base
                    .join(size)
                    .join("apps")
                    .join(format!("{name}.{ext}"))
                    .is_file()
                {
                    return true;
                }
            }
        }
        // /usr/share/pixmaps 这类目录是平铺的
        for ext in EXTS {
            if base.join(format!("{name}.{ext}")).is_file() {
                return true;
            }
        }
    }
    false
}

/// 配置值 → 真正交给宿主的图标名；解析不出来就退回 fallback
pub fn resolve(configured: &str, fallback: &str) -> String {
    let name = configured.trim();
    if name.is_empty() {
        return fallback.to_string();
    }
    if is_builtin(name) || exists(name) || KNOWN_THEME.contains(&name) {
        return name.to_string();
    }
    fallback.to_string()
}

/// 给 `--check` 用：这个配置值最终会解析成什么
pub fn explain(configured: &str, fallback: &str) -> String {
    let name = configured.trim();
    if name.is_empty() {
        return format!("未配置 → 用兜底 {fallback}");
    }
    if is_builtin(name) {
        return format!("{name}（内置多尺寸图标，无需安装图标主题）");
    }
    if exists(name) {
        return format!("{name}（主题里有这个文件）");
    }
    if KNOWN_THEME.contains(&name) {
        return format!("{name}（实测可辨的系统图标）");
    }
    format!("⚠ 找不到 {name}，会退回 {fallback}（放进 hicolor 主题，或改用实测可辨的名字）")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_name_falls_back() {
        assert_eq!(resolve("no-such-icon-xyz", FALLBACK_OK), FALLBACK_OK);
        assert_eq!(resolve("", FALLBACK_OK), FALLBACK_OK);
        assert_eq!(resolve("process-stop", FALLBACK_OK), "process-stop");
    }

    #[test]
    fn explain_flags_missing() {
        assert!(explain("no-such-icon-xyz", FALLBACK_OK).contains("找不到"));
        assert!(explain("process-stop", FALLBACK_OK).contains("实测可辨"));
    }
}

/// Built-in status artwork is sent directly over D-Bus, independent of theme caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Ready,
    Starting,
    Error,
    Idle,
}

impl Badge {
    pub fn from_state(state: crate::tmux::State) -> Self {
        use crate::tmux::State;
        match state {
            State::Ready | State::Running => Self::Ready,
            State::Starting => Self::Starting,
            State::Exited(Some(code)) if code != 0 => Self::Error,
            _ => Self::Idle,
        }
    }

    pub fn png(self) -> Vec<u8> {
        match self {
            Self::Ready => {
                include_bytes!("../assets/icons/hicolor/24x24/apps/devtray.png").to_vec()
            }
            Self::Starting => {
                include_bytes!("../assets/icons/hicolor/24x24/apps/devtray-starting.png").to_vec()
            }
            Self::Error => {
                include_bytes!("../assets/icons/hicolor/24x24/apps/devtray-error.png").to_vec()
            }
            Self::Idle => {
                include_bytes!("../assets/icons/hicolor/24x24/apps/devtray-idle.png").to_vec()
            }
        }
    }

    pub fn pixmaps(self) -> Vec<ksni::Icon> {
        macro_rules! images {
            ($name:literal) => {
                [
                    (
                        16,
                        include_bytes!(concat!("../assets/icons/argb/", $name, "-16.argb"))
                            .as_slice(),
                    ),
                    (
                        22,
                        include_bytes!(concat!("../assets/icons/argb/", $name, "-22.argb"))
                            .as_slice(),
                    ),
                    (
                        24,
                        include_bytes!(concat!("../assets/icons/argb/", $name, "-24.argb"))
                            .as_slice(),
                    ),
                    (
                        32,
                        include_bytes!(concat!("../assets/icons/argb/", $name, "-32.argb"))
                            .as_slice(),
                    ),
                    (
                        48,
                        include_bytes!(concat!("../assets/icons/argb/", $name, "-48.argb"))
                            .as_slice(),
                    ),
                ]
            };
        }
        let data = match self {
            Self::Ready => images!("devtray"),
            Self::Starting => images!("devtray-starting"),
            Self::Error => images!("devtray-error"),
            Self::Idle => images!("devtray-idle"),
        };
        data.into_iter()
            .map(|(size, data)| ksni::Icon {
                width: size,
                height: size,
                data: data.to_vec(),
            })
            .collect()
    }
}

fn is_builtin(name: &str) -> bool {
    [DEFAULT_OK, DEFAULT_STARTING, DEFAULT_ERROR, "devtray-idle"].contains(&name)
}

#[cfg(test)]
mod payload_tests {
    use super::*;
    #[test]
    fn embedded_pixmaps_have_valid_argb_dimensions_and_visible_pixels() {
        for badge in [Badge::Ready, Badge::Starting, Badge::Error, Badge::Idle] {
            let images = badge.pixmaps();
            assert_eq!(images.len(), 5);
            for image in images {
                assert_eq!(image.data.len(), (image.width * image.height * 4) as usize);
                assert!(image.data.chunks_exact(4).any(|pixel| pixel[0] == 255));
                assert_eq!(image.data[0], 0); // transparent corner, alpha is the first byte
            }
        }
    }
}
