//! All OS-specific probing lives here, behind three small functions.
//!
//! On non-Windows targets each one is a no-op so the rest of the app compiles
//! and behaves as if the session were unlocked, not quiet, and the cursor
//! unavailable.

/// True when the interactive desktop is not the normal one, i.e. the session is
/// locked (or the secure/UAC desktop is up).
pub fn session_locked() -> bool {
    imp::session_locked()
}

/// True when Windows says notifications should be held back: Focus Assist,
/// Do Not Disturb, a full-screen D3D app, presentation mode or quiet time.
pub fn notifications_suppressed() -> bool {
    imp::notifications_suppressed()
}

/// Global cursor position in physical screen pixels.
pub fn cursor_position() -> Option<(i32, i32)> {
    imp::cursor_position()
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{HANDLE, POINT};
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_READOBJECTS, UOI_NAME,
    };
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_APP, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_QUIET_TIME,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    /// `OpenInputDesktop` fails outright while the workstation is locked; when it
    /// succeeds, a desktop name other than "Default" (e.g. "Winlogon") also means
    /// the user is not looking at their session.
    pub fn session_locked() -> bool {
        // SAFETY: plain Win32 calls; the handle is closed on every path below.
        unsafe {
            let Ok(desktop) = OpenInputDesktop(Default::default(), false, DESKTOP_READOBJECTS)
            else {
                return true;
            };

            let mut buf = [0u16; 256];
            let mut needed = 0u32;
            let ok = GetUserObjectInformationW(
                HANDLE(desktop.0),
                UOI_NAME,
                Some(buf.as_mut_ptr().cast()),
                std::mem::size_of_val(&buf) as u32,
                Some(&mut needed),
            )
            .is_ok();
            let _ = CloseDesktop(desktop);

            if !ok {
                // Could not read the name: assume unlocked rather than nagging
                // about a lock that may not exist.
                return false;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let name = String::from_utf16_lossy(&buf[..len]);
            !name.eq_ignore_ascii_case("Default")
        }
    }

    pub fn notifications_suppressed() -> bool {
        // SAFETY: a single read-only shell query.
        let state = unsafe { SHQueryUserNotificationState() };
        match state {
            Ok(state) => {
                state == QUNS_BUSY
                    || state == QUNS_RUNNING_D3D_FULL_SCREEN
                    || state == QUNS_PRESENTATION_MODE
                    || state == QUNS_QUIET_TIME
                    || state == QUNS_APP
            }
            Err(err) => {
                log::debug!("SHQueryUserNotificationState failed: {err}");
                false
            }
        }
    }

    pub fn cursor_position() -> Option<(i32, i32)> {
        let mut point = POINT::default();
        // SAFETY: `point` is a valid, correctly sized out-parameter.
        match unsafe { GetCursorPos(&mut point) } {
            Ok(()) => Some((point.x, point.y)),
            Err(err) => {
                log::debug!("GetCursorPos failed: {err}");
                None
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn session_locked() -> bool {
        false
    }

    pub fn notifications_suppressed() -> bool {
        false
    }

    pub fn cursor_position() -> Option<(i32, i32)> {
        None
    }
}
