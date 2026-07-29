//! Rust 侧需要翻译的文案。
//!
//! 只放**会流到界面上**的字符串：托盘菜单，以及 hook 事件生成的那几个固定
//! detail（工具摘要如 `Bash: npm test` 本身是语言中立的，不在这里）。
//!
//! 诊断日志和 CLI 输出刻意不翻译 —— 那些是给排查问题的人看的，
//! 一份固定语言反而更容易被搜索和比对。

/// 界面语言。`Lang` 而不是字符串，是为了让 match 穷尽检查帮忙 ——
/// 加语言时编译器会指出所有漏掉的地方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// `"auto"` 时按系统 UI 语言猜。
    pub fn resolve(pref: &str) -> Lang {
        match pref {
            "zh" => Lang::Zh,
            "en" => Lang::En,
            _ => detect_os(),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
        }
    }
}

/// 读系统 UI 语言。只区分「中文」和「其它」—— 我们只有两种界面语言，
/// 把德语用户送去英文界面是对的。
#[cfg(windows)]
fn detect_os() -> Lang {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // LANGID 的低 10 位是主语言 ID；LANG_CHINESE = 0x04
    // SAFETY: 无参数无指针的纯查询调用。
    let langid = unsafe { GetUserDefaultUILanguage() };
    if (langid & 0x3ff) == 0x04 {
        Lang::Zh
    } else {
        Lang::En
    }
}

#[cfg(not(windows))]
fn detect_os() -> Lang {
    match std::env::var("LANG") {
        Ok(v) if v.starts_with("zh") => Lang::Zh,
        _ => Lang::En,
    }
}

macro_rules! strings {
    ($($name:ident => $zh:expr, $en:expr;)*) => {
        $(
            pub fn $name(lang: Lang) -> &'static str {
                match lang {
                    Lang::Zh => $zh,
                    Lang::En => $en,
                }
            }
        )*
    };
}

strings! {
    // hook 事件生成的固定 detail
    thinking        => "思考中",        "Thinking";
    awaiting_reply  => "等你下一句",    "Waiting for you";
    session_started => "会话开始",      "Session started";
    restored        => "恢复的会话",    "Restored session";
    tool_fallback   => "工具",          "tool";

    // gateway 类 agent（Hermes / OpenClaw）的 detail。
    // 它们没有 hook，detail 是探测时现拼的，所以也得走这里 ——
    // 第一版硬编码了中文，英文界面下会混进中文。
    gateway_up      => "gateway 在跑",  "gateway up";
    // 「没有活动会话」这类空状态文案刻意不放这里 —— 那是前端自己渲染的，
    // Rust 侧的 AppView 在没会话时压根不带任何文案。

    // 托盘菜单
    tray_autostart  => "开机自启",           "Start with Windows";
    tray_sound      => "提示音",             "Sound";
    tray_settings   => "设置…",              "Settings…";
    tray_quit       => "退出 Claude Pet",    "Quit Claude Pet";

    // 设置窗口的标题栏。这个由 WebviewWindowBuilder 定，改不了 document.title
    window_settings => "Claude Pet 设置",    "Claude Pet Settings";
}

/// 「N 个 agent 在跑」。带数字所以不能进 `strings!`（那个宏只处理静态串）。
pub fn gateway_active(lang: Lang, n: u64) -> String {
    match lang {
        Lang::Zh => format!("{n} 个 agent 在跑"),
        Lang::En => format!("{n} agent(s) running"),
    }
}
