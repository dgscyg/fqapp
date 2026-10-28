//! Emulated libmetasec signing via the prebuilt `libfqsign` artifact.
//!
//! The artifact (libfqsign.so / fqsign.dll, built outside this repo) embeds
//! the real FQNovel signing SDK and produces the full 8-header signature
//! (x-argus/x-gorgon/x-helios/x-khronos/x-ladon/x-medusa/x-neptune/
//! x-soter) by emulating the original .so — unlike the pure-algorithm
//! modules here, which only approximate a subset.
//!
//! Loading is best-effort: the library is dlopen'd on first use from
//! `FQSIGN_LIB`, the executable's directory, or the working directory.
//! When it is missing or fails to boot, [`sign_request`] returns None and
//! the caller falls back to the pure algorithms — the app keeps working.
//!
//! ABI contract (fqsign.h, see rustnidbg release): a single engine handle
//! is NOT thread-safe, so all calls serialize through one Mutex. Boot is
//! ~50 ms once per process; each signature is a blocking call of a few
//! tens of ms.

use std::ffi::{c_char, c_int, CString};
use std::sync::{Mutex, OnceLock};

use md5::{Digest, Md5};

// ------------------------------------------------------------ platform ---

#[cfg(unix)]
mod imp {
    use std::ffi::{c_char, c_int, c_void, CStr};

    pub struct Lib(*mut c_void);
    // SAFETY: the handle is only used to resolve symbols at boot (single
    // thread) and is never closed; the raw pointer itself is inert.
    unsafe impl Send for Lib {}
    unsafe impl Sync for Lib {}

    pub unsafe fn open(path: &CStr) -> Option<Lib> {
        // RTLD_NOW | RTLD_LOCAL = 2 (Linux/Android) / 0x10002 on macOS; the
        // artifact targets Linux/Android only, so the plain glibc value.
        let h = dlopen(path.as_ptr(), 2);
        if h.is_null() {
            None
        } else {
            Some(Lib(h))
        }
    }
    pub unsafe fn sym<T>(lib: &Lib, name: &CStr) -> Option<T> {
        let p = dlsym(lib.0, name.as_ptr());
        if p.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy(&p))
        }
    }
    // The handle is never closed: the signer lives for the process
    // lifetime and the engine holds state derived from it.
    #[allow(dead_code)]
    pub unsafe fn close(_lib: Lib) {}

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, CStr, CString};

    pub struct Lib(*mut core::ffi::c_void);
    // SAFETY: see the unix twin — inert handle, resolved once at boot.
    unsafe impl Send for Lib {}
    unsafe impl Sync for Lib {}

    pub unsafe fn open(path: &CStr) -> Option<Lib> {
        let h = LoadLibraryA(path.as_ptr());
        if h.is_null() {
            None
        } else {
            Some(Lib(h as _))
        }
    }
    pub unsafe fn sym<T>(lib: &Lib, name: &CStr) -> Option<T> {
        let p = GetProcAddress(lib.0, name.as_ptr());
        if p.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy(&p))
        }
    }
    pub unsafe fn close(_lib: Lib) {}

    type HModule = *mut core::ffi::c_void;
    type Proc = *mut core::ffi::c_void;
    extern "system" {
        fn LoadLibraryA(name: *const c_char) -> HModule;
        fn GetProcAddress(module: HModule, name: *const c_char) -> Proc;
    }
}

// ----------------------------------------------------------------- ABI ---

/// Result buffer required by the ABI for one signature blob.
const RESULT_MIN: usize = 4096;

#[repr(C)]
struct FqConfig {
    resource_dir: *const c_char,
    asset_dir: *const c_char,
    fixed_time_ms: i64,
    /// NULL = embedded default device identity; otherwise the persisted
    /// per-device JSON {"serialno": "<16 hex>"} (see fqsign.h).
    device_json: *const c_char,
}

#[repr(C)]
struct FqEngineOpaque {
    _priv: [u8; 0],
}

struct Abi {
    // destroy is loaded for ABI completeness; the singleton engine is
    // released by process exit (the emulated .so holds per-engine state
    // that outlives any single request anyway).
    create: unsafe extern "C" fn(*const FqConfig, *mut *mut FqEngineOpaque) -> c_int,
    #[allow(dead_code)]
    destroy: unsafe extern "C" fn(*mut FqEngineOpaque),
    sign: unsafe extern "C" fn(
        *mut FqEngineOpaque,
        *const c_char,
        *const c_char,
        *mut c_char,
        usize,
    ) -> c_int,
    version: unsafe extern "C" fn() -> *const c_char,
}

unsafe impl Send for Abi {}

// ------------------------------------------------------------- signer ---

pub struct EmuSigner {
    _lib: imp::Lib,
    abi: Abi,
    /// The booted engine plus the device identity it was booted with; the
    /// engine is recreated when the caller signs for a different device
    /// (device rotation), which costs one ~50 ms boot.
    engine: Mutex<EngineSlot>,
}

struct EngineSlot {
    device_json: Option<CString>,
    handle: *mut FqEngineOpaque,
}

static EMU: OnceLock<Option<EmuSigner>> = OnceLock::new();

// SAFETY: the engine handle is only touched under `engine`'s Mutex (the ABI
// requires serialized calls); the library handle and fn pointers are inert.
unsafe impl Send for EmuSigner {}
unsafe impl Sync for EmuSigner {}

impl EmuSigner {
    /// Process-wide singleton; None once boot has failed (never retried —
    /// a broken artifact should not add a boot attempt to every request).
    pub fn global() -> Option<&'static EmuSigner> {
        EMU.get_or_init(init).as_ref()
    }

    /// Signs with the engine booted for `device_json`; a different identity
    /// (device rotation) re-boots the engine first.
    fn sign(&self, url: &str, header: &str, device_json: Option<&str>) -> Result<String, String> {
        let url = CString::new(url).map_err(|_| "url contains NUL")?;
        let header = CString::new(header).map_err(|_| "header contains NUL")?;
        let want = device_json
            .and_then(|s| CString::new(s).ok())
            .filter(|c| !c.to_bytes().is_empty());
        let mut guard = self.engine.lock().unwrap();
        if guard.handle.is_null() || guard.device_json.as_ref() != want.as_ref() {
            self.reboot(&mut guard, want);
        }
        if guard.handle.is_null() {
            return Err("engine boot failed".into());
        }
        let mut buf = vec![0u8; RESULT_MIN];
        // SAFETY: the engine handle is valid (rebooted above on failure is
        // fatal for this call) and calls are serialized by the Mutex, as
        // the ABI requires.
        let rc = unsafe {
            (self.abi.sign)(
                guard.handle,
                url.as_ptr(),
                header.as_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
            )
        };
        if rc > 0 {
            // Positive = required size; grow once and retry.
            buf.resize(rc as usize + 1, 0);
            let rc2 = unsafe {
                (self.abi.sign)(
                    guard.handle,
                    url.as_ptr(),
                    header.as_ptr(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                )
            };
            return finish(rc2, &buf);
        }
        finish(rc, &buf)
    }

    /// Swaps the engine under `slot` for one booted with `device_json`.
    fn reboot(&self, slot: &mut EngineSlot, device_json: Option<CString>) {
        unsafe { (self.abi.destroy)(slot.handle) };
        let dj = device_json
            .as_deref()
            .map(|c| c.to_string_lossy().into_owned());
        let handle = boot(&self.abi, dj.as_deref());
        slot.device_json = device_json;
        slot.handle = handle;
    }
}

fn finish(rc: c_int, buf: &[u8]) -> Result<String, String> {
    match rc {
        0 => {
            let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
        }
        _ => Err(format!("fqsign_sign rc={rc}")),
    }
}

/// Resolves one symbol with its type inferred from the call site.
fn resolve<T>(lib: &imp::Lib, name: &str) -> Option<T> {
    let c = CString::new(name).ok()?;
    // SAFETY: dlsym of a fixed name; the caller transmutes to the declared
    // ABI type, which matches fqsign.h.
    unsafe { imp::sym(lib, &c) }
}

fn init() -> Option<EmuSigner> {
    // Path-based lookup first (desktop); on Android `current_exe` is the
    // zygote, so fall back to a bare soname dlopen — the app's
    // nativeLibraryDir is on the linker search path there.
    let (path, cpath) = match locate_library() {
        Some(p) => {
            let c = CString::new(p.to_string_lossy().as_bytes()).ok()?;
            (p, c)
        }
        None => {
            let c = CString::new(lib_file_name()).ok()?;
            (std::path::PathBuf::from(lib_file_name()), c)
        }
    };
    // SAFETY: dlopen of a fixed path or soname; symbols are resolved
    // before any use.
    let lib = unsafe { imp::open(&cpath) }?;
    let abi = Abi {
        create: resolve(&lib, "fqsign_create")?,
        destroy: resolve(&lib, "fqsign_destroy")?,
        sign: resolve(&lib, "fqsign_sign")?,
        version: resolve(&lib, "fqsign_version")?,
    };
    let ver = unsafe { CStr::from_ptr((abi.version)()) }
        .to_string_lossy()
        .into_owned();
    log::info!("sign/emu: {} loaded ({})", ver, path.display());
    // The engine itself boots lazily on the first sign (with that call's
    // device identity), so a device rotation never pays two boots.
    Some(EmuSigner {
        _lib: lib,
        abi,
        engine: Mutex::new(EngineSlot {
            device_json: None,
            handle: std::ptr::null_mut(),
        }),
    })
}

/// Boots one engine with `device_json` (NULL = embedded default identity).
/// Returns a null handle on failure — the caller falls back to the pure
/// algorithms for that request.
fn boot(abi: &Abi, device_json: Option<&str>) -> *mut FqEngineOpaque {
    // Embedded resources (resource_dir = NULL) + real clock.
    let dj = device_json.and_then(|s| CString::new(s).ok());
    let cfg = FqConfig {
        resource_dir: std::ptr::null(),
        asset_dir: std::ptr::null(),
        fixed_time_ms: 0,
        device_json: dj.as_deref().map_or(std::ptr::null(), |c| c.as_ptr()),
    };
    let mut engine: *mut FqEngineOpaque = std::ptr::null_mut();
    // SAFETY: one boot per identity; the handle is owned by the EngineSlot
    // and freed by reboot()/process exit.
    let rc = unsafe { (abi.create)(&cfg, &mut engine) };
    if rc != 0 || engine.is_null() {
        log::warn!("sign/emu: fqsign boot failed rc={rc}");
        return std::ptr::null_mut();
    }
    engine
}

use std::ffi::CStr;
use std::path::PathBuf;

fn lib_file_name() -> &'static str {
    if cfg!(windows) {
        "fqsign.dll"
    } else {
        "libfqsign.so"
    }
}

fn locate_library() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("FQSIGN_LIB") {
        if !p.is_empty() {
            return Some(p.into());
        }
    }
    let name = lib_file_name();
    let exe = std::env::current_exe().ok()?;
    let mut dirs: Vec<PathBuf> = vec![exe.parent()?.to_path_buf()];
    if let Some(parent) = exe.parent().and_then(|p| p.parent()) {
        dirs.push(parent.join("lib"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    dirs.into_iter()
        .find(|d| d.join(name).is_file())
        .map(|d| d.join(name))
}

// ------------------------------------------------------------- signing ---

/// Signs one request through the emulated SDK. Returns None when the
/// artifact is unavailable or the signature failed — the caller then uses
/// the pure-algorithm path. On success the returned headers cover the 8
/// x-* signature headers plus x-ss-req-ticket and x-ss-stub, matching what
/// [`crate::sign::Manager::generate_headers`] produces for the same call.
pub fn sign_request(
    full_url: &str,
    request_headers: &[(String, String)],
    post_data: Option<&[u8]>,
    now_ms: i64,
    device_serial: Option<&str>,
) -> Option<Vec<(String, String)>> {
    let emu = EmuSigner::global()?;

    let mut block = String::new();
    for (k, v) in request_headers {
        push_line(&mut block, k, v);
    }
    push_line(&mut block, "x-ss-req-ticket", &now_ms.to_string());
    let mut stub_hex = String::new();
    if let Some(body) = post_data {
        if !body.is_empty() {
            let mut h = Md5::new();
            h.update(body);
            stub_hex = hex::encode(h.finalize());
            push_line(&mut block, "x-ss-stub", &stub_hex);
        }
    }

    // The persisted per-device identity (fqsign device_json); the engine
    // re-boots when the pooled device rotates.
    let device_json = device_serial
        .filter(|s| !s.is_empty())
        .map(|s| format!("{{\"serialno\":\"{s}\"}}"));
    let blob = match emu.sign(full_url, &block, device_json.as_deref()) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("sign/emu: sign failed: {}", e);
            return None;
        }
    };
    let mut headers = parse_blob(&blob)?;
    // The emulated SDK does not emit the caller-side ticket/stub headers;
    // add the ones whose values went into the signature input.
    headers.push(("x-ss-req-ticket".into(), now_ms.to_string()));
    if !stub_hex.is_empty() {
        headers.push(("x-ss-stub".into(), stub_hex));
    }
    Some(headers)
}

fn push_line(block: &mut String, name: &str, value: &str) {
    block.push_str(name);
    block.push_str("\r\n");
    block.push_str(value);
    block.push_str("\r\n");
}

/// Parses the "name\r\nvalue\r\n..." signature blob. Returns None on a
/// structurally invalid blob (odd fragment count) so the caller falls back
/// rather than sending a half-parsed signature.
fn parse_blob(blob: &str) -> Option<Vec<(String, String)>> {
    let blob = blob.trim_end_matches("\r\n");
    if blob.is_empty() {
        return None;
    }
    let parts: Vec<&str> = blob.split("\r\n").collect();
    if !parts.len().is_multiple_of(2) {
        return None;
    }
    Some(
        parts
            .chunks(2)
            .map(|kv| (kv[0].to_string(), kv[1].to_string()))
            .collect(),
    )
}
