use super::{ForegroundApp, ForegroundAppProvider, PlatformContextError};
use std::ffi::OsString;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, HWND};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

const PROCESS_IMAGE_BUFFER_LEN: usize = 32_768;

#[derive(Debug, Clone, Default)]
pub struct WindowsForegroundAppProvider;

impl WindowsForegroundAppProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ForegroundAppProvider for WindowsForegroundAppProvider {
    fn foreground_app(&mut self) -> Result<ForegroundApp, PlatformContextError> {
        foreground_app()
    }
}

fn foreground_app() -> Result<ForegroundApp, PlatformContextError> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd_is_null(hwnd) {
        return Err(PlatformContextError::Unavailable);
    }

    let process_id = foreground_process_id(hwnd)?;
    let process = ProcessHandle::open(process_id)?;
    let process_image_path = query_process_image_path(process.raw())?;
    let app_id = executable_name(&process_image_path)?;
    let elevated = process_blocks_unelevated_access(process.raw());

    Ok(ForegroundApp {
        app_id,
        window_id: Some(format_hwnd(hwnd)),
        display_name: Some(process_image_path),
        secure_input: false,
        elevated,
    })
}

fn hwnd_is_null(hwnd: HWND) -> bool {
    hwnd as usize == 0
}

fn format_hwnd(hwnd: HWND) -> String {
    format!("0x{:x}", hwnd as usize)
}

fn foreground_process_id(hwnd: HWND) -> Result<u32, PlatformContextError> {
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };

    if thread_id == 0 || process_id == 0 {
        return Err(PlatformContextError::Unavailable);
    }

    Ok(process_id)
}

fn query_process_image_path(process: HANDLE) -> Result<String, PlatformContextError> {
    let mut buffer = vec![0u16; PROCESS_IMAGE_BUFFER_LEN];
    let mut size = buffer.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size)
    };

    if ok == FALSE || size == 0 {
        return Err(PlatformContextError::CommandFailed);
    }

    wide_to_string(&buffer[..size as usize])
}

fn wide_to_string(wide: &[u16]) -> Result<String, PlatformContextError> {
    let value = OsString::from_wide(wide).to_string_lossy().into_owned();
    if value.trim().is_empty() {
        return Err(PlatformContextError::InvalidOutput);
    }

    Ok(value)
}

fn executable_name(process_image_path: &str) -> Result<String, PlatformContextError> {
    Path::new(process_image_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .ok_or(PlatformContextError::InvalidOutput)
}

fn process_blocks_unelevated_access(process: HANDLE) -> bool {
    let target_elevated = is_process_elevated(process).unwrap_or(true);
    if !target_elevated {
        return false;
    }

    let current_elevated = current_process_elevated().unwrap_or(false);
    target_elevated && !current_elevated
}

fn current_process_elevated() -> Result<bool, PlatformContextError> {
    let process = unsafe { GetCurrentProcess() };
    is_process_elevated(process)
}

fn is_process_elevated(process: HANDLE) -> Result<bool, PlatformContextError> {
    let token = TokenHandle::open(process)?;
    let mut elevation = unsafe { zeroed::<TOKEN_ELEVATION>() };
    let mut return_length = 0;

    let ok = unsafe {
        GetTokenInformation(
            token.raw(),
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut _,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        )
    };

    if ok == FALSE || return_length == 0 {
        return Err(PlatformContextError::CommandFailed);
    }

    Ok(elevation.TokenIsElevated != 0)
}

#[derive(Debug)]
struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(process_id: u32) -> Result<Self, PlatformContextError> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, process_id) };
        if handle.is_null() {
            return Err(PlatformContextError::Unavailable);
        }

        Ok(Self(handle))
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[derive(Debug)]
struct TokenHandle(HANDLE);

impl TokenHandle {
    fn open(process: HANDLE) -> Result<Self, PlatformContextError> {
        let mut token = null_mut();
        let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
        if ok == FALSE || token.is_null() {
            return Err(PlatformContextError::Unavailable);
        }

        Ok(Self(token))
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for TokenHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_executable_name_from_process_path() {
        assert_eq!(
            executable_name(r"C:\Windows\System32\notepad.exe").unwrap(),
            "notepad.exe"
        );
    }

    #[test]
    fn rejects_empty_executable_name() {
        assert_eq!(
            executable_name(""),
            Err(PlatformContextError::InvalidOutput)
        );
    }

    #[test]
    fn converts_wide_string_to_utf8_lossy_string() {
        let wide: Vec<u16> = "notepad.exe".encode_utf16().collect();

        assert_eq!(wide_to_string(&wide).unwrap(), "notepad.exe");
    }
}
