//! 提示音。
//!
//! 直接 FFI 调 winmm 的 `PlaySoundW`，播放 Windows 声音方案里**已有**的系统音。
//! 所以：不引入任何音频 crate、**零字节**体积增量，而且自动尊重用户在系统
//! 设置里配的声音和音量 —— 嵌一个自带 wav 反而会绕过这些。

/// 可选的声音别名。都验证过在本机注册表里配有实际 wav。
///
/// `Notification.Default` 是 Windows 自己的通知音（`Windows Notify System
/// Generic.wav`）；`SystemNotification` / `SystemAsterisk` / `SystemExclamation`
/// 在实测机器上都指向同一个偏闷的 `Windows Background.wav`，区分度不如前者。
pub const AVAILABLE: &[&str] = &[
    "Notification.Default",
    "Notification.IM",
    "Notification.Reminder",
    "SystemExclamation",
    "SystemAsterisk",
];

pub const DEFAULT_ALIAS: &str = "Notification.Default";

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(
            pszSound: *const u16,
            hmod: *mut std::ffi::c_void,
            fdwSound: u32,
        ) -> i32;
    }

    const SND_SYNC: u32 = 0x0000;
    /// 别名没配声音时保持安静，而不是退回系统蜂鸣 ——
    /// 用户把某个事件设成「无」是明确的意图，不该被我们绕过。
    const SND_NODEFAULT: u32 = 0x0002;
    /// pszSound 是声音方案里的事件名，而不是文件路径
    const SND_ALIAS: u32 = 0x0001_0000;

    /// 在一个用完即弃的线程里同步播放。
    ///
    /// 为什么不用 SND_ASYNC 省掉这个线程：异步模式下 `PlaySoundW` 立即返回，
    /// 而字符串指针的生命周期要求我没法从文档确证（对 SND_ALIAS 大概是调用期间
    /// 就解析完了，但「大概」不足以拿来写 unsafe）。同步播放 + 独立线程把这个
    /// 疑问彻底消掉，代价只是一个短命线程，而提示音本身是稀有事件。
    pub fn play(alias: &str) {
        let alias = alias.to_string();
        std::thread::spawn(move || {
            let wide: Vec<u16> = OsStr::new(&alias).encode_wide().chain(Some(0)).collect();
            // SAFETY: wide 以 NUL 结尾，且在整个同步调用期间都存活。
            unsafe {
                PlaySoundW(
                    wide.as_ptr(),
                    std::ptr::null_mut(),
                    SND_ALIAS | SND_SYNC | SND_NODEFAULT,
                );
            }
        });
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn play(_alias: &str) {}
}

pub use imp::play;
