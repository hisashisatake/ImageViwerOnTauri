use std::path::{Path, PathBuf};

pub fn sanitize_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "archive".to_string()
    } else {
        sanitized
    }
}

pub fn is_supported_image(path: &Path) -> bool {
    let ext = if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        ext.to_ascii_lowercase()
    } else {
        return false;
    };
    return matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "bmp" | "avif"
    );
}

pub fn is_supported_archive(path: &Path) -> bool {
    let ext = if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        ext.to_ascii_lowercase()
    } else {
        return false;
    };
    return matches!(ext.as_str(), "zip" | "cbz");
}

pub fn is_supported_rar(path: &Path) -> bool {
    let ext = if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        ext.to_ascii_lowercase()
    } else {
        return false;
    };
    return matches!(ext.as_str(), "rar");
}

pub fn is_supported_pdf(path: &Path) -> bool {
    let ext = if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        ext.to_ascii_lowercase()
    } else {
        return false;
    };
    return matches!(ext.as_str(), "pdf");
}

pub fn safe_relative_path(path: &Path) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        if let std::path::Component::Normal(segment) = component {
            clean.push(segment);
        }
    }
    if clean.as_os_str().is_empty() {
        None
    } else {
        Some(clean)
    }
}

pub fn entry_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string())
}
