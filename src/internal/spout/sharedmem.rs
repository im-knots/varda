//! Spout's sender registry, which lives in named shared memory.
//!
//! Three maps, all created with `CreateFileMappingA` over the page file and
//! named exactly as Spout names them, because other applications open them by
//! name:
//!
//! - `SpoutSenderNames`, a table of `max_senders()` fixed 256-byte name slots.
//! - `ActiveSenderName`, one 256-byte slot naming the sender receivers default to.
//! - One map per sender, named after the sender, holding its
//!   [`SharedTextureInfo`](super::protocol::SharedTextureInfo).
//!
//! Each map has a companion mutex named `<map name>_mutex`. Spout waits 100ms on
//! it and proceeds anyway on timeout, which is the behaviour mirrored here: a
//! frame is worth more than a perfectly serialised read of a 280-byte struct
//! another process is rewriting.
//!
//! No GPU is involved, so all of this runs on a CI runner. See
//! /spec/spout-output.md § Wire protocol.

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Memory::{
    CreateFileMappingA, FILE_MAP_ALL_ACCESS, MEMORY_BASIC_INFORMATION, MapViewOfFile,
    OpenFileMappingA, PAGE_READWRITE, UnmapViewOfFile, VirtualQuery,
};
use windows::Win32::System::Threading::{CreateMutexA, ReleaseMutex, WaitForSingleObject};
use windows::core::PCSTR;

use super::protocol::{
    DEFAULT_MAX_SENDERS, SENDER_NAME_BYTES, SHARED_TEXTURE_INFO_BYTES, SharedTextureInfo,
    clamp_sender_name, decode_sender_names, encode_sender_names,
};

/// Spout's own wait, in milliseconds.
const WAIT_TIMEOUT_MS: u32 = 100;

/// The table of registered sender names.
const NAME_TABLE: &str = "SpoutSenderNames";

/// The sender a receiver connects to when it is given no name.
const ACTIVE_SENDER: &str = "ActiveSenderName";

/// A NUL-terminated copy of `name`, which the ANSI Win32 entry points require.
fn c_string(name: &str) -> Vec<u8> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

/// One named region plus the mutex guarding it.
///
/// Owns its handles and unmaps on drop. Not `Send`: every caller is on the
/// render thread, the same discipline the Syphon manager follows.
pub struct SharedMemory {
    map: HANDLE,
    view: *mut u8,
    mutex: HANDLE,
    size: usize,
}

impl SharedMemory {
    /// Open a region another application created.
    ///
    /// `None` when no sender of that name is publishing, which is the ordinary
    /// case rather than an error.
    pub fn open(name: &str, size: usize) -> Option<Self> {
        let c_name = c_string(name);
        let map = unsafe { OpenFileMappingA(FILE_MAP_ALL_ACCESS.0, false, PCSTR(c_name.as_ptr())) }
            .ok()?;
        Self::finish(map, name, size)
    }

    /// Create the region, or open it if it already exists.
    ///
    /// `CreateFileMappingA` returns the existing mapping when the name is taken,
    /// which is what Spout relies on: two applications publishing the same sender
    /// name share one region rather than one of them failing.
    pub fn create(name: &str, size: usize) -> Option<Self> {
        let c_name = c_string(name);
        let map = unsafe {
            CreateFileMappingA(
                windows::Win32::Foundation::INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                u32::try_from(size).ok()?,
                PCSTR(c_name.as_ptr()),
            )
        }
        .ok()?;
        Self::finish(map, name, size)
    }

    fn finish(map: HANDLE, name: &str, size: usize) -> Option<Self> {
        let view = unsafe { MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, 0) };
        if view.Value.is_null() {
            unsafe { CloseHandle(map) }.ok()?;
            return None;
        }
        // Never trust the requested size. `CreateFileMappingA` hands back the
        // *existing* mapping when the name is already taken, ignoring the size
        // asked for, and `open` is reading a region another application sized. So
        // the usable length is whatever was actually mapped, and every access is
        // bounded by that rather than by what the caller hoped for.
        let mut info = MEMORY_BASIC_INFORMATION::default();
        let queried = unsafe {
            VirtualQuery(
                Some(view.Value.cast_const()),
                std::ptr::from_mut(&mut info),
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        let mapped = if queried == 0 { 0 } else { info.RegionSize };
        let mutex_name = c_string(&format!("{name}_mutex"));
        let Ok(mutex) = (unsafe { CreateMutexA(None, false, PCSTR(mutex_name.as_ptr())) }) else {
            // Do not leak the view and handle on the way out.
            unsafe {
                let _ =
                    UnmapViewOfFile(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: view.Value,
                    });
                let _ = CloseHandle(map);
            }
            return None;
        };
        Some(Self {
            map,
            view: view.Value.cast::<u8>(),
            mutex,
            size: size.min(mapped),
        })
    }

    /// Hold the region's mutex for the duration of `f`.
    ///
    /// Proceeds on timeout rather than giving up, matching Spout: the worst case
    /// is one torn read of a struct that is rewritten every frame anyway, and
    /// blocking the render thread on another application's lock is worse.
    fn locked<T>(&self, f: impl FnOnce() -> T) -> T {
        let held = unsafe { WaitForSingleObject(self.mutex, WAIT_TIMEOUT_MS) } == WAIT_OBJECT_0;
        let out = f();
        if held {
            let _ = unsafe { ReleaseMutex(self.mutex) };
        }
        out
    }

    /// Copy the region out.
    ///
    /// Copies through a raw pointer rather than materialising a `&mut [u8]`.
    /// Several `SharedMemory` values can address one region, by design and by
    /// test, so handing out overlapping mutable slices would be aliasing UB even
    /// though the bytes are shared memory rather than heap.
    #[must_use]
    pub fn read(&self) -> Vec<u8> {
        self.locked(|| {
            let mut out = vec![0u8; self.size];
            if self.size > 0 {
                unsafe { std::ptr::copy_nonoverlapping(self.view, out.as_mut_ptr(), self.size) };
            }
            out
        })
    }

    /// Overwrite the region, up to whichever of the two is shorter.
    pub fn write(&self, data: &[u8]) {
        self.locked(|| {
            let n = data.len().min(self.size);
            if n > 0 {
                unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.view, n) };
            }
        });
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.view.cast(),
            });
            let _ = CloseHandle(self.map);
            let _ = CloseHandle(self.mutex);
        }
    }
}

/// How many sender slots the name table holds.
///
/// Spout stores this in the registry so every application on the machine agrees.
/// Reading it matters for compatibility: a user who raised the limit for another
/// application would otherwise see Varda miss senders past slot ten.
#[must_use]
pub fn max_senders() -> usize {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExA,
        RegQueryValueExA,
    };
    let subkey = c_string("Software\\Leading Edge\\Spout");
    let value = c_string("MaxSenders");
    let mut key = HKEY::default();
    let opened = unsafe {
        RegOpenKeyExA(
            HKEY_CURRENT_USER,
            PCSTR(subkey.as_ptr()),
            None,
            KEY_READ,
            std::ptr::from_mut(&mut key),
        )
    };
    if opened.is_err() {
        return DEFAULT_MAX_SENDERS;
    }
    let mut data = [0u8; 4];
    let mut size = u32::try_from(data.len()).unwrap_or(4);
    let mut kind = REG_VALUE_TYPE::default();
    let read = unsafe {
        RegQueryValueExA(
            key,
            PCSTR(value.as_ptr()),
            None,
            Some(std::ptr::from_mut(&mut kind)),
            Some(data.as_mut_ptr()),
            Some(std::ptr::from_mut(&mut size)),
        )
    };
    let _ = unsafe { RegCloseKey(key) };
    if read.is_err() {
        return DEFAULT_MAX_SENDERS;
    }
    let n = u32::from_le_bytes(data) as usize;
    // A zero or absurd value is a corrupt registry rather than an instruction to
    // allocate a gigabyte of name table.
    if n == 0 || n > 1024 {
        DEFAULT_MAX_SENDERS
    } else {
        n
    }
}

/// Every sender currently registered on this machine.
#[must_use]
pub fn sender_names() -> Vec<String> {
    let max = max_senders();
    let Some(table) = SharedMemory::open(NAME_TABLE, max * SENDER_NAME_BYTES) else {
        return Vec::new();
    };
    decode_sender_names(&table.read(), max)
}

/// The sender a receiver connects to when none is named.
#[must_use]
pub fn active_sender() -> Option<String> {
    let map = SharedMemory::open(ACTIVE_SENDER, SENDER_NAME_BYTES)?;
    decode_sender_names(&map.read(), 1).into_iter().next()
}

/// A sender's published texture, if it has published one.
#[must_use]
pub fn sender_info(name: &str) -> Option<SharedTextureInfo> {
    let map = SharedMemory::open(name, SHARED_TEXTURE_INFO_BYTES)?;
    SharedTextureInfo::decode(&map.read()).filter(SharedTextureInfo::is_publishable)
}

/// Add a name to the table, keeping the region alive for the caller.
///
/// The returned handle must be held: Windows frees a mapping when its last
/// handle closes, so dropping it deregisters the sender. `None` when the table
/// is full, which is Spout's behaviour rather than evicting somebody else.
#[must_use]
pub fn register_sender(name: &str) -> Option<SharedMemory> {
    let name = clamp_sender_name(name);
    let max = max_senders();
    let table = SharedMemory::create(NAME_TABLE, max * SENDER_NAME_BYTES)?;
    let mut names = decode_sender_names(&table.read(), max);
    if !names.iter().any(|n| n == &name) {
        names.push(name);
        table.write(&encode_sender_names(&names, max)?);
    }
    Some(table)
}

/// Remove a name from the table.
pub fn release_sender(name: &str) {
    let name = clamp_sender_name(name);
    let max = max_senders();
    let Some(table) = SharedMemory::open(NAME_TABLE, max * SENDER_NAME_BYTES) else {
        return;
    };
    let mut names = decode_sender_names(&table.read(), max);
    names.retain(|n| n != &name);
    if let Some(encoded) = encode_sender_names(&names, max) {
        table.write(&encoded);
    }
}

/// Publish a sender's texture description, keeping its region alive.
///
/// Held for the same reason as [`register_sender`]: the map lives only as long
/// as a handle to it does.
#[must_use]
pub fn publish_sender_info(name: &str, info: &SharedTextureInfo) -> Option<SharedMemory> {
    let map = SharedMemory::create(&clamp_sender_name(name), SHARED_TEXTURE_INFO_BYTES)?;
    map.write(&info.encode());
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These touch machine-global named objects, so two of them running at once
    /// would race on the same name table. Cargo runs tests in parallel by
    /// default, hence the lock.
    static TABLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Distinct per test, and distinctive enough not to collide with a real
    /// application if someone runs the suite with Spout senders live.
    fn test_name(suffix: &str) -> String {
        format!("Varda Test Sender {suffix}")
    }

    #[test]
    fn a_region_round_trips_through_shared_memory() {
        let name = test_name("roundtrip");
        let map = SharedMemory::create(&name, 64).expect("create");
        map.write(b"hello spout");
        let back = map.read();
        assert_eq!(&back[..11], b"hello spout");
        assert_eq!(
            back.len(),
            64,
            "the whole region is returned, not just the write"
        );
    }

    #[test]
    fn a_second_handle_sees_the_first_ones_writes() {
        // The property the whole protocol rests on: `CreateFileMappingA` returns
        // the existing mapping for a taken name rather than a private one.
        let name = test_name("shared");
        let a = SharedMemory::create(&name, 32).expect("create");
        let b = SharedMemory::open(&name, 32).expect("the name is taken, so open must find it");
        a.write(b"written by a");
        assert_eq!(&b.read()[..12], b"written by a");
    }

    #[test]
    fn opening_a_name_nobody_published_is_none_rather_than_an_error() {
        // A receiver polls for senders that mostly do not exist. This is the
        // ordinary case, not a failure.
        assert!(SharedMemory::open("Varda Test Sender That Does Not Exist", 32).is_none());
        assert!(sender_info("Varda Test Sender That Does Not Exist").is_none());
    }

    #[test]
    fn a_registered_sender_appears_in_the_table_and_leaves_on_release() {
        let _guard = TABLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let name = test_name("registry");
        let held = register_sender(&name).expect("register");
        assert!(
            sender_names().contains(&name),
            "a registered sender must be discoverable by other applications"
        );
        release_sender(&name);
        assert!(!sender_names().contains(&name));
        drop(held);
    }

    #[test]
    fn published_texture_info_survives_the_round_trip_through_windows() {
        let _guard = TABLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let name = test_name("info");
        let info = SharedTextureInfo {
            share_handle: 0x0004_2A1C,
            width: 1920,
            height: 1080,
            format: super::super::protocol::DxgiFormat::Bgra8Unorm.as_u32(),
            usage: 0,
            partner_id: 0,
        };
        let held = publish_sender_info(&name, &info).expect("publish");
        assert_eq!(
            sender_info(&name),
            Some(info),
            "what a receiver reads must be what the sender wrote"
        );
        drop(held);
    }

    #[test]
    fn dropping_the_handle_takes_the_sender_down() {
        // Windows frees a mapping when its last handle closes, so the lifetime of
        // the returned handle *is* the lifetime of the sender. Getting this wrong
        // would leave a sender advertised after the output stopped.
        let name = test_name("lifetime");
        let info = SharedTextureInfo {
            share_handle: 1,
            width: 16,
            height: 16,
            format: 0,
            usage: 0,
            partner_id: 0,
        };
        let held = publish_sender_info(&name, &info).expect("publish");
        assert!(sender_info(&name).is_some());
        drop(held);
        assert!(
            sender_info(&name).is_none(),
            "the region must not outlive the handle that published it"
        );
    }

    #[test]
    fn max_senders_is_sane_whatever_the_registry_says() {
        let n = max_senders();
        assert!(n > 0 && n <= 1024, "got {n}");
    }
}
