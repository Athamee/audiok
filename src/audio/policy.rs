use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{GUID, HRESULT, HSTRING};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

const CLASS: &str = "Windows.Media.Internal.AudioPolicyConfig";
const IID_WIN11: GUID = GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324);
const IID_DOWNLEVEL: GUID = GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258);
const MMDEVAPI_TOKEN: &str = r"\\?\SWD#MMDEVAPI#";
const DEVINTERFACE_AUDIO_RENDER: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const DEVINTERFACE_AUDIO_CAPTURE: &str = "#{2eef81be-33fa-4800-9670-1cd474972c3f}";
const E_RENDER: i32 = 0;
const E_CAPTURE: i32 = 1;
const E_CONSOLE: i32 = 0;
const E_MULTIMEDIA: i32 = 1;
const E_COMMUNICATIONS: i32 = 2;
const ROLES: [i32; 3] = [E_CONSOLE, E_MULTIMEDIA, E_COMMUNICATIONS];
const PROCESS_NO_AUDIO: HRESULT = HRESULT(0x80070057u32 as i32);

type SetEndpoint = unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut c_void) -> HRESULT;
type GetEndpoint = unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut HSTRING) -> HRESULT;

static FACTORY: OnceLock<usize> = OnceLock::new();

pub fn set_default_capture(pid: u32, endpoint_id: &str) -> Result<(), String> {
    set_default_endpoint(
        pid,
        E_CAPTURE,
        DEVINTERFACE_AUDIO_CAPTURE,
        Some(endpoint_id),
        &ROLES,
    )
}

pub fn set_default_render(pid: u32, endpoint_id: Option<&str>) -> Result<(), String> {
    let results = apply_endpoint(
        pid,
        E_RENDER,
        DEVINTERFACE_AUDIO_RENDER,
        endpoint_id,
        &ROLES,
    )?;
    let failures: Vec<String> = results
        .iter()
        .filter(|(_, hr)| !hr.is_ok())
        .map(|(role, hr)| format!("rôle {role}: {hr:?}"))
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join(", "))
    }
}

pub fn get_default_render(pid: u32) -> [Option<String>; 3] {
    ROLES.map(|role| get_default_endpoint(pid, E_RENDER, role))
}

pub fn get_default_capture(pid: u32) -> [Option<String>; 3] {
    ROLES.map(|role| get_default_endpoint(pid, E_CAPTURE, role))
}

pub fn restore_default_render(pid: u32, previous: &[Option<String>; 3]) -> Result<(), String> {
    restore_default(pid, E_RENDER, DEVINTERFACE_AUDIO_RENDER, previous)
}

pub fn restore_default_capture(pid: u32, previous: &[Option<String>; 3]) -> Result<(), String> {
    restore_default(pid, E_CAPTURE, DEVINTERFACE_AUDIO_CAPTURE, previous)
}

fn restore_default(
    pid: u32,
    flow: i32,
    interface: &str,
    previous: &[Option<String>; 3],
) -> Result<(), String> {
    let mut errors = Vec::new();
    for (role, value) in ROLES.iter().zip(previous) {
        if let Err(error) = set_default_endpoint(pid, flow, interface, value.as_deref(), &[*role]) {
            errors.push(format!("rôle {role}: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(", "))
    }
}

fn set_default_endpoint(
    pid: u32,
    flow: i32,
    interface: &str,
    endpoint_id: Option<&str>,
    roles: &[i32],
) -> Result<(), String> {
    let results = apply_endpoint(pid, flow, interface, endpoint_id, roles)?;
    let mut last = PROCESS_NO_AUDIO;
    let mut any_ok = false;
    for (_, hr) in results {
        if hr.is_ok() {
            any_ok = true;
        } else {
            last = hr;
        }
    }
    if any_ok {
        Ok(())
    } else {
        Err(format!("{last:?}"))
    }
}

fn apply_endpoint(
    pid: u32,
    flow: i32,
    interface: &str,
    endpoint_id: Option<&str>,
    roles: &[i32],
) -> Result<Vec<(i32, HRESULT)>, String> {
    let policy = factory().ok_or("AudioPolicyConfig indisponible")?;
    let device =
        endpoint_id.map(|endpoint| HSTRING::from(format!("{MMDEVAPI_TOKEN}{endpoint}{interface}")));
    let vtbl = unsafe { *(policy as *const *const usize) };
    let set: SetEndpoint = unsafe { std::mem::transmute(*vtbl.add(25)) };

    let mut results = Vec::with_capacity(roles.len());
    for &role in roles {
        let device = match &device {
            Some(device) => unsafe { std::mem::transmute_copy(device) },
            None => std::ptr::null_mut(),
        };
        let hr = unsafe { set(policy, pid, flow, role, device) };
        results.push((role, hr));
    }
    Ok(results)
}

fn get_default_endpoint(pid: u32, flow: i32, role: i32) -> Option<String> {
    let policy = factory()?;
    let vtbl = unsafe { *(policy as *const *const usize) };
    let get: GetEndpoint = unsafe { std::mem::transmute(*vtbl.add(26)) };
    let mut device = HSTRING::default();
    let hr = unsafe { get(policy, pid, flow, role, &mut device) };
    if !hr.is_ok() {
        return None;
    }
    normalize_device_id(unpack_device_id(&device.to_string_lossy()))
}

fn normalize_device_id(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn unpack_device_id(packed: &str) -> String {
    let packed = packed.strip_prefix(MMDEVAPI_TOKEN).unwrap_or(packed);
    packed
        .strip_suffix(DEVINTERFACE_AUDIO_RENDER)
        .or_else(|| packed.strip_suffix(DEVINTERFACE_AUDIO_CAPTURE))
        .unwrap_or(packed)
        .to_string()
}

fn factory() -> Option<*mut c_void> {
    let pointer = *FACTORY.get_or_init(|| unsafe {
        create_factory().unwrap_or_else(|error| {
            crate::logging::log(
                "policy",
                &format!("routage applicatif indisponible: {error}"),
            );
            0
        })
    });
    (pointer != 0).then_some(pointer as *mut c_void)
}

unsafe fn create_factory() -> Result<usize, String> {
    let module: HMODULE = LoadLibraryW(windows::core::w!("audioses.dll"))
        .map_err(|error| format!("audioses.dll: {error}"))?;
    let proc = GetProcAddress(module, windows::core::s!("DllGetActivationFactory"))
        .ok_or("DllGetActivationFactory introuvable")?;
    let get_factory: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT =
        unsafe { std::mem::transmute(proc) };

    let class = HSTRING::from(CLASS);
    let mut raw: *mut c_void = std::ptr::null_mut();
    let class_arg = unsafe { std::mem::transmute_copy(&class) };
    unsafe { get_factory(class_arg, &mut raw) }
        .ok()
        .map_err(|error| format!("DllGetActivationFactory: {error}"))?;
    if raw.is_null() {
        return Err("fabrique nulle".into());
    }

    let vtbl = unsafe { *(raw as *const *const usize) };
    let query_interface: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT = unsafe { std::mem::transmute(*vtbl) };

    for iid in [&IID_WIN11, &IID_DOWNLEVEL] {
        let mut policy: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { query_interface(raw, iid, &mut policy) };
        if hr.is_ok() && !policy.is_null() {
            unsafe { release(raw) };
            return Ok(policy as usize);
        }
    }

    unsafe { release(raw) };
    Err("interface AudioPolicyConfig non supportée".into())
}

unsafe fn release(object: *mut c_void) {
    let vtbl = unsafe { *(object as *const *const usize) };
    let release: unsafe extern "system" fn(*mut c_void) -> u32 =
        unsafe { std::mem::transmute(*vtbl.add(2)) };
    unsafe { release(object) };
}

#[cfg(test)]
mod tests {
    use super::normalize_device_id;

    #[test]
    fn empty_persisted_endpoint_is_none() {
        assert_eq!(normalize_device_id(String::new()), None);
    }

    #[test]
    fn persisted_endpoint_is_kept() {
        let id = "{0.0.0.00000000}.{4e7006a3-8404-42bd-b4ce-2b166506d705}".to_string();
        assert_eq!(normalize_device_id(id.clone()), Some(id));
    }
}
