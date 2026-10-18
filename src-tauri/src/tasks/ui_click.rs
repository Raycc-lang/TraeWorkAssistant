//! UI 坐标点击（F-18 兜底，原 src-python/workbuddy_ui_click.py 的 Rust 移植）。
//! 无 API 可用时的最后手段：驱动鼠标对 WorkBuddy 客户端签到按钮做坐标点击。
//! 红线：仅手动触发、默认关闭（settings.ui_click_enabled）；坐标需用户预配置；
//! 全程零 token 输出；单次执行只点击一次，不做循环连点。
//! 注：windows-sys 的 mouse_event 已标注 deprecated（违反零 warning 红线），
//! 点击统一走等价的 SendInput（注入级 API，行为一致）。
//!
//! F-75 M2-2.5：Windows 专属能力（user32 SendInput）——mac 分支保留接口并
//! 明示错误，前端入口按 platform 标志隐藏。

#[cfg(windows)]
use std::thread::sleep;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use windows_sys::Win32::Foundation::POINT;
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_MOUSE, INPUT_0, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SetCursorPos, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

#[cfg(windows)]
const MOVE_DELAY_MS: u64 = 350; // 移动后停顿，给悬浮反馈留时间
#[cfg(windows)]
const CAPTURE_DELAY_S: u64 = 3; // 取点倒计时
#[cfg(windows)]
const CLICK_GAP_MS: u64 = 50; // 按下与抬起间隔（对齐 python 版 0.05s）

/// 3 秒倒计时后记录当前鼠标坐标（取点模式，对齐 python --capture：
/// 用户把鼠标移到签到按钮上，倒计时结束时读位置）。
pub fn capture_point() -> Result<(i32, i32), String> {
    #[cfg(windows)]
    {
        sleep(Duration::from_secs(CAPTURE_DELAY_S));
        let mut pt = POINT { x: 0, y: 0 };
        let ok = unsafe { GetCursorPos(&mut pt) };
        if ok == 0 {
            return Err("取点失败：GetCursorPos 不可用".to_string());
        }
        Ok((pt.x, pt.y))
    }
    #[cfg(not(windows))]
    {
        Err("UI 坐标点击兜底仅支持 Windows（macOS 请使用 API 签到路径）".to_string())
    }
}

/// 坐标是否落在 Windows 虚拟屏幕范围内（多显示器含主屏左侧负坐标副屏）。
#[cfg(windows)]
fn in_virtual_screen(x: i32, y: i32) -> bool {
    unsafe {
        let vs_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vs_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vs_w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vs_h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        x >= vs_x && y >= vs_y && x < vs_x.saturating_add(vs_w) && y < vs_y.saturating_add(vs_h)
    }
}

/// 移动到坐标并左键单击一次（对齐 python --click）。
/// M5 修复：按 Windows 虚拟屏幕边界校验——原 `x/y <= 0` 一刀切把主屏左侧
/// 显示器的合法负坐标判为非法，左副屏用户取点/点击不可用。
pub fn click_at(x: i32, y: i32) -> Result<(), String> {
    #[cfg(windows)]
    if !in_virtual_screen(x, y) {
        return Err(format!("坐标无效（超出虚拟屏幕范围）: ({x}, {y})"));
    }
    #[cfg(not(windows))]
    let _ = (x, y); // 非 Windows 平台在下方 cfg 分支统一报错，吞掉未用参数
    #[cfg(windows)]
    {
        let moved = unsafe { SetCursorPos(x, y) };
        if moved == 0 {
            return Err("移动鼠标失败：SetCursorPos 返回 0".to_string());
        }
        sleep(Duration::from_millis(MOVE_DELAY_MS));
        send_mouse_flag(MOUSEEVENTF_LEFTDOWN)?;
        sleep(Duration::from_millis(CLICK_GAP_MS));
        send_mouse_flag(MOUSEEVENTF_LEFTUP)?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("UI 坐标点击兜底仅支持 Windows（macOS 请使用 API 签到路径）".to_string())
    }
}

/// 注入单次鼠标事件（down/up 各调一次，等价 python mouse_event 两次调用）。
/// M5 修复：校验 SendInput 返回值——原实现忽略返回值，注入失败静默当成功。
#[cfg(windows)]
fn send_mouse_flag(dw_flags: u32) -> Result<(), String> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: dw_flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let sent = SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
        if sent != 1 {
            return Err(format!("SendInput 注入失败（返回 {sent}，期望 1）"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod ui_click_tests {
    use super::*;

    #[test]
    fn click_rejects_out_of_virtual_screen() {
        // 不触达真实鼠标 API：坐标校验前置失败（两平台共享的校验路径）。
        // M5 修复后校验口径为 Windows 虚拟屏幕边界（左副屏负坐标合法），
        // 取远超虚拟屏的坐标断言被拒
        assert!(click_at(i32::MAX / 2, 100).is_err());
        assert!(click_at(100, i32::MAX / 2).is_err());
        assert!(click_at(i32::MIN / 2, 100).is_err());
        assert!(click_at(-1, -1).is_err());
    }
}
