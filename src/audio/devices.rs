use std::collections::HashSet;

use windows::core::{Interface, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows::Win32::Media::Audio::{
    eCapture, eRender, AudioSessionStateActive, AudioSessionStateInactive, EDataFlow,
    IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointKind {
    Hardware,
    Software,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioEndpoint {
    pub id: String,
    pub name: String,
    pub kind: EndpointKind,
}

pub(crate) struct ComGuard(bool);

impl ComGuard {
    pub(crate) fn new() -> Self {
        Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

pub fn list_input_devices() -> Vec<AudioEndpoint> {
    enumerate_endpoints(eCapture)
}

pub fn list_output_devices() -> Vec<AudioEndpoint> {
    enumerate_endpoints(eRender)
}

pub fn list_audio_sessions() -> Vec<AudioEndpoint> {
    list_sessions("session", true)
}

pub fn list_output_apps() -> Vec<AudioEndpoint> {
    list_sessions("app", false)
}

fn list_sessions(prefix: &str, include_system: bool) -> Vec<AudioEndpoint> {
    let _com = ComGuard::new();
    unsafe { list_sessions_inner(prefix, include_system).unwrap_or_default() }
}

fn enumerate_endpoints(flow: EDataFlow) -> Vec<AudioEndpoint> {
    let _com = ComGuard::new();
    unsafe { enumerate_endpoints_inner(flow).unwrap_or_default() }
}

unsafe fn enumerate_endpoints_inner(flow: EDataFlow) -> windows::core::Result<Vec<AudioEndpoint>> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let collection = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
    let prefix = if flow == eCapture { "input" } else { "output" };
    let mut endpoints = Vec::new();
    for i in 0..collection.GetCount()? {
        let device = collection.Item(i)?;
        if let Ok(endpoint) = device_endpoint(&device, prefix) {
            endpoints.push(endpoint);
        }
    }
    Ok(endpoints)
}

unsafe fn device_endpoint(
    device: &IMMDevice,
    prefix: &str,
) -> windows::core::Result<AudioEndpoint> {
    let id = device.GetId()?;
    let id_text = id.to_string().unwrap_or_default();
    CoTaskMemFree(Some(id.0 as *const core::ffi::c_void));
    Ok(AudioEndpoint {
        id: format!("{prefix}:{id_text}"),
        name: device_name(device)?,
        kind: EndpointKind::Hardware,
    })
}

unsafe fn device_name(device: &IMMDevice) -> windows::core::Result<String> {
    let store = device.OpenPropertyStore(STGM_READ)?;
    let mut value = store.GetValue(&PKEY_Device_FriendlyName)?;
    let name = PropVariantToStringAlloc(&value);
    let _ = PropVariantClear(&mut value);
    let name = name?;
    let text = name.to_string().unwrap_or_default();
    CoTaskMemFree(Some(name.0 as *const core::ffi::c_void));
    Ok(text)
}

unsafe fn list_sessions_inner(
    prefix: &str,
    include_system: bool,
) -> windows::core::Result<Vec<AudioEndpoint>> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
    let captures = capture_pids();
    let own_pid = std::process::id();
    let mut seen = HashSet::new();
    let mut endpoints = Vec::new();
    for i in 0..collection.GetCount()? {
        let device = collection.Item(i)?;
        let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
            Ok(manager) => manager,
            Err(_) => continue,
        };
        let sessions = match manager.GetSessionEnumerator() {
            Ok(sessions) => sessions,
            Err(_) => continue,
        };
        for j in 0..sessions.GetCount().unwrap_or(0) {
            let control = match sessions.GetSession(j) {
                Ok(control) => control,
                Err(_) => continue,
            };
            if !matches!(
                control.GetState(),
                Ok(state) if state == AudioSessionStateActive || state == AudioSessionStateInactive
            ) {
                continue;
            }
            let control: IAudioSessionControl2 = match control.cast() {
                Ok(control) => control,
                Err(_) => continue,
            };
            let pid = control.GetProcessId().unwrap_or(0);
            if pid == 0 && !include_system {
                continue;
            }
            if pid == own_pid {
                continue;
            }
            if !seen.insert(pid) {
                continue;
            }
            endpoints.push(AudioEndpoint {
                id: format!("{prefix}:{pid}"),
                name: session_label(process_name(pid), pid, &captures),
                kind: EndpointKind::Software,
            });
        }
    }
    Ok(endpoints)
}

pub(crate) fn capture_pids() -> HashSet<u32> {
    let _com = ComGuard::new();
    unsafe { capture_pids_inner() }
}

unsafe fn capture_pids_inner() -> HashSet<u32> {
    let mut pids = HashSet::new();
    let enumerator: IMMDeviceEnumerator =
        match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
            Ok(enumerator) => enumerator,
            Err(_) => return pids,
        };
    let collection = match enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) {
        Ok(collection) => collection,
        Err(_) => return pids,
    };
    for i in 0..collection.GetCount().unwrap_or(0) {
        let device = match collection.Item(i) {
            Ok(device) => device,
            Err(_) => continue,
        };
        let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
            Ok(manager) => manager,
            Err(_) => continue,
        };
        let sessions = match manager.GetSessionEnumerator() {
            Ok(sessions) => sessions,
            Err(_) => continue,
        };
        for j in 0..sessions.GetCount().unwrap_or(0) {
            let control = match sessions.GetSession(j) {
                Ok(control) => control,
                Err(_) => continue,
            };
            if !matches!(
                control.GetState(),
                Ok(state) if state == AudioSessionStateActive || state == AudioSessionStateInactive
            ) {
                continue;
            }
            let control: IAudioSessionControl2 = match control.cast() {
                Ok(control) => control,
                Err(_) => continue,
            };
            let pid = control.GetProcessId().unwrap_or(0);
            if pid != 0 {
                pids.insert(pid);
            }
        }
    }
    pids
}

fn session_label(name: String, pid: u32, captures: &HashSet<u32>) -> String {
    if pid != 0 && captures.contains(&pid) {
        format!("{name} (appel)")
    } else {
        name
    }
}

pub(crate) fn endpoint_pid(id: &str) -> Option<u32> {
    id.strip_prefix("app:")
        .or_else(|| id.strip_prefix("session:"))
        .and_then(|pid| pid.parse().ok())
}

pub(crate) fn process_name(pid: u32) -> String {
    if pid == 0 {
        return "Sons du système".to_string();
    }
    let Some(path) = process_path(pid) else {
        return format!("Processus {pid}");
    };
    let file = path.rsplit('\\').next().unwrap_or(&path);
    let name = file
        .strip_suffix(".exe")
        .or_else(|| file.strip_suffix(".EXE"))
        .unwrap_or(file);
    if name.is_empty() {
        format!("Processus {pid}")
    } else {
        name.to_string()
    }
}

pub(crate) fn process_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

pub(crate) fn process_alive(pid: u32) -> bool {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(handle);
        alive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_label_marks_capture_without_pid() {
        let captures = HashSet::from([42]);
        assert_eq!(
            session_label("Discord".into(), 42, &captures),
            "Discord (appel)"
        );
        assert_eq!(session_label("Discord".into(), 43, &captures), "Discord");
        assert_eq!(
            session_label("Sons du système".into(), 0, &captures),
            "Sons du système"
        );
    }

    #[test]
    fn endpoint_pid_parses_session_and_app_ids() {
        assert_eq!(endpoint_pid("session:42"), Some(42));
        assert_eq!(endpoint_pid("app:7"), Some(7));
        assert_eq!(endpoint_pid("input:mic"), None);
    }
}
