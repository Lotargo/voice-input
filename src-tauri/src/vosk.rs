//! Minimal late-bound Vosk C API. A normal Rust build needs no Kaldi/Vosk linker setup;
//! `libvosk.dll` is required only when transcription actually starts.
use std::{ffi::{c_char, c_int, c_void, CStr, CString}, path::Path};
use libloading::os::windows::{
    Library, Symbol, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
};
use serde::Deserialize;

type ModelNew = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type ModelFree = unsafe extern "C" fn(*mut c_void);
type RecognizerNew = unsafe extern "C" fn(*mut c_void, f32) -> *mut c_void;
type RecognizerFree = unsafe extern "C" fn(*mut c_void);
type AcceptShort = unsafe extern "C" fn(*mut c_void, *const i16, c_int) -> c_int;
type GetResult = unsafe extern "C" fn(*mut c_void) -> *const c_char;

#[derive(Deserialize)]
struct RecognitionResult {
    #[serde(default)] text: String,
    #[serde(default)] partial: String,
}

pub struct Vosk {
    _library: Library,
    model: *mut c_void,
    recognizer: *mut c_void,
    model_free: ModelFree,
    recognizer_free: RecognizerFree,
    accept_short: AcceptShort,
    result: GetResult,
    partial: GetResult,
    final_result: GetResult,
}

impl Vosk {
    pub fn open(dll: &Path, model_path: &Path) -> Result<Self, String> {
        // SAFETY: Function signatures match the published Vosk C API (v0.3.45).
        // The Library remains owned by Self until after both native objects are freed.
        unsafe {
            // The shipped Vosk DLL has additional MinGW DLL dependencies next to it.
            // A normal LoadLibraryEx(0) does NOT look in the DLL's own directory.
            // Explicitly include that directory without modifying global PATH.
            let lib = Library::load_with_flags(
                dll,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
            .map_err(|e| format!("Загрузка libvosk.dll: {e}"))?;
            let get = |symbol: &[u8]| -> Result<*const (), String> {
                let f: Symbol<*const ()> = lib.get(symbol).map_err(|e| e.to_string())?;
                Ok(*f)
            };
            let model_new: ModelNew = std::mem::transmute(get(b"vosk_model_new\0")?);
            let model_free: ModelFree = std::mem::transmute(get(b"vosk_model_free\0")?);
            let recognizer_new: RecognizerNew = std::mem::transmute(get(b"vosk_recognizer_new\0")?);
            let recognizer_free: RecognizerFree = std::mem::transmute(get(b"vosk_recognizer_free\0")?);
            let accept_short: AcceptShort = std::mem::transmute(get(b"vosk_recognizer_accept_waveform_s\0")?);
            let result: GetResult = std::mem::transmute(get(b"vosk_recognizer_result\0")?);
            let partial: GetResult = std::mem::transmute(get(b"vosk_recognizer_partial_result\0")?);
            let final_result: GetResult = std::mem::transmute(get(b"vosk_recognizer_final_result\0")?);
            let path = CString::new(model_path.to_string_lossy().as_bytes())
                .map_err(|_| "Путь к модели содержит NUL".to_string())?;
            let model = model_new(path.as_ptr());
            if model.is_null() {
                return Err("Vosk не смог загрузить модель".into());
            }
            let recognizer = recognizer_new(model, 16_000.0);
            if recognizer.is_null() {
                model_free(model);
                return Err("Vosk не смог создать распознаватель".into());
            }
            Ok(Self {
                _library: lib, model, recognizer, model_free, recognizer_free,
                accept_short, result, partial, final_result,
            })
        }
    }

    pub fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
        let length = i32::try_from(samples.len()).map_err(|_| "Слишком большой аудиоблок".to_string())?;
        // SAFETY: The recognizer is live and samples remain valid for the whole call.
        let code = unsafe { (self.accept_short)(self.recognizer, samples.as_ptr(), length) };
        match code {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("Vosk вернул ошибку обработки аудио".into()),
        }
    }

    pub fn result(&mut self) -> String { self.read(self.result, false) }
    pub fn partial(&mut self) -> String { self.read(self.partial, true) }
    pub fn finish(&mut self) -> String { self.read(self.final_result, false) }

    fn read(&self, f: GetResult, partial: bool) -> String {
        // SAFETY: Vosk owns the JSON result memory until the next recognizer call.
        let ptr = unsafe { f(self.recognizer) };
        if ptr.is_null() { return String::new(); }
        let Ok(json) = (unsafe { CStr::from_ptr(ptr) }).to_str() else { return String::new() };
        match serde_json::from_str::<RecognitionResult>(json) {
            Ok(result) if partial => result.partial,
            Ok(result) => result.text,
            _ => String::new(),
        }
    }
}

impl Drop for Vosk {
    fn drop(&mut self) {
        // SAFETY: These are the matching destructor functions from the same DLL.
        unsafe {
            (self.recognizer_free)(self.recognizer);
            (self.model_free)(self.model);
        }
    }
}
