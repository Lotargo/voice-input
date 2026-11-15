//! Explicitly disable native runtime telemetry before any model/session is created.
use std::{ffi::CStr, sync::OnceLock};
static ENVIRONMENT: OnceLock<Result<usize, String>> = OnceLock::new();

pub fn ensure_offline() -> Result<(), String> {
    ENVIRONMENT
        .get_or_init(|| {
            // SAFETY: ort-sys supplies the published C ABI. Sherpa statically links the
            // compatible runtime; GetApi rejects unsupported versions. ORT reuses the
            // process-wide environment. Retain one reference for the process lifetime
            // so every later Sherpa session inherits disabled telemetry.
            unsafe {
                let base = ort_sys::OrtGetApiBase();
                if base.is_null() {
                    return Err("ONNX Runtime недоступен".into());
                }
                let api = ((*base).GetApi)(ort_sys::ORT_API_VERSION);
                if api.is_null() {
                    return Err("Несовместимая версия ONNX Runtime".into());
                }
                let api = &*api;
                let check = |status: ort_sys::OrtStatusPtr| {
                    if status.0.is_null() {
                        return Ok(());
                    }
                    let error = CStr::from_ptr((api.GetErrorMessage)(status.0))
                        .to_string_lossy()
                        .into_owned();
                    (api.ReleaseStatus)(status.0);
                    Err(error)
                };
                let mut environment = std::ptr::null_mut();
                check((api.CreateEnv)(
                    ort_sys::OrtLoggingLevel::ORT_LOGGING_LEVEL_ERROR,
                    c"VoiceInput-local".as_ptr(),
                    &mut environment,
                ))?;
                if let Err(error) = check((api.DisableTelemetryEvents)(environment)) {
                    (api.ReleaseEnv)(environment);
                    return Err(error);
                }
                Ok(environment as usize)
            }
        })
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

#[cfg(test)]
mod tests {
    #[test]
    fn telemetry_is_disabled_before_native_model_creation() {
        super::ensure_offline().unwrap();
        super::ensure_offline().unwrap();
    }
}
