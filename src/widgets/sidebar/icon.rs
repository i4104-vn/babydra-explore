//! Sidebar and file icon resolution:
//! 1. Prioritizes PNG icons from the current icon theme hierarchy.
//! 2. Falls back to full-color theme icons (avoiding monochrome/symbolic icons).
//! 3. Maps MIME types and file extensions to freedesktop theme icons (e.g. PDF -> application-pdf).
//! 4. If the theme does not have the icon, falls back to embedded SVG icons in `babydra-icons`.

use babydra_core::{FileEntry, FileType};
use gdk4::Texture;
use gdk_pixbuf::Pixbuf;
use gio::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

thread_local! {
    static THEME_ICON_INDEX: RefCell<ThemeIconIndex> = RefCell::new(ThemeIconIndex::new());
}

struct ThemeIconIndex {
    theme_name: String,
    png_icons: HashMap<String, (i32, PathBuf)>,
    svg_icons: HashMap<String, (i32, PathBuf)>,
    resolved_cache: HashMap<String, Option<PathBuf>>,
}

impl ThemeIconIndex {
    fn new() -> Self {
        Self {
            theme_name: String::new(),
            png_icons: HashMap::new(),
            svg_icons: HashMap::new(),
            resolved_cache: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        self.theme_name.clear();
        self.png_icons.clear();
        self.svg_icons.clear();
        self.resolved_cache.clear();
    }

    fn populate(&mut self, theme_name: &str) {
        self.clear();
        self.theme_name = theme_name.to_string();
        scan_theme_directories(theme_name, &mut self.png_icons, &mut self.svg_icons);
    }

    fn resolve_candidates(&self, candidates: &[String]) -> Option<PathBuf> {
        // Priority 1: Check PNG icons across candidates
        let mut best_png: Option<(i32, PathBuf)> = None;
        for cand in candidates {
            if let Some((score, path)) = self.png_icons.get(cand) {
                if best_png.as_ref().map(|b| *score > b.0).unwrap_or(true) {
                    best_png = Some((*score, path.clone()));
                }
            }
        }
        if let Some((_, path)) = best_png {
            return Some(path);
        }

        // Priority 2: Check full-color SVG icons (score > 0, avoiding symbolic)
        let mut best_svg: Option<(i32, PathBuf)> = None;
        for cand in candidates {
            if let Some((score, path)) = self.svg_icons.get(cand) {
                if *score > 0 && best_svg.as_ref().map(|b| *score > b.0).unwrap_or(true) {
                    best_svg = Some((*score, path.clone()));
                }
            }
        }
        if let Some((_, path)) = best_svg {
            return Some(path);
        }

        None
    }
}

/// Clears the cached theme icon paths (e.g. when icon theme changes).
pub fn clear_icon_cache() {
    THEME_ICON_INDEX.with(|idx| {
        idx.borrow_mut().clear();
    });
}

/// Retrieves the active icon theme name from GtkSettings or GSettings.
fn get_active_theme_name() -> String {
    if let Some(settings) = gtk4::Settings::default() {
        if let Some(name) = settings.gtk_icon_theme_name() {
            let s = name.trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    let gsettings = gio::Settings::new("org.gnome.desktop.interface");
    let val = gsettings.string("icon-theme");
    let trimmed = val.trim().to_string();
    if !trimmed.is_empty() {
        return trimmed;
    }
    "Adwaita".to_string()
}

/// Returns standard system and user icon search directories.
fn get_search_roots() -> Vec<PathBuf> {
    let home = glib::home_dir();
    let mut roots = Vec::new();
    for p in &[
        home.join(".local/share/icons"),
        home.join(".icons"),
        PathBuf::from("/usr/local/share/icons"),
        PathBuf::from("/usr/share/icons"),
    ] {
        if p.is_dir() {
            roots.push(p.clone());
        }
    }
    roots
}

/// Scores icon size and resolution quality.
fn score_size(s: &str, is_svg: bool) -> i32 {
    if is_svg && s.contains("scalable") {
        100
    } else if s.contains("64") || s.contains("64x64") {
        80
    } else if s.contains("48") || s.contains("48x48") {
        70
    } else if s.contains("32") || s.contains("32x32") {
        60
    } else if s.contains("24") || s.contains("24x24") || s.contains("22") || s.contains("22x22") {
        if s.contains("mime") || s.contains("app") || s.contains("place") {
            45
        } else {
            15
        }
    } else if s.contains("scalable") || s.contains("128") || s.contains("256") || s.contains("512") {
        35
    } else if s.contains("16") || s.contains("16x16") {
        15
    } else {
        25
    }
}

/// Unified scoring function for PNG and SVG paths.
fn score_icon_path(path: &Path, is_svg: bool) -> i32 {
    let s = path.to_string_lossy().to_lowercase();
    if s.contains("symbolic") {
        return -100;
    }
    let mut score = score_size(&s, is_svg);
    if s.contains("mime") || s.contains("mimetypes") {
        score += 15;
    }
    score
}

fn score_png_path(path: &Path) -> i32 {
    score_icon_path(path, false)
}

fn score_svg_path(path: &Path) -> i32 {
    score_icon_path(path, true)
}

/// Scans theme directories to populate available PNG and SVG icons.
fn scan_theme_directories(
    theme_name: &str,
    png_map: &mut HashMap<String, (i32, PathBuf)>,
    svg_map: &mut HashMap<String, (i32, PathBuf)>,
) {
    let roots = get_search_roots();
    let mut themes_to_visit = vec![theme_name.to_string()];
    let mut visited_themes = HashSet::new();
    let mut theme_dirs = Vec::new();

    while let Some(thm) = themes_to_visit.pop() {
        if !visited_themes.insert(thm.clone()) || thm.is_empty() {
            continue;
        }

        for root in &roots {
            let t_dir = root.join(&thm);
            if t_dir.is_dir() {
                theme_dirs.push(t_dir.clone());

                // Read index.theme for Inherits
                let index_path = t_dir.join("index.theme");
                if let Ok(content) = std::fs::read_to_string(index_path) {
                    for line in content.lines() {
                        let line = line.trim();
                        if let Some(inherits) = line.strip_prefix("Inherits=") {
                            for parent in inherits.split(',') {
                                let parent = parent.trim().to_string();
                                if !parent.is_empty() && !visited_themes.contains(&parent) {
                                    themes_to_visit.push(parent);
                                }
                            }
                            break;
                        }
                    }
                }
                break;
            }
        }
    }

    if !visited_themes.contains("hicolor") {
        for root in &roots {
            let hicolor_dir = root.join("hicolor");
            if hicolor_dir.is_dir() {
                theme_dirs.push(hicolor_dir);
                break;
            }
        }
    }

    // Add pixmap paths as fallback
    let home = glib::home_dir();
    let local_pixmaps = home.join(".local/share/pixmaps");
    if local_pixmaps.is_dir() {
        theme_dirs.push(local_pixmaps);
    }
    let sys_pixmaps = PathBuf::from("/usr/share/pixmaps");
    if sys_pixmaps.is_dir() {
        theme_dirs.push(sys_pixmaps);
    }

    // Walk directories and collect icons
    for base in theme_dirs {
        let mut dirs = vec![base];
        while let Some(dir) = dirs.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let path_str = path.to_string_lossy().to_lowercase();
                    if path_str.contains("symbolic") {
                        continue;
                    }
                    dirs.push(path);
                } else if path.is_file() {
                    let ext = match path.extension().and_then(|e| e.to_str()) {
                        Some(e) => e.to_lowercase(),
                        None => continue,
                    };

                    let stem = match path.file_stem().and_then(|s| s.to_str()) {
                        Some(s) => s.to_string(),
                        None => continue,
                    };

                    if ext == "png" {
                        let score = score_png_path(&path);
                        if score > 0 {
                            let entry_val = png_map.entry(stem).or_insert((score, path.clone()));
                            if score > entry_val.0 {
                                *entry_val = (score, path);
                            }
                        }
                    } else if ext == "svg" {
                        let score = score_svg_path(&path);
                        if score > 0 {
                            let entry_val = svg_map.entry(stem).or_insert((score, path.clone()));
                            if score > entry_val.0 {
                                *entry_val = (score, path);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Static alias table for common named icons.
fn get_name_aliases(name: &str) -> &'static [&'static str] {
    match name {
        "folder-download" => &[
            "folder-downloads",
            "folder_download",
            "folder_downloads",
            "download-folder",
            "folder-download-open",
            "download",
        ],
        "folder-documents" => &[
            "folder-document",
            "folders-documents",
            "folder_documents",
            "folder_document",
            "document-folder",
            "folder-documents-open",
            "documents",
        ],
        "folder-pictures" => &[
            "folder-picture",
            "folders-pictures",
            "folder_pictures",
            "folder_picture",
            "folder-images",
            "folder-pictures-open",
            "pictures",
        ],
        "folder-music" => &[
            "folder_music",
            "folders-music",
            "music-folder",
            "folder-music-open",
            "music",
        ],
        "folder-desktop" => &[
            "user-desktop",
            "desktop",
            "user-home",
            "folder_color_default_desktop",
            "default-user-desktop",
        ],
        "folder-videos" => &[
            "folder-video",
            "folders-videos",
            "folder_videos",
            "video-folder",
            "folder-videos-open",
            "videos",
        ],
        "drive-harddisk" => &[
            "drive-harddisk-system",
            "drive-harddisk-root",
            "harddisk",
            "drive",
            "disks",
        ],
        "user-trash" | "trash" => &[
            "user-trash",
            "trashcan_empty",
            "trash-empty",
            "user-trash-full",
            "emptytrash",
            "trash",
        ],
        "drive-removable-media" => &[
            "media-removable",
            "drive-removable",
            "media-flash",
            "usb-drive",
        ],
        "babydra-explore" => &[
            "system-file-manager",
            "file-manager",
            "org.babydra.explore",
            "babydra",
        ],
        "folder" => &[
            "folder_color_default",
            "stock_folder",
            "default-folder",
            "inode-directory",
        ],
        "document-pdf" | "application-pdf" | "pdf" | "gnome-mime-application-pdf" | "x-pdf" => &[
            "application-pdf",
            "pdf",
            "gnome-mime-application-pdf",
            "document-pdf",
            "application-x-pdf",
            "x-office-document",
        ],
        "package-x-generic"
        | "application-zip"
        | "application-x-archive"
        | "application-x-tar" => &[
            "package-x-generic",
            "application-zip",
            "application-x-archive",
            "application-x-tar",
            "application-x-7z-compressed",
            "application-x-rar",
            "application-x-compressed",
            "archive",
            "zip",
        ],
        "application-x-executable" | "system-run" | "application-x-ms-dos-executable" => &[
            "application-x-executable",
            "application-x-ms-dos-executable",
            "system-run",
            "application-default-icon",
        ],
        "image-x-generic" => &["image-x-generic", "image", "image-jpeg", "image-png"],
        "video-x-generic" => &["video-x-generic", "video", "video-mp4", "video-x-matroska"],
        "audio-x-generic" => &["audio-x-generic", "audio", "sound", "audio-mp3"],
        "text-x-generic" => &["text-x-generic", "text-plain", "text-x-preview"],
        "text-x-script" => &["text-x-script", "application-x-shellscript", "text-x-generic"],
        "x-office-document" => &[
            "x-office-document",
            "application-vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application-msword",
            "document",
        ],
        "x-office-spreadsheet" => &[
            "x-office-spreadsheet",
            "application-vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application-vnd.ms-excel",
        ],
        "x-office-presentation" => &[
            "x-office-presentation",
            "application-vnd.openxmlformats-officedocument.presentationml.presentation",
            "application-vnd.ms-powerpoint",
        ],
        _ => &[],
    }
}

/// Static alias table for file extensions to icon theme candidates.
fn get_extension_aliases(ext: &str) -> &'static [&'static str] {
    match ext {
        "pdf" => &[
            "application-pdf",
            "pdf",
            "gnome-mime-application-pdf",
            "document-pdf",
            "application-x-pdf",
            "x-office-document",
        ],
        "doc" | "docx" | "docm" | "dot" | "dotx" | "odt" | "rtf" | "wps" => &[
            "application-vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application-msword",
            "x-office-document",
            "application-vnd.oasis.opendocument.text",
            "libreoffice-writer",
            "wps-office-wps",
            "word",
            "document-word",
        ],
        "xls" | "xlsx" | "xlsm" | "xlt" | "xltx" | "ods" | "csv" | "tsv" | "et" => &[
            "application-vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application-vnd.ms-excel",
            "x-office-spreadsheet",
            "application-vnd.oasis.opendocument.spreadsheet",
            "text-csv",
            "libreoffice-calc",
            "wps-office-et",
            "excel",
            "document-excel",
        ],
        "ppt" | "pptx" | "pptm" | "pot" | "potx" | "odp" | "dps" => &[
            "application-vnd.openxmlformats-officedocument.presentationml.presentation",
            "application-vnd.ms-powerpoint",
            "x-office-presentation",
            "application-vnd.oasis.opendocument.presentation",
            "libreoffice-impress",
            "wps-office-dpp",
            "powerpoint",
            "document-powerpoint",
        ],
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "tgz" | "tbz2" | "zst"
        | "lz4" | "cab" => &[
            "application-zip",
            "package-x-generic",
            "application-x-archive",
            "application-x-tar",
            "application-x-7z-compressed",
            "application-x-rar",
            "application-x-compressed",
            "archive",
            "zip",
        ],
        "deb" => &[
            "application-vnd.debian.binary-package",
            "package-x-generic",
            "deb",
        ],
        "rpm" => &["application-x-rpm", "package-x-generic", "rpm"],
        "apk" => &[
            "application-vnd.android.package-archive",
            "package-x-generic",
            "apk",
        ],
        "iso" | "img" | "raw" | "dmg" => &[
            "application-x-cd-image",
            "application-x-raw-disk-image",
            "media-optical",
            "iso",
        ],
        "exe" | "msi" => &[
            "application-x-ms-dos-executable",
            "application-x-executable",
            "system-run",
            "exe",
        ],
        "sh" | "bash" | "zsh" | "fish" | "bat" | "cmd" => &[
            "application-x-shellscript",
            "text-x-script",
            "application-x-executable",
        ],
        "py" | "pyw" => &[
            "text-x-python",
            "text-x-python3",
            "text-x-script",
            "python",
        ],
        "rs" => &["text-x-rust", "text-rust", "text-x-generic"],
        "c" | "h" => &["text-x-c", "text-x-chdr", "text-x-csrc", "text-x-generic"],
        "cpp" | "cxx" | "cc" | "hpp" => &[
            "text-x-c++",
            "text-x-c++hdr",
            "text-x-c++src",
            "text-x-generic",
        ],
        "js" | "mjs" | "cjs" => &[
            "text-javascript",
            "application-javascript",
            "text-x-javascript",
        ],
        "ts" | "tsx" => &[
            "text-typescript",
            "application-typescript",
            "text-x-typescript",
        ],
        "html" | "htm" => &["text-html", "text-x-generic"],
        "css" | "scss" | "sass" | "less" => &["text-css", "text-x-generic"],
        "json" => &["application-json", "text-x-generic"],
        "xml" => &["application-xml", "text-xml", "text-x-generic"],
        "yaml" | "yml" => &["application-x-yaml", "text-yaml", "text-x-generic"],
        "toml" | "ini" | "conf" | "cfg" => &["text-x-generic", "text-plain"],
        "txt" | "log" => &["text-plain", "text-x-generic"],
        "md" | "markdown" => &[
            "text-markdown",
            "text-x-markdown",
            "text-plain",
            "text-x-generic",
        ],
        "mp3" => &["audio-mp3", "audio-x-generic", "audio", "sound"],
        "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" | "wma" | "mid" | "midi" => {
            &["audio-x-generic", "audio", "sound"]
        }
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" => {
            &["video-x-generic", "video", "video-mp4"]
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "tiff" | "ico"
        | "avif" | "heic" => &["image-x-generic", "image"],
        "ttf" | "otf" | "woff" | "woff2" => &[
            "font-x-generic",
            "application-x-font-ttf",
            "application-x-font-otf",
        ],
        _ => &[],
    }
}

/// Returns candidates/aliases for the given icon name.
fn get_candidates(name: &str) -> Vec<String> {
    let clean = name.strip_suffix("-symbolic").unwrap_or(name);
    let mut list = Vec::new();
    list.push(clean.to_string());
    for alias in get_name_aliases(clean) {
        list.push(alias.to_string());
    }
    list
}

/// Generates ordered candidate icon names for a file or directory based on its extension,
/// MIME type, GIO content type, and given icon_name.
pub fn get_file_candidates(
    path: Option<&Path>,
    name: &str,
    is_dir: bool,
    mime_type: Option<&str>,
    icon_name: Option<&str>,
) -> Vec<String> {
    let mut list = Vec::new();
    let mut seen = HashSet::new();

    let mut add = |item: &str| {
        let clean = item.trim().strip_suffix("-symbolic").unwrap_or(item.trim());
        if !clean.is_empty() && seen.insert(clean.to_string()) {
            list.push(clean.to_string());
        }
    };

    if is_dir {
        let lower = name.to_lowercase();
        let folder_key = if lower.contains("download") || lower.contains("tải về") {
            "folder-download"
        } else if lower.contains("document") || lower.contains("tài liệu") {
            "folder-documents"
        } else if lower.contains("picture")
            || lower.contains("photo")
            || lower.contains("hình")
            || lower.contains("ảnh")
        {
            "folder-pictures"
        } else if lower.contains("music") || lower.contains("nhạc") {
            "folder-music"
        } else if lower.contains("video") || lower.contains("phim") {
            "folder-videos"
        } else if lower.contains("desktop") || lower.contains("bàn làm việc") {
            "folder-desktop"
        } else {
            "folder"
        };
        for a in get_candidates(folder_key) {
            add(&a);
        }
        for a in &[
            "folder",
            "folder_color_default",
            "stock_folder",
            "default-folder",
            "inode-directory",
        ] {
            add(a);
        }
        return list;
    }

    // 1. Extension matching
    let ext_opt = path
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .or_else(|| name.rfind('.').map(|i| &name[i + 1..]))
        .map(|e| e.to_lowercase());

    if let Some(ext) = ext_opt.as_deref() {
        for a in get_extension_aliases(ext) {
            add(a);
        }
    }

    // 2. GIO content type lookup
    let lookup_name = path
        .map(|p| p.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed(name));
    let (ct, _) = gio::content_type_guess(Some(lookup_name.as_ref()), &[]);
    let icon = gio::content_type_get_icon(&ct);
    if let Ok(themed) = icon.downcast::<gio::ThemedIcon>() {
        for tname in themed.names() {
            add(tname.as_str());
        }
    }
    if let Some(gen_name) = gio::content_type_get_generic_icon_name(&ct) {
        add(gen_name.as_str());
    }

    // 3. MIME type conversion
    if let Some(m) = mime_type {
        if let Some(slash_idx) = m.find('/') {
            let dashed = format!("{}-{}", &m[..slash_idx], &m[slash_idx + 1..]);
            add(&dashed);
        }
        add(m);
    }

    // 4. Input icon_name aliases
    if let Some(in_name) = icon_name {
        for a in get_candidates(in_name) {
            add(&a);
        }
    }

    // 5. Broad category fallbacks
    if let Some(m) = mime_type {
        if m.starts_with("image/") {
            add("image-x-generic");
        } else if m.starts_with("video/") {
            add("video-x-generic");
        } else if m.starts_with("audio/") {
            add("audio-x-generic");
        }
    }

    add("text-x-generic");
    list
}

/// Resolves candidates across active icon theme hierarchy.
pub fn resolve_candidates_to_path(candidates: &[String]) -> Option<PathBuf> {
    THEME_ICON_INDEX.with(|idx_cell| {
        let mut idx = idx_cell.borrow_mut();
        let current_theme = get_active_theme_name();

        if idx.theme_name != current_theme {
            idx.populate(&current_theme);
        }

        idx.resolve_candidates(candidates)
    })
}

/// Resolves an icon name to a theme file path.
fn resolve_theme_icon_path(name: &str) -> Option<PathBuf> {
    THEME_ICON_INDEX.with(|idx_cell| {
        let mut idx = idx_cell.borrow_mut();
        let current_theme = get_active_theme_name();

        if idx.theme_name != current_theme {
            idx.populate(&current_theme);
        }

        if let Some(cached) = idx.resolved_cache.get(name) {
            return cached.clone();
        }

        let candidates = get_candidates(name);
        let result = idx.resolve_candidates(&candidates);
        idx.resolved_cache.insert(name.to_string(), result.clone());
        result
    })
}

/// Resolves a file entry to an icon file path in the current icon theme with caching.
pub fn resolve_file_entry_path(entry: &FileEntry) -> Option<PathBuf> {
    let cache_key = if matches!(entry.file_type, FileType::Directory) {
        format!("dir:{}", entry.display_name.to_lowercase())
    } else if let Some(ext) = entry.path.extension().and_then(|e| e.to_str()) {
        format!("ext:{}:{}", ext.to_lowercase(), entry.icon_name)
    } else {
        format!("file:{}:{}", entry.mime_type, entry.icon_name)
    };

    THEME_ICON_INDEX.with(|idx_cell| {
        let mut idx = idx_cell.borrow_mut();
        let current_theme = get_active_theme_name();

        if idx.theme_name != current_theme {
            idx.populate(&current_theme);
        }

        if let Some(cached) = idx.resolved_cache.get(&cache_key) {
            return cached.clone();
        }

        let is_dir = matches!(entry.file_type, FileType::Directory);
        let candidates = get_file_candidates(
            Some(&entry.path),
            &entry.display_name,
            is_dir,
            Some(&entry.mime_type),
            Some(&entry.icon_name),
        );

        let result = idx.resolve_candidates(&candidates);
        idx.resolved_cache.insert(cache_key, result.clone());
        result
    })
}

/// Loads an image file (PNG or SVG) rendered at the exact pixel size using GdkPixbuf and GdkTexture.
fn load_image_from_file(path: &Path, size: i32) -> Option<gtk4::Image> {
    if let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, size, size, true) {
        let texture = Texture::for_pixbuf(&pixbuf);
        let img = gtk4::Image::from_paintable(Some(&texture));
        img.set_pixel_size(size);
        Some(img)
    } else {
        let img = gtk4::Image::from_file(path);
        img.set_pixel_size(size);
        Some(img)
    }
}

/// Helper that creates a GTK Image from resolved path, or fallback to GTK IconTheme / embedded SVG.
fn create_image_from_resolution(
    resolved_path: Option<&Path>,
    candidates: &[String],
    fallback_icon: &str,
    size: i32,
) -> gtk4::Image {
    if let Some(path) = resolved_path {
        if let Some(img) = load_image_from_file(path, size) {
            return img;
        }
    }
    if let Some(disp) = gdk4::Display::default() {
        let theme = gtk4::IconTheme::for_display(&disp);
        for cand in candidates {
            if theme.has_icon(cand) {
                let img = gtk4::Image::from_icon_name(cand);
                img.set_pixel_size(size);
                return img;
            }
        }
    }
    let img = babydra_ui_kit::ui::icon::get_fallback_icon(fallback_icon, "text-x-generic");
    img.set_pixel_size(size);
    img
}

/// Creates a sidebar icon widget.
pub fn get_sidebar_icon(icon_name: &str, size: i32) -> gtk4::Image {
    if (icon_name.starts_with('/') || icon_name.starts_with('.'))
        && Path::new(icon_name).is_file()
    {
        if let Some(img) = load_image_from_file(Path::new(icon_name), size) {
            return img;
        }
    }

    let candidates = get_candidates(icon_name);
    let resolved = resolve_theme_icon_path(icon_name);
    create_image_from_resolution(resolved.as_deref(), &candidates, icon_name, size)
}

/// Returns a GTK Image for a `FileEntry`.
pub fn get_file_entry_icon(entry: &FileEntry, size: i32) -> gtk4::Image {
    let resolved = resolve_file_entry_path(entry);
    let is_dir = matches!(entry.file_type, FileType::Directory);
    let candidates = get_file_candidates(
        Some(&entry.path),
        &entry.display_name,
        is_dir,
        Some(&entry.mime_type),
        Some(&entry.icon_name),
    );
    create_image_from_resolution(resolved.as_deref(), &candidates, &entry.icon_name, size)
}

/// Sets the image source for an existing GTK Image widget for a `FileEntry`.
pub fn set_file_entry_icon(img: &gtk4::Image, entry: &FileEntry, size: i32) {
    if let Some(path) = resolve_file_entry_path(entry) {
        if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&path, size, size, true) {
            let texture = Texture::for_pixbuf(&pixbuf);
            img.set_icon_name(None);
            img.set_paintable(Some(&texture));
            img.set_pixel_size(size);
            return;
        } else {
            img.set_icon_name(None);
            img.set_from_file(Some(path.to_string_lossy().as_ref()));
            img.set_pixel_size(size);
            return;
        }
    }

    img.set_paintable(gtk4::gdk::Paintable::NONE);
    let is_dir = matches!(entry.file_type, FileType::Directory);
    let candidates = get_file_candidates(
        Some(&entry.path),
        &entry.display_name,
        is_dir,
        Some(&entry.mime_type),
        Some(&entry.icon_name),
    );

    if let Some(disp) = gdk4::Display::default() {
        let theme = gtk4::IconTheme::for_display(&disp);
        for cand in &candidates {
            if theme.has_icon(cand) {
                img.set_icon_name(Some(cand));
                img.set_pixel_size(size);
                return;
            }
        }
    }

    babydra_ui_kit::ui::icon::set_fallback_icon(img, &entry.icon_name, "text-x-generic");
    img.set_pixel_size(size);
}

/// Resolves an icon for a file or directory path (used e.g. in Properties dialog).
pub fn get_icon_for_path(path: &Path, is_dir: bool, size: i32) -> gtk4::Image {
    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    let candidates = get_file_candidates(Some(path), &filename, is_dir, None, None);
    let resolved = resolve_candidates_to_path(&candidates);
    let fallback = if is_dir { "folder" } else { "text-x-generic" };
    create_image_from_resolution(resolved.as_deref(), &candidates, fallback, size)
}

/// Resolves a Texture paintable for drag-and-drop icon or thumbnail representation.
pub fn get_icon_paintable(icon_name: &str, file_path: &Path, size: i32) -> Option<Texture> {
    let is_dir = file_path.is_dir();
    let filename = file_path.file_name().unwrap_or_default().to_string_lossy();
    let candidates =
        get_file_candidates(Some(file_path), &filename, is_dir, None, Some(icon_name));
    if let Some(icon_path) = resolve_candidates_to_path(&candidates) {
        if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&icon_path, size, size, true) {
            return Some(Texture::for_pixbuf(&pixbuf));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidates_aliases() {
        let dl = get_candidates("folder-download");
        assert!(dl.contains(&"folder-downloads".to_string()));
        assert!(dl.contains(&"download".to_string()));

        let desk = get_candidates("folder-desktop");
        assert!(desk.contains(&"user-desktop".to_string()));
        assert!(desk.contains(&"desktop".to_string()));

        let trash = get_candidates("user-trash");
        assert!(trash.contains(&"trash".to_string()));
        assert!(trash.contains(&"trashcan_empty".to_string()));

        let pdf = get_candidates("document-pdf");
        assert!(pdf.contains(&"application-pdf".to_string()));
        assert!(pdf.contains(&"pdf".to_string()));
    }

    #[test]
    fn test_file_candidates_pdf() {
        let cands = get_file_candidates(
            Some(Path::new("/tmp/3000.pdf")),
            "3000.pdf",
            false,
            Some("application/pdf"),
            Some("document-pdf"),
        );
        assert!(cands.contains(&"application-pdf".to_string()));
        assert!(cands.contains(&"pdf".to_string()));
    }

    #[test]
    fn test_png_and_svg_scoring() {
        let png_48 = score_png_path(Path::new("/usr/share/icons/theme/48x48/places/folder.png"));
        let png_16 = score_png_path(Path::new("/usr/share/icons/theme/16x16/places/folder.png"));
        assert!(png_48 > png_16);

        let svg_scalable =
            score_svg_path(Path::new("/usr/share/icons/theme/places/scalable/folder.svg"));
        let svg_22 = score_svg_path(Path::new("/usr/share/icons/theme/places/22/folder.svg"));
        assert!(svg_scalable > svg_22);

        let svg_symbolic = score_svg_path(Path::new(
            "/usr/share/icons/theme/places/symbolic/folder-symbolic.svg",
        ));
        assert_eq!(svg_symbolic, -100);
    }
}
