use tauri::WebviewWindow;

/// Starts an operating-system window drag only while the primary mouse button
/// is still physically held. On Windows this check and the non-client message
/// happen together in the backend, closing the webview IPC race that can leave
/// the native move loop active after a quick mouseup.
#[tauri::command]
pub fn start_window_drag(window: WebviewWindow) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::{
            Foundation::{HWND, LPARAM, POINT, WPARAM},
            UI::{
                Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, VK_LBUTTON},
                WindowsAndMessaging::{GetCursorPos, SendMessageW, HTCAPTION, WM_NCLBUTTONDOWN},
            },
        };

        // The high bit is set only while the key is currently down. Checking it
        // here (rather than in JavaScript) observes the state at IPC execution.
        if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0 {
            return Ok(false);
        }

        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor) }.map_err(|error| error.to_string())?;

        // WM_NCLBUTTONDOWN expects signed 16-bit screen coordinates packed into
        // LPARAM, matching the native caption-drag message Windows generates.
        let packed_cursor = ((cursor.y as u16 as u32) << 16 | cursor.x as u16 as u32) as isize;

        // Recheck immediately before entering the native move loop. A mouseup
        // that arrived while the cursor position was queried cancels the drag.
        if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0 {
            return Ok(false);
        }

        let hwnd = HWND(window.hwnd().map_err(|error| error.to_string())?.0);
        let _ = unsafe { ReleaseCapture() };
        unsafe {
            SendMessageW(
                hwnd,
                WM_NCLBUTTONDOWN,
                Some(WPARAM(HTCAPTION as usize)),
                Some(LPARAM(packed_cursor)),
            );
        }
        Ok(true)
    }

    #[cfg(not(target_os = "windows"))]
    {
        window
            .start_dragging()
            .map(|_| true)
            .map_err(|error| error.to_string())
    }
}
