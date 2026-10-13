//! The C interface for songs: load a tracker module or one of our own songs
//! into an engine, and read where it is playing. `docs/engine-api.md` is
//! the reference. Playing, pausing, seeking and the rest are `song` events,
//! sent like any other event.
//!
//! As with the rest of the interface, the functions that take a `*mut
//! Engine` must be called from one thread at a time. `synths_song_position`
//! takes a `*const Engine` and only reads numbers the audio side stores one
//! at a time, so a display may call it while another thread renders.

use crate::engine::Engine;
use crate::song::{self, LoadError};
use crate::song_player::{self, InstallError, INFO_SIZE, POSITION_SIZE};
use std::sync::Mutex;

/// The song was loaded.
pub const SONG_OK: i32 = 0;
/// A null pointer.
pub const SONG_NULL: i32 = -1;
/// The data is not a song this engine knows, or is damaged beyond loading;
/// `synths_song_error` says what is wrong.
pub const SONG_BAD: i32 = -2;
/// A sample file the song names has not been given with `synths_song_file`.
pub const SONG_MISSING_FILE: i32 = -3;
/// More than 32 channels: a song's channels play on parts 32 to 63.
pub const SONG_NO_PARTS: i32 = -4;
/// Too many samples for the slots from `first_slot` on.
pub const SONG_NO_SLOTS: i32 = -5;

/// Sample files waiting for the next song to be loaded, and what went wrong
/// with the last one. Neither is touched while sound is being made.
static FILES: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
static ERROR: Mutex<String> = Mutex::new(String::new());

fn fail(code: i32, message: String) -> i32 {
    if let Ok(mut e) = ERROR.lock() {
        *e = message;
        e.push('\0');
    }
    code
}

/// Reads a song from the bytes of a file of any kind this build knows, with
/// the sample files a JSON song may name. Also says whether it is a tracker
/// module, to be played with those trackers' raw sound.
fn read(data: &[u8], files: &[(String, Vec<u8>)]) -> Result<(song::Song, bool), LoadError> {
    #[cfg(feature = "json")]
    if let Some(text) = core::str::from_utf8(data).ok().filter(|text| song::native::is_native(text)) {
        let file = |name: &str| files.iter().find(|(n, _)| n == name).map(|(_, d)| d.clone());
        return song::native::load(text, &file).map(|song| (song, false));
    }
    let _ = files;
    let raw = !song::binary::is_binary(data);
    song::load_module(data).map(|song| (song, raw))
}

/// The packed song `synths_song_pack` last made.
static PACKED: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Turn a song file of any kind this build reads (MOD, XM, JSON where the
/// build has it, or packed already) into the packed form, which every build
/// reads. Returns the packed song's length in bytes, or a negative number
/// as `synths_song_load` does. `synths_song_packed` then gives the bytes.
/// Sample files given with `synths_song_file` are used and forgotten.
///
/// # Safety
/// `data` must point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn synths_song_pack(data: *const u8, len: u32) -> i32 {
    let files = FILES.lock().map(|mut f| core::mem::take(&mut *f)).unwrap_or_default();
    if data.is_null() {
        return fail(SONG_NULL, "a null pointer".to_string());
    }
    match read(core::slice::from_raw_parts(data, len as usize), &files) {
        Ok((song, _)) => {
            let packed = song::binary::save(&song);
            let length = packed.len().min(i32::MAX as usize) as i32;
            if let Ok(mut p) = PACKED.lock() {
                *p = packed;
            }
            fail(SONG_OK, String::new());
            length
        }
        Err(e @ LoadError::MissingSample(_)) => fail(SONG_MISSING_FILE, e.to_string()),
        Err(e) => fail(SONG_BAD, e.to_string()),
    }
}

/// The bytes of the song `synths_song_pack` last packed. The pointer is
/// good until the next `synths_song_pack`.
#[no_mangle]
pub extern "C" fn synths_song_packed() -> *const u8 {
    PACKED.lock().map_or(core::ptr::null(), |p| p.as_ptr())
}

/// Give a sample file for the next song loaded: `name` is the path the
/// song's JSON names (not ending in a zero byte), `data` the WAV file. The
/// files given are used, and then forgotten, by the next `synths_song_load`.
/// Returns 0, or -1 for a null pointer or a name that is not UTF-8.
///
/// # Safety
/// `name` must point to `name_len` bytes and `data` to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn synths_song_file(name: *const u8, name_len: u32, data: *const u8, len: u32) -> i32 {
    if name.is_null() || data.is_null() {
        return SONG_NULL;
    }
    let Ok(name) = core::str::from_utf8(core::slice::from_raw_parts(name, name_len as usize)) else { return SONG_NULL };
    let data = core::slice::from_raw_parts(data, len as usize).to_vec();
    match FILES.lock() {
        Ok(mut files) => {
            files.retain(|(n, _)| n != name);
            files.push((name.to_string(), data));
            SONG_OK
        }
        Err(_) => SONG_NULL,
    }
}

/// Load a song from the bytes of a file: a MOD, an XM, one of our own songs
/// packed, or (where the build has it) one of our own as JSON. Its channels take parts 32 to 63, one each (their volume,
/// pan, amp envelope and filter are set for playing samples as they are),
/// and its samples the slots from `first_slot` on; a song already loaded is
/// unloaded first. The song is ready and stopped: send a `song`
/// event to play it. Returns 0, or a negative number (see the document).
///
/// # Safety
/// `engine` must be a live engine or null; `data` must point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn synths_song_load(engine: *mut Engine, data: *const u8, len: u32, first_slot: u32) -> i32 {
    let files = FILES.lock().map(|mut f| core::mem::take(&mut *f)).unwrap_or_default();
    let (Some(engine), false) = (engine.as_mut(), data.is_null()) else { return fail(SONG_NULL, "a null pointer".to_string()) };
    let (song, raw) = match read(core::slice::from_raw_parts(data, len as usize), &files) {
        Ok(read) => read,
        Err(e @ LoadError::MissingSample(_)) => return fail(SONG_MISSING_FILE, e.to_string()),
        Err(e) => return fail(SONG_BAD, e.to_string()),
    };
    if first_slot > u16::MAX as u32 {
        return fail(SONG_NO_SLOTS, "there is no such slot".to_string());
    }
    let channels = song.channels();
    match song_player::install(engine, song, song_player::FIRST_PART, first_slot as u16, raw) {
        Ok(_) => fail(SONG_OK, String::new()),
        Err(InstallError::Parts) => fail(SONG_NO_PARTS, song::text::join(&["the song has ", &song::text::int(channels as i64), " channels, and the engine plays at most 32"])),
        Err(InstallError::Slots) => fail(SONG_NO_SLOTS, "the song has more samples than there are slots left".to_string()),
    }
}

/// Why the last `synths_song_load` failed, in words, as text ending in a
/// zero byte (empty if it did not fail). The pointer is good until the next
/// call to `synths_song_load`.
#[no_mangle]
pub extern "C" fn synths_song_error() -> *const u8 {
    match ERROR.lock() {
        Ok(mut e) => {
            if e.is_empty() {
                e.push('\0');
            }
            e.as_ptr()
        }
        Err(_) => c"".as_ptr().cast(),
    }
}

/// Take the loaded song out: its voices are cut and its sample slots emptied.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_song_unload(engine: *mut Engine) {
    if let Some(engine) = engine.as_mut() {
        song_player::remove(engine);
    }
}

/// Write eight facts about the loaded song into `out`: channels, orders,
/// patterns, instruments, samples, first part, first slot, and its rules (0
/// FastTracker 2, 1 ProTracker). Returns 8, or 0 if no song is loaded.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for 8 numbers.
#[no_mangle]
pub unsafe extern "C" fn synths_song_info(engine: *const Engine, out: *mut u32) -> u32 {
    match engine.as_ref().and_then(song_player::shared) {
        Some(shared) if !out.is_null() => {
            core::ptr::copy_nonoverlapping(shared.info().as_ptr(), out, INFO_SIZE);
            INFO_SIZE as u32
        }
        _ => 0,
    }
}

/// Write where the song is into `out`, ten numbers: state (0 no song, 1
/// stopped, 2 playing, 3 paused), order, pattern, row, tick, ticks in a
/// row, beats a minute, times the song has come round, rows in this
/// pattern, orders in the song. Returns 10; with no song loaded, writes
/// zeros. Safe to call while another thread renders.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for 10 numbers.
#[no_mangle]
pub unsafe extern "C" fn synths_song_position(engine: *const Engine, out: *mut u32) -> u32 {
    if out.is_null() {
        return 0;
    }
    let numbers = engine.as_ref().and_then(song_player::shared).map_or([0; POSITION_SIZE], |s| s.position());
    core::ptr::copy_nonoverlapping(numbers.as_ptr(), out, POSITION_SIZE);
    POSITION_SIZE as u32
}

/// Copy the name of the synth patch an instrument plays (instruments count
/// from 1) into `out`, at most `capacity` bytes, with no zero byte. Returns
/// the name's whole length in bytes: 0 if the instrument plays samples.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for `capacity` bytes.
#[no_mangle]
pub unsafe extern "C" fn synths_song_patch_name(engine: *const Engine, instrument: u32, out: *mut u8, capacity: u32) -> u32 {
    let name = engine.as_ref().and_then(song_player::shared).and_then(|s| s.patch_name(instrument));
    match name {
        Some(name) => {
            if !out.is_null() {
                core::ptr::copy_nonoverlapping(name.as_ptr(), out, name.len().min(capacity as usize));
            }
            name.len() as u32
        }
        None => 0,
    }
}
