//! Snippet Library: native shell around the single-page UI in ../src/index.html.
//!
//! The UI talks to these commands through `window.__TAURI__.core.invoke`.
//! The library is a plain JSON file (default: iCloud Drive/Snippet Library/snippets.json)
//! so it syncs between Macs and can be opened by the browser version too.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    AppHandle, Manager, RunEvent, State, WindowEvent,
};
use tauri_plugin_dialog::DialogExt;

const LIBRARY_FILE: &str = "snippets.json";
const BACKUP_DIR: &str = "Backups";
const KEEP_BACKUPS: usize = 30;

/* ---------------- settings ---------------- */

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    #[serde(default)]
    library_dir: Option<String>,
    #[serde(default = "yes")]
    show_tray: bool,
}
fn yes() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Settings { library_dir: None, show_tray: true }
    }
}

struct AppState {
    settings: Mutex<Settings>,
    tray: Mutex<Option<TrayIcon>>,
    last_backup_day: Mutex<i64>,
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}
fn load_settings(app: &AppHandle) -> Settings {
    settings_path(app)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
fn save_settings(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let p = settings_path(app)?;
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(p, json).map_err(|e| e.to_string())
}

/* ---------------- paths ---------------- */

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}
fn icloud_root() -> PathBuf {
    home().join("Library/Mobile Documents/com~apple~CloudDocs")
}
fn default_dir() -> PathBuf {
    let ic = icloud_root();
    if ic.is_dir() {
        ic.join("Snippet Library")
    } else {
        home().join("Documents").join("Snippet Library")
    }
}
fn library_dir(state: &AppState) -> PathBuf {
    state
        .settings
        .lock()
        .unwrap()
        .library_dir
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(default_dir)
}
fn in_icloud(dir: &Path) -> bool {
    dir.starts_with(icloud_root())
}
fn mtime_ms(p: &Path) -> Option<u64> {
    fs::metadata(p)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// iCloud may evict a file and leave a ".name.icloud" placeholder. Ask iCloud to download it.
fn ensure_downloaded(file: &Path) {
    if file.exists() {
        return;
    }
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else { return };
    let placeholder = dir.join(format!(".{}.icloud", name.to_string_lossy()));
    if !placeholder.exists() {
        return;
    }
    let _ = Command::new("brctl").arg("download").arg(file).status();
    for _ in 0..60 {
        if file.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Days since 1970-01-01 (UTC) to a calendar date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe as i64 + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}
fn today() -> i64 {
    (SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) / 86_400) as i64
}

/// Once a day, copy the current library into Backups/ before overwriting it. Keeps the newest 30.
fn daily_backup(state: &AppState, dir: &Path, file: &Path) {
    let day = today();
    {
        let mut last = state.last_backup_day.lock().unwrap();
        if *last == day || !file.exists() {
            return;
        }
        *last = day;
    }
    let bdir = dir.join(BACKUP_DIR);
    if fs::create_dir_all(&bdir).is_err() {
        return;
    }
    let (y, m, d) = civil_from_days(day);
    let target = bdir.join(format!("snippets-{y:04}-{m:02}-{d:02}.json"));
    if !target.exists() {
        let _ = fs::copy(file, &target);
    }
    if let Ok(rd) = fs::read_dir(&bdir) {
        let mut names: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .map(|n| {
                        let n = n.to_string_lossy();
                        n.starts_with("snippets-") && n.ends_with(".json")
                    })
                    .unwrap_or(false)
            })
            .collect();
        names.sort();
        if names.len() > KEEP_BACKUPS {
            for p in &names[..names.len() - KEEP_BACKUPS] {
                let _ = fs::remove_file(p);
            }
        }
    }
}

/* ---------------- library commands ---------------- */

#[derive(Serialize)]
struct LibInfo {
    dir: String,
    path: String,
    in_icloud: bool,
    data: Option<String>,
    mtime: Option<u64>,
}

fn read_library(dir: &Path) -> Result<LibInfo, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    let file = dir.join(LIBRARY_FILE);
    ensure_downloaded(&file);
    let data = if file.exists() {
        Some(fs::read_to_string(&file).map_err(|e| format!("Couldn't read {}: {e}", file.display()))?)
    } else {
        None
    };
    Ok(LibInfo {
        dir: dir.display().to_string(),
        path: file.display().to_string(),
        in_icloud: in_icloud(dir),
        data,
        mtime: mtime_ms(&file),
    })
}

#[tauri::command]
fn load_library(state: State<AppState>) -> Result<LibInfo, String> {
    read_library(&library_dir(&state))
}

#[tauri::command]
fn library_mtime(state: State<AppState>) -> Option<u64> {
    mtime_ms(&library_dir(&state).join(LIBRARY_FILE))
}

#[tauri::command]
fn save_library(state: State<AppState>, json: String) -> Result<Option<u64>, String> {
    // Refuse to write anything that isn't valid JSON, so a bug can never clobber the library.
    serde_json::from_str::<serde_json::Value>(&json).map_err(|e| format!("Not saved, invalid data: {e}"))?;
    let dir = library_dir(&state);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(LIBRARY_FILE);
    daily_backup(&state, &dir, &file);
    // Write to a temp file, then rename over the original, so a crash never leaves half a file.
    let tmp = dir.join(".snippets.json.saving");
    fs::write(&tmp, json.as_bytes()).map_err(|e| format!("Couldn't save: {e}"))?;
    fs::rename(&tmp, &file).map_err(|e| format!("Couldn't save: {e}"))?;
    Ok(mtime_ms(&file))
}

#[derive(Serialize)]
struct SettingsInfo {
    library_dir: String,
    default_dir: String,
    in_icloud: bool,
    show_tray: bool,
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> SettingsInfo {
    let dir = library_dir(&state);
    SettingsInfo {
        library_dir: dir.display().to_string(),
        default_dir: default_dir().display().to_string(),
        in_icloud: in_icloud(&dir),
        show_tray: state.settings.lock().unwrap().show_tray,
    }
}

/// Lets the user pick a folder. Does NOT switch to it; returns what's there so the UI can ask first.
#[tauri::command]
async fn pick_library_dir(app: AppHandle) -> Result<Option<LibInfo>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Choose a folder for your library")
        .blocking_pick_folder();
    let Some(fp) = picked else { return Ok(None) };
    let dir = fp.into_path().map_err(|e| e.to_string())?;
    read_library(&dir).map(Some)
}

#[tauri::command]
fn set_library_dir(app: AppHandle, state: State<AppState>, dir: String) -> Result<(), String> {
    let p = PathBuf::from(&dir);
    if !p.is_dir() {
        return Err(format!("{dir} isn't a folder"));
    }
    let snapshot = {
        let mut s = state.settings.lock().unwrap();
        s.library_dir = Some(dir);
        s.clone()
    };
    *state.last_backup_day.lock().unwrap() = 0;
    save_settings(&app, &snapshot)
}

#[tauri::command]
fn set_show_tray(app: AppHandle, state: State<AppState>, show: bool) -> Result<(), String> {
    let snapshot = {
        let mut s = state.settings.lock().unwrap();
        s.show_tray = show;
        s.clone()
    };
    if let Some(t) = state.tray.lock().unwrap().as_ref() {
        t.set_visible(show).map_err(|e| e.to_string())?;
    }
    save_settings(&app, &snapshot)
}

#[tauri::command]
fn reveal_library(state: State<AppState>) -> Result<(), String> {
    let dir = library_dir(&state);
    let file = dir.join(LIBRARY_FILE);
    let mut cmd = Command::new("open");
    if file.exists() {
        cmd.arg("-R").arg(file);
    } else {
        cmd.arg(dir);
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_backups(state: State<AppState>) -> Result<(), String> {
    let b = library_dir(&state).join(BACKUP_DIR);
    fs::create_dir_all(&b).map_err(|e| e.to_string())?;
    Command::new("open").arg(b).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    let lower = url.to_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")) {
        return Err("Only web and mail links can be opened".into());
    }
    Command::new("open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn export_backup(app: AppHandle, json: String, name: String) -> Result<bool, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Save a backup of your library")
        .set_file_name(&name)
        .add_filter("Snippet library", &["json"])
        .blocking_save_file();
    let Some(fp) = picked else { return Ok(false) };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
async fn import_backup(app: AppHandle) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Load a library backup")
        .add_filter("Snippet library", &["json"])
        .blocking_pick_file();
    let Some(fp) = picked else { return Ok(None) };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    fs::read_to_string(path).map(Some).map_err(|e| e.to_string())
}

/* ---------------- clipboard ---------------- */

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string())
}

/// Accepts a PNG as a data URL or bare base64 and puts the image on the clipboard.
#[tauri::command]
fn copy_image_png(data: String) -> Result<(), String> {
    let b64 = data.split_once("base64,").map(|(_, b)| b).unwrap_or(&data);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("Bad image data: {e}"))?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("Unsupported PNG format".into()),
    };
    let img = arboard::ImageData {
        width: info.width as usize,
        height: info.height as usize,
        bytes: rgba.into(),
    };
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_image(img))
        .map_err(|e| e.to_string())
}

/* ---------------- window + tray ---------------- */

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let open = MenuItem::with_id(app, "show", "Open Snippet Library", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Snippet Library", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &sep, &quit])?;
    TrayIconBuilder::with_id("main-tray")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true)
        .tooltip("Snippet Library")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .menu(|handle| Menu::default(handle))
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = load_settings(&handle);
            let show_tray = settings.show_tray;
            app.manage(AppState {
                settings: Mutex::new(settings),
                tray: Mutex::new(None),
                last_backup_day: Mutex::new(0),
            });
            let tray = build_tray(&handle)?;
            if !show_tray {
                tray.set_visible(false)?;
            }
            *app.state::<AppState>().tray.lock().unwrap() = Some(tray);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides it; the app keeps running in the Dock / menu bar.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_library,
            library_mtime,
            save_library,
            get_settings,
            pick_library_dir,
            set_library_dir,
            set_show_tray,
            reveal_library,
            open_backups,
            open_url,
            export_backup,
            import_backup,
            copy_text,
            copy_image_png
        ])
        .build(tauri::generate_context!())
        .expect("error while building Snippet Library");

    app.run(|_handle, _event| {
        // Clicking the Dock icon brings the hidden window back.
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = _event {
            show_main(_handle);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_742), (2026, 10, 16));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
