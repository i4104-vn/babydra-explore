//! Sidebar icon resolution:
//! 1. Prioritizes PNG icons from the current icon theme hierarchy.
//! 2. Falls back to full-color theme icons (avoiding monochrome/symbolic icons).
//! 3. If the theme does not have the icon, falls back to embedded SVG icons in `babydra-icons`.

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
    let user_local = home.join(".local/share/icons");
    if user_local.is_dir() {
        roots.push(user_local);
    }
    let user_icons = home.join(".icons");
    if user_icons.is_dir() {
        roots.push(user_icons);
    }
    let sys_local = PathBuf::from("/usr/local/share/icons");
    if sys_local.is_dir() {
        roots.push(sys_local);
    }
    let sys_icons = PathBuf::from("/usr/share/icons");
    if sys_icons.is_dir() {
        roots.push(sys_icons);
    }
    roots
}

/// Scores a PNG path based on its resolution / directory name.
/// Prefer resolutions around 32-64px for clean scaling down to 22-26px.
fn score_png_path(path: &Path) -> i32 {
    let s = path.to_string_lossy().to_lowercase();
    if s.contains("symbolic") {
        return -100;
    }
    if s.contains("48") || s.contains("48x48") {
        60
    } else if s.contains("64") || s.contains("64x64") {
        55
    } else if s.contains("32") || s.contains("32x32") {
        50
    } else if s.contains("24") || s.contains("24x24") || s.contains("22") || s.contains("22x22") {
        45
    } else if s.contains("scalable") || s.contains("128") || s.contains("256") {
        35
    } else if s.contains("16") || s.contains("16x16") {
        15
    } else {
        25
    }
}

/// Scores an SVG path to prioritize full-color, scalable artwork over small monochrome line icons.
fn score_svg_path(path: &Path) -> i32 {
    let s = path.to_string_lossy().to_lowercase();
    if s.contains("symbolic") {
        return -100;
    }
    if s.contains("scalable") {
        100
    } else if s.contains("64") || s.contains("64x64") {
        80
    } else if s.contains("48") || s.contains("48x48") {
        70
    } else if s.contains("32") || s.contains("32x32") {
        60
    } else if s.contains("24")
        || s.contains("24x24")
        || s.contains("22")
        || s.contains("22x22")
        || s.contains("16")
        || s.contains("16x16")
    {
        // 16/22/24 directories in many themes contain monochrome single-color icons
        5
    } else {
        30
    }
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

/// Returns candidates/aliases for the given icon name.
fn get_candidates(name: &str) -> Vec<String> {
    let clean = name.strip_suffix("-symbolic").unwrap_or(name);
    let mut list = Vec::new();
    list.push(clean.to_string());

    match clean {
        "folder-download" => {
            for a in &[
                "folder-downloads",
                "folder_download",
                "folder_downloads",
                "download-folder",
                "folder-download-open",
                "download",
            ] {
                list.push(a.to_string());
            }
        }
        "folder-documents" => {
            for a in &[
                "folder-document",
                "folders-documents",
                "folder_documents",
                "folder_document",
                "document-folder",
                "folder-documents-open",
                "documents",
            ] {
                list.push(a.to_string());
            }
        }
        "folder-pictures" => {
            for a in &[
                "folder-picture",
                "folders-pictures",
                "folder_pictures",
                "folder_picture",
                "folder-images",
                "folder-pictures-open",
                "pictures",
            ] {
                list.push(a.to_string());
            }
        }
        "folder-music" => {
            for a in &[
                "folder_music",
                "folders-music",
                "music-folder",
                "folder-music-open",
                "music",
            ] {
                list.push(a.to_string());
            }
        }
        "folder-desktop" => {
            for a in &[
                "user-desktop",
                "desktop",
                "user-home",
                "folder_color_default_desktop",
                "default-user-desktop",
            ] {
                list.push(a.to_string());
            }
        }
        "folder-videos" => {
            for a in &[
                "folder-video",
                "folders-videos",
                "folder_videos",
                "video-folder",
                "folder-videos-open",
                "videos",
            ] {
                list.push(a.to_string());
            }
        }
        "drive-harddisk" => {
            for a in &[
                "drive-harddisk-system",
                "drive-harddisk-root",
                "harddisk",
                "drive",
                "disks",
            ] {
                list.push(a.to_string());
            }
        }
        "user-trash" | "trash" => {
            for a in &[
                "user-trash",
                "trashcan_empty",
                "trash-empty",
                "user-trash-full",
                "emptytrash",
                "trash",
            ] {
                list.push(a.to_string());
            }
        }
        "drive-removable-media" => {
            for a in &[
                "media-removable",
                "drive-removable",
                "media-flash",
                "usb-drive",
            ] {
                list.push(a.to_string());
            }
        }
        "babydra-explore" => {
            for a in &[
                "system-file-manager",
                "file-manager",
                "org.babydra.explore",
                "babydra",
            ] {
                list.push(a.to_string());
            }
        }
        "folder" => {
            for a in &[
                "folder_color_default",
                "stock_folder",
                "default-folder",
            ] {
                list.push(a.to_string());
            }
        }
        _ => {}
    }

    list
}

/// Resolves an icon to a theme file path.
/// Prioritizes PNG icons first, then full-color SVG icons.
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

        // 1. Priority 1: Check PNG icons across candidates
        let mut best_png: Option<(i32, PathBuf)> = None;
        for cand in &candidates {
            if let Some((score, path)) = idx.png_icons.get(cand) {
                if best_png.as_ref().map(|b| *score > b.0).unwrap_or(true) {
                    best_png = Some((*score, path.clone()));
                }
            }
        }

        if let Some((_, path)) = best_png {
            idx.resolved_cache.insert(name.to_string(), Some(path.clone()));
            return Some(path);
        }

        // 2. Priority 2: Check full-color SVG icons (score >= 30, avoiding 16/22/24 line icons)
        let mut best_svg: Option<(i32, PathBuf)> = None;
        for cand in &candidates {
            if let Some((score, path)) = idx.svg_icons.get(cand) {
                if *score >= 30 && best_svg.as_ref().map(|b| *score > b.0).unwrap_or(true) {
                    best_svg = Some((*score, path.clone()));
                }
            }
        }

        if let Some((_, path)) = best_svg {
            idx.resolved_cache.insert(name.to_string(), Some(path.clone()));
            return Some(path);
        }

        idx.resolved_cache.insert(name.to_string(), None);
        None
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

/// Creates a sidebar icon widget.
/// - Checks if `icon_name` is an existing file path.
/// - Searches the icon theme hierarchy for a matching PNG image (or color SVG).
/// - If not found in themes, falls back to the embedded SVG in `babydra-icons`.
pub fn get_sidebar_icon(icon_name: &str, size: i32) -> gtk4::Image {
    // 1. Direct file path
    if (icon_name.starts_with('/') || icon_name.starts_with('.'))
        && Path::new(icon_name).is_file()
    {
        if let Some(img) = load_image_from_file(Path::new(icon_name), size) {
            return img;
        }
    }

    // 2. Search theme hierarchy for PNG or full-color icon
    if let Some(path) = resolve_theme_icon_path(icon_name) {
        if let Some(img) = load_image_from_file(&path, size) {
            return img;
        }
    }

    // 3. Fallback to embedded SVG from babydra-icons
    babydra_ui_kit::ui::icon::get_icon(icon_name, size)
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
    }

    #[test]
    fn test_png_and_svg_scoring() {
        let png_48 = score_png_path(Path::new("/usr/share/icons/theme/48x48/places/folder.png"));
        let png_16 = score_png_path(Path::new("/usr/share/icons/theme/16x16/places/folder.png"));
        assert!(png_48 > png_16);

        let svg_scalable = score_svg_path(Path::new("/usr/share/icons/theme/places/scalable/folder.svg"));
        let svg_22 = score_svg_path(Path::new("/usr/share/icons/theme/places/22/folder.svg"));
        assert!(svg_scalable > svg_22);

        let svg_symbolic = score_svg_path(Path::new("/usr/share/icons/theme/places/symbolic/folder-symbolic.svg"));
        assert_eq!(svg_symbolic, -100);
    }
}
