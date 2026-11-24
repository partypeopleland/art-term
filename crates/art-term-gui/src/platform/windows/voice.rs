//! Windows dictation commands backed by SAPI through `robius-speech`.
//!
//! SAPI dictation is the only recognizer that works in an unpackaged desktop
//! app without package identity, and it is fully on-device. It still depends on
//! the user having installed the matching Windows speech recognition language.

use crate::voice_dto::{VoiceAckDto, VoiceEventDto, VoicePermissionDto, VoiceStatusDto};
use crate::voice_text::{append_segment, display_text};
use robius_speech::{NativeSpeechEvent, NativeSpeechOptions, NativeSpeechSession, SpeechError, SpeechErrorKind};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::{AppHandle, Emitter, State};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Globalization::LocaleNameToLCID;
use windows::Win32::Media::Speech::*;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED,
};

const DEFAULT_LANGUAGE: &str = "zh-TW";
const EVENT_NAME: &str = "gui-voice";

struct ActiveSession {
    native: Option<NativeSpeechSession>,
    committed: String,
    partial: String,
    terminal: bool,
}

#[derive(Default)]
pub struct VoiceSessions {
    inner: Mutex<HashMap<String, Arc<Mutex<ActiveSession>>>>,
}

impl VoiceSessions {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Arc<Mutex<ActiveSession>>>> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn find(&self, session_id: &str) -> Option<Arc<Mutex<ActiveSession>>> {
        self.lock().get(session_id).cloned()
    }

    fn purge_terminal(&self) {
        let stale: Vec<String> = {
            let sessions = self.lock();
            sessions
                .iter()
                .filter(|(_, session)| session.lock().map(|state| state.terminal).unwrap_or(true))
                .map(|(id, _)| id.clone())
                .collect()
        };
        if stale.is_empty() {
            return;
        }
        let mut sessions = self.lock();
        for id in stale {
            sessions.remove(&id);
        }
    }
}

fn emit(app: &AppHandle, session_id: &str, kind: &str, state: Option<&str>, text: Option<String>, error: Option<String>) {
    let _ = app.emit(
        EVENT_NAME,
        VoiceEventDto {
            session_id: session_id.to_string(),
            kind: kind.to_string(),
            state: state.map(str::to_string),
            text,
            error,
        },
    );
}

fn lock_session(session: &Arc<Mutex<ActiveSession>>) -> MutexGuard<'_, ActiveSession> {
    session.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn handle_event(app: &AppHandle, session_id: &str, session: &Arc<Mutex<ActiveSession>>, event: NativeSpeechEvent) {
    match event {
        NativeSpeechEvent::Started => {
            emit(app, session_id, "state", Some("listening"), None, None);
        }
        NativeSpeechEvent::Transcript { text, is_final } => {
            let display = {
                let mut state = lock_session(session);
                if state.terminal {
                    return;
                }
                if is_final {
                    append_segment(&mut state.committed, &text);
                    state.partial.clear();
                } else {
                    state.partial = text;
                }
                display_text(&state.committed, &state.partial)
            };
            emit(app, session_id, "partial", None, Some(display), None);
        }
        NativeSpeechEvent::AudioLevel(_) => {}
        NativeSpeechEvent::Stopped => {
            let text = {
                let mut state = lock_session(session);
                state.terminal = true;
                display_text(&state.committed, &state.partial)
            };
            emit(app, session_id, "final", None, Some(text), None);
            emit(app, session_id, "state", Some("idle"), None, None);
        }
        NativeSpeechEvent::Error(error) => {
            {
                let mut state = lock_session(session);
                state.terminal = true;
            }
            emit(app, session_id, "error", None, None, Some(speech_error_code(&error).to_string()));
            emit(app, session_id, "state", Some("idle"), None, None);
        }
    }
}

fn speech_error_code(error: &SpeechError) -> &'static str {
    match error.kind() {
        SpeechErrorKind::PermissionDenied => "PERMISSION_DENIED",
        SpeechErrorKind::Unavailable => "NOT_AVAILABLE",
        SpeechErrorKind::Language => "LANGUAGE_UNAVAILABLE",
        SpeechErrorKind::Audio => "AUDIO_UNAVAILABLE",
        SpeechErrorKind::Busy => "BUSY",
        _ => "UNKNOWN",
    }
}

#[derive(Default)]
struct Capabilities {
    recognizers: bool,
    zh_tw: bool,
    audio: bool,
}

struct Apartment;

impl Apartment {
    fn new() -> Result<Self, SpeechError> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
            .map_err(|error| SpeechError::new(SpeechErrorKind::Other, format!("Could not initialize COM for speech status: {error}")))?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn probe_capabilities() -> Capabilities {
    std::thread::Builder::new()
        .name("voice-status".into())
        .spawn(|| {
            let Ok(_apartment) = Apartment::new() else { return Capabilities::default() };
            unsafe {
                Capabilities {
                    recognizers: recognizers_available(),
                    zh_tw: recognizer_language_installed(DEFAULT_LANGUAGE),
                    audio: audio_input_available(),
                }
            }
        })
        .ok()
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default()
}

unsafe fn recognizers_available() -> bool {
    unsafe {
        let Ok(category): Result<ISpObjectTokenCategory, _> =
            CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_INPROC_SERVER)
        else { return false };
        if category.SetId(SPCAT_RECOGNIZERS, false).is_err() {
            return false;
        }
        let Ok(tokens) = category.EnumTokens(PCWSTR::null(), PCWSTR::null()) else { return false };
        let mut count = 0;
        tokens.GetCount(&mut count).is_ok() && count > 0
    }
}

unsafe fn recognizer_language_installed(locale: &str) -> bool {
    unsafe {
        let name: Vec<u16> = locale.encode_utf16().chain(Some(0)).collect();
        let language = LocaleNameToLCID(PCWSTR(name.as_ptr()), 0) & 0xffff;
        if language == 0 {
            return false;
        }
        let Ok(category): Result<ISpObjectTokenCategory, _> =
            CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_INPROC_SERVER)
        else { return false };
        if category.SetId(SPCAT_RECOGNIZERS, false).is_err() {
            return false;
        }
        let attributes: Vec<u16> = format!("Language={language:x}").encode_utf16().chain(Some(0)).collect();
        let Ok(tokens) = category.EnumTokens(PCWSTR(attributes.as_ptr()), PCWSTR::null()) else { return false };
        tokens.Item(0).is_ok()
    }
}

unsafe fn audio_input_available() -> bool {
    unsafe { default_token(SPCAT_AUDIOIN).is_ok() }
}

unsafe fn default_token(category_id: PCWSTR) -> windows::core::Result<ISpObjectToken> {
    unsafe {
        let category: ISpObjectTokenCategory = CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_INPROC_SERVER)?;
        category.SetId(category_id, false)?;
        let id = ComText(category.GetDefaultTokenId()?);
        let token: ISpObjectToken = CoCreateInstance(&SpObjectToken, None, CLSCTX_INPROC_SERVER)?;
        token.SetId(PCWSTR::null(), PCWSTR(id.0.0), false)?;
        Ok(token)
    }
}

struct ComText(PWSTR);

impl Drop for ComText {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}

#[tauri::command]
pub async fn gui_voice_status() -> VoiceStatusDto {
    let capabilities = probe_capabilities();
    let (available, detail) = if !capabilities.recognizers {
        (false, Some("此 Windows 裝置沒有可用的語音辨識引擎。".to_string()))
    } else if !capabilities.zh_tw {
        (false, Some("尚未安裝繁體中文（zh-TW）Windows 語音辨識語言，請在 Windows 設定安裝。".to_string()))
    } else if !capabilities.audio {
        (false, Some("找不到可用的麥克風輸入裝置，請檢查 Windows 麥克風設定。".to_string()))
    } else {
        (true, None)
    };
    let installed = if capabilities.zh_tw { vec![DEFAULT_LANGUAGE.to_string()] } else { Vec::new() };
    VoiceStatusDto {
        available,
        sdk_int: None,
        permission: if capabilities.audio { "granted".into() } else { "denied".into() },
        supported: installed.clone(),
        installed,
        language_switch_supported: false,
        detail,
    }
}

#[tauri::command]
pub async fn gui_voice_request_permission() -> VoicePermissionDto {
    let capabilities = probe_capabilities();
    VoicePermissionDto {
        permission: if capabilities.audio { "granted".into() } else { "denied".into() },
    }
}

#[tauri::command]
pub async fn gui_voice_start(
    app: AppHandle,
    sessions: State<'_, Arc<VoiceSessions>>,
    session_id: String,
    language: String,
    _allow_language_switch: bool,
) -> Result<VoiceAckDto, String> {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() || session_id.len() > 256 || session_id.chars().any(char::is_control) {
        return Err("UNKNOWN".into());
    }
    let language = if language.trim().is_empty() { DEFAULT_LANGUAGE.to_string() } else { language.trim().to_string() };
    if language.len() > 35 || language.contains('\0') {
        return Err("LANGUAGE_UNAVAILABLE".into());
    }

    sessions.purge_terminal();
    {
        let active_sessions = sessions.lock();
        if active_sessions.contains_key(&session_id) || !active_sessions.is_empty() {
            return Err("BUSY".into());
        }
    }

    let session = Arc::new(Mutex::new(ActiveSession {
        native: None,
        committed: String::new(),
        partial: String::new(),
        terminal: false,
    }));
    sessions.lock().insert(session_id.clone(), session.clone());

    let callback_app = app.clone();
    let callback_session_id = session_id.clone();
    let callback_session = session.clone();
    let started = NativeSpeechSession::start(
        NativeSpeechOptions { locale: Some(language), prefer_on_device: true },
        move |event| handle_event(&callback_app, &callback_session_id, &callback_session, event),
    );
    match started {
        Ok(native) => {
            // A cancel can land while `NativeSpeechSession::start` is still
            // running on another command thread; never leave the microphone
            // recording for a session the frontend already abandoned.
            // Dropping `native` cancels the just-started session.
            let mut state = lock_session(&session);
            if state.terminal {
                drop(state);
                sessions.lock().remove(&session_id);
            } else {
                state.native = Some(native);
            }
            Ok(VoiceAckDto { ok: true, mechanism: Some("windows-sapi".into()) })
        }
        Err(error) => {
            sessions.lock().remove(&session_id);
            Err(speech_error_code(&error).to_string())
        }
    }
}

#[tauri::command]
pub fn gui_voice_stop(sessions: State<'_, Arc<VoiceSessions>>, session_id: String) -> VoiceAckDto {
    if let Some(session) = sessions.find(&session_id) {
        let state = lock_session(&session);
        if let Some(native) = state.native.as_ref() {
            native.stop();
        }
    }
    VoiceAckDto { ok: true, mechanism: None }
}

#[tauri::command]
pub fn gui_voice_cancel(app: AppHandle, sessions: State<'_, Arc<VoiceSessions>>, session_id: String) -> VoiceAckDto {
    if let Some(session) = sessions.find(&session_id) {
        {
            let mut state = lock_session(&session);
            state.terminal = true;
            if let Some(native) = state.native.as_ref() {
                native.cancel();
            }
        }
        sessions.lock().remove(&session_id);
        emit(&app, &session_id, "state", Some("idle"), None, None);
    }
    VoiceAckDto { ok: true, mechanism: None }
}
