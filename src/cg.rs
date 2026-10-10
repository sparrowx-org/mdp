//! CoreGraphics 绑定。
//!
//! 只使用 SDK 头文件里公开声明的 API（`<CoreGraphics/CGDisplayConfiguration.h>`），
//! 没有私有符号，因此不需要辅助功能 / 屏幕录制权限。
//!
//! 关键调用链（一次事务里完成布局切换）：
//! `CGBeginDisplayConfiguration` → 多次 `CGConfigureDisplayOrigin` → `CGCompleteDisplayConfiguration`

use std::ffi::c_void;

pub type CGDirectDisplayID = u32;
pub type CGError = i32;
pub type CFIndex = isize;
pub type CGDisplayConfigRef = *mut c_void;
pub type CGDisplayModeRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

/// 配置作用范围：本次登录会话 / 永久保存。
pub const SCOPE_SESSION: u32 = 1;
pub const SCOPE_PERMANENT: u32 = 2;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut CGDirectDisplayID,
        display_count: *mut u32,
    ) -> CGError;

    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    fn CGDisplayIsMain(display: CGDirectDisplayID) -> u8;
    fn CGDisplayIsBuiltin(display: CGDirectDisplayID) -> u8;
    fn CGDisplayIsAsleep(display: CGDirectDisplayID) -> u8;
    fn CGDisplayIsInMirrorSet(display: CGDirectDisplayID) -> u8;

    fn CGDisplayCopyDisplayMode(display: CGDirectDisplayID) -> CGDisplayModeRef;
    fn CGDisplayModeRelease(mode: CGDisplayModeRef);
    fn CGDisplayModeGetPixelWidth(mode: CGDisplayModeRef) -> usize;
    fn CGDisplayModeGetPixelHeight(mode: CGDisplayModeRef) -> usize;
    fn CGDisplayModeGetRefreshRate(mode: CGDisplayModeRef) -> f64;

    fn CGBeginDisplayConfiguration(config: *mut CGDisplayConfigRef) -> CGError;
    fn CGCancelDisplayConfiguration(config: CGDisplayConfigRef) -> CGError;
    fn CGConfigureDisplayOrigin(
        config: CGDisplayConfigRef,
        display: CGDirectDisplayID,
        x: i32,
        y: i32,
    ) -> CGError;
    fn CGCompleteDisplayConfiguration(config: CGDisplayConfigRef, scope: u32) -> CGError;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFUUIDCreateString(allocator: *const c_void, uuid: *const c_void) -> *const c_void;
    fn CFStringGetLength(string: *const c_void) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> usize;
    fn CFStringGetCString(
        string: *const c_void,
        buffer: *mut u8,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> u8;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    /// 显示器的持久 UUID（EDID 序列号派生），跨重启稳定。返回 +1 引用。
    fn CGDisplayCreateUUIDFromDisplayID(display: CGDirectDisplayID) -> *mut c_void;
}

/// 一块当前处于活动状态的显示器。
#[derive(Clone, Debug)]
pub struct DisplayInfo {
    /// 运行时 ID，**每次开机 / 插拔都会变**，不可作为存储主键。
    pub id: CGDirectDisplayID,
    /// 持久标识，作为存储主键。
    pub uuid: String,
    /// 人类可读名（Apple Silicon 上拿不到 EDID 产品名，只能内建/外接 + 序号）。
    pub name: String,
    pub builtin: bool,
    pub asleep: bool,
    pub mirroring: bool,
    pub primary: bool,
    /// 逻辑坐标（点），全局坐标系原点 = 主屏左上角。
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// 物理像素。
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub refresh: f64,
}

impl DisplayInfo {
    /// 带尺寸的显示名，两块同款屏时用它区分。
    pub fn label(&self) -> String {
        format!("{} · {}x{}", self.name, self.pixel_width, self.pixel_height)
    }
}

pub fn uuid_head(uuid: &str) -> String {
    uuid.chars().take(8).collect()
}

/// 把 CoreFoundation 字符串转成 Rust `String`，按第一个 NUL 截断。
pub unsafe fn cf_string_to_string(s: *const c_void) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let len = CFStringGetLength(s) as usize;
    // +1 是为 NUL 终止符；实际字节数可能小于 CFStringGetMaximumSizeForEncoding 的上限，
    // 所以必须按第一个 NUL 截断——否则会把一串 \0 写进 UUID，导致跨进程匹配失败。
    let cap = CFStringGetMaximumSizeForEncoding(len as CFIndex, K_CF_STRING_ENCODING_UTF8);
    let mut buf = vec![0u8; cap + 1];
    let ok = CFStringGetCString(
        s,
        buf.as_mut_ptr(),
        (cap + 1) as CFIndex,
        K_CF_STRING_ENCODING_UTF8,
    );
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

/// 取显示器的持久 UUID；拿不到时退化成带前缀的显示 ID，保证 key 永不冲突。
unsafe fn display_uuid(id: CGDirectDisplayID) -> String {
    let uuid = CGDisplayCreateUUIDFromDisplayID(id);
    if uuid.is_null() {
        return format!("id-{id}");
    }
    let s = CFUUIDCreateString(std::ptr::null(), uuid);
    let out = cf_string_to_string(s);
    if !s.is_null() {
        CFRelease(s);
    }
    CFRelease(uuid);
    out.unwrap_or_else(|| format!("id-{id}"))
}

/// 列出全部活动显示器，按 display ID 升序，保证顺序稳定。
pub fn list_displays() -> Result<Vec<DisplayInfo>, CGError> {
    unsafe {
        let mut count: u32 = 0;
        let err = CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count);
        if err != 0 {
            return Err(err);
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut ids = vec![0 as CGDirectDisplayID; count as usize];
        let mut got: u32 = 0;
        let err = CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut got);
        if err != 0 {
            return Err(err);
        }
        ids.truncate(got as usize);
        ids.sort_unstable();

        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let rect = CGDisplayBounds(id);
            let mode = CGDisplayCopyDisplayMode(id);
            let (pw, ph, refresh) = if mode.is_null() {
                (0, 0, 0.0)
            } else {
                let v = (
                    CGDisplayModeGetPixelWidth(mode) as u32,
                    CGDisplayModeGetPixelHeight(mode) as u32,
                    CGDisplayModeGetRefreshRate(mode),
                );
                CGDisplayModeRelease(mode);
                v
            };
            out.push(DisplayInfo {
                id,
                uuid: display_uuid(id),
                name: String::new(), // 下面统一命名
                builtin: CGDisplayIsBuiltin(id) != 0,
                asleep: CGDisplayIsAsleep(id) != 0,
                mirroring: CGDisplayIsInMirrorSet(id) != 0,
                primary: CGDisplayIsMain(id) != 0,
                x: rect.origin.x.round() as i32,
                y: rect.origin.y.round() as i32,
                width: rect.size.width.round() as i32,
                height: rect.size.height.round() as i32,
                pixel_width: pw,
                pixel_height: ph,
                refresh,
            });
        }
        assign_names(&mut out);
        Ok(out)
    }
}

/// 同类显示器超过一块时补序号，避免「两块外接屏」重名。
fn assign_names(displays: &mut [DisplayInfo]) {
    for d in displays.iter_mut() {
        d.name = if d.builtin {
            "Built-in Display".to_string()
        } else {
            "External Display".to_string()
        };
    }
    for kind in [true, false] {
        let idx: Vec<usize> = (0..displays.len())
            .filter(|&i| displays[i].builtin == kind)
            .collect();
        if idx.len() > 1 {
            for (n, &i) in idx.iter().enumerate() {
                displays[i].name = format!("{} {}", displays[i].name, n + 1);
            }
        }
    }
}

/// 把一组目标原点写入系统。
///
/// * 主屏必须排在最前且坐标为 `(0, 0)`——macOS 用「原点为 0,0 的那块屏」定义主屏，
///   所以换主屏等价于换原点。
/// * 一次事务提交，中途失败会 `CGCancelDisplayConfiguration` 回滚，不会留下半截布局。
/// * `scope = SCOPE_PERMANENT` 时布局会持久化，重启 / 重新登录仍生效。
pub fn apply_origins(origins: &[(CGDirectDisplayID, i32, i32)], scope: u32) -> Result<(), CGError> {
    unsafe {
        let mut cfg: CGDisplayConfigRef = std::ptr::null_mut();
        let err = CGBeginDisplayConfiguration(&mut cfg);
        if err != 0 {
            return Err(err);
        }
        for &(id, x, y) in origins {
            let err = CGConfigureDisplayOrigin(cfg, id, x, y);
            if err != 0 {
                CGCancelDisplayConfiguration(cfg);
                return Err(err);
            }
        }
        let err = CGCompleteDisplayConfiguration(cfg, scope);
        if err != 0 {
            CGCancelDisplayConfiguration(cfg);
            return Err(err);
        }
        Ok(())
    }
}

/// 等待窗口服务器落地并回读实际位置。返回每块屏的实际原点。
pub fn settle_and_read(timeout_ms: u64) -> Vec<DisplayInfo> {
    let step = std::time::Duration::from_millis(25);
    let mut waited = 0;
    loop {
        if let Ok(d) = list_displays() {
            return d;
        }
        waited += step.as_millis() as u64;
        if waited >= timeout_ms {
            return Vec::new();
        }
        std::thread::sleep(step);
    }
}

/// `<CoreGraphics/CGError.h>` 里的错误码。
pub fn cg_error_message(err: CGError) -> String {
    let name = match err {
        0 => "kCGErrorSuccess",
        1 => "kCGErrorFailure",
        2 => "kCGErrorIllegalConfiguration",
        3 => "kCGErrorInvalidCGDisplay",
        4 => "kCGErrorInvalidConfiguration",
        5 => "kCGErrorDisplayNotConfigured",
        6 => "kCGErrorDisplayRemovabilityRestricted",
        7 => "kCGErrorDisplayNotRemovable",
        9 => "kCGErrorUnknownDisplayMode",
        10 => "kCGErrorNoDisplayList",
        11 => "kCGErrorInvalidParameter",
        12 => "kCGErrorInvalidConnection",
        13 => "kCGErrorExceededResourceLimit",
        14 => "kCGErrorNotPermitted",
        15 => "kCGErrorFailed",
        _ => "unknown CGError",
    };
    format!("{name} ({err})")
}
