use super::defaults;
use super::types::Template;
use std::path::PathBuf;
use tracing::{debug, info, warn};
use once_cell::sync::Lazy;
use std::sync::RwLock;

// Global storage for the bundled templates directory path
static BUNDLED_TEMPLATES_DIR: Lazy<RwLock<Option<PathBuf>>> = Lazy::new(|| RwLock::new(None));

/// Set the bundled templates directory path (called once at app startup)
pub fn set_bundled_templates_dir(path: PathBuf) {
    info!("Bundled templates directory set to: {:?}", path);
    if let Ok(mut dir) = BUNDLED_TEMPLATES_DIR.write() {
        *dir = Some(path);
    }
}

/// Get the user's custom templates directory path
///
/// Returns the platform-specific application data directory for custom templates:
/// - macOS: ~/Library/Application Support/Noetis/templates/
/// - Windows: %APPDATA%\Noetis\templates\
/// - Linux: ~/.config/Noetis/templates/
fn get_custom_templates_dir() -> Option<PathBuf> {
    let mut path = dirs::data_dir()?;
    path.push("Noetis");
    path.push("templates");
    Some(path)
}

/// Template directories in priority order: org library (managed policy), user custom,
/// bundled app resources. Built-in embedded templates are the final fallback.
fn template_dirs() -> Vec<PathBuf> {
    let bundled = BUNDLED_TEMPLATES_DIR.read().ok().and_then(|d| d.clone());
    [crate::policy::templates_dir(), get_custom_templates_dir(), bundled]
        .into_iter()
        .flatten()
        .collect()
}

/// Ids become file names, so only allow plain names (no `..`, slashes, drive letters).
fn is_safe_id(template_id: &str) -> bool {
    !template_id.is_empty()
        && template_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Load and parse a template by identifier: org library, then user custom, then
/// bundled, then built-in.
pub fn get_template(template_id: &str) -> Result<Template, String> {
    info!("Loading template: {}", template_id);
    if !is_safe_id(template_id) {
        return Err(format!("Invalid template id '{}'", template_id));
    }

    let from_disk = template_dirs().into_iter().find_map(|dir| {
        let path = dir.join(format!("{}.json", template_id));
        let content = std::fs::read_to_string(&path).ok()?;
        debug!("Using template '{}' from {:?}", template_id, path);
        Some(content)
    });

    let json_content = match from_disk {
        Some(content) => content,
        None => match defaults::get_builtin_template(template_id) {
            Some(builtin) => builtin.to_string(),
            None => {
                return Err(format!(
                    "Template '{}' not found. Available templates: {}",
                    template_id,
                    list_template_ids().join(", ")
                ))
            }
        },
    };

    validate_and_parse_template(&json_content)
}

/// Validate and parse template JSON
///
/// # Arguments
/// * `json_content` - Raw JSON string
///
/// # Returns
/// Parsed and validated Template struct
pub fn validate_and_parse_template(json_content: &str) -> Result<Template, String> {
    let template: Template = serde_json::from_str(json_content)
        .map_err(|e| format!("Failed to parse template JSON: {}", e))?;

    template.validate()?;

    Ok(template)
}

/// Where a template id resolves from: "org", "custom", or "builtin" (bundled + embedded).
/// Only "custom" templates are editable by the user.
pub fn template_source(template_id: &str) -> &'static str {
    let exists_in = |dir: Option<PathBuf>| {
        dir.map_or(false, |d| d.join(format!("{}.json", template_id)).is_file())
    };
    if exists_in(crate::policy::templates_dir()) {
        "org"
    } else if exists_in(get_custom_templates_dir()) {
        "custom"
    } else {
        "builtin"
    }
}

/// Save (create or overwrite) a user template. A custom template with a built-in id
/// shadows the built-in one; org templates always win and can't be shadowed.
pub fn save_custom_template(template_id: &str, template: &Template) -> Result<(), String> {
    if !is_safe_id(template_id) {
        return Err(format!("Invalid template id '{}'", template_id));
    }
    if template_source(template_id) == "org" {
        return Err("This template is managed by your organization".into());
    }
    template.validate()?;
    let dir = get_custom_templates_dir().ok_or("No user data directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {:?}: {}", dir, e))?;
    let json = serde_json::to_string_pretty(template).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{}.json", template_id)), json)
        .map_err(|e| format!("Failed to save template: {}", e))
}

pub fn delete_custom_template(template_id: &str) -> Result<(), String> {
    if !is_safe_id(template_id) || template_source(template_id) != "custom" {
        return Err(format!("'{}' is not a user template", template_id));
    }
    let dir = get_custom_templates_dir().ok_or("No user data directory")?;
    std::fs::remove_file(dir.join(format!("{}.json", template_id)))
        .map_err(|e| format!("Failed to delete template: {}", e))
}

/// List all available template identifiers (built-in plus every template directory).
pub fn list_template_ids() -> Vec<String> {
    let mut ids: Vec<String> = defaults::list_builtin_template_ids()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    for dir in template_dirs() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                debug!("Skipping templates directory {:?}: {}", dir, e);
                continue;
            }
        };
        for entry in entries.flatten() {
            if let Some(id) = entry.file_name().to_str().and_then(|f| f.strip_suffix(".json")) {
                if is_safe_id(id) && !ids.iter().any(|i| i == id) {
                    ids.push(id.to_string());
                }
            }
        }
    }

    ids.sort();
    ids
}

/// List all available templates with their metadata
///
/// Returns a list of (id, name, description) tuples
pub fn list_templates() -> Vec<(String, String, String)> {
    let mut templates = Vec::new();

    for id in list_template_ids() {
        match get_template(&id) {
            Ok(template) => {
                templates.push((id, template.name, template.description));
            }
            Err(e) => {
                warn!("Failed to load template '{}': {}", id, e);
            }
        }
    }

    templates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_builtin_template() {
        let template = get_template("daily_standup");
        assert!(template.is_ok());

        let template = template.unwrap();
        assert_eq!(template.name, "Daily Standup");
        assert!(!template.sections.is_empty());
    }

    #[test]
    fn test_get_nonexistent_template() {
        let result = get_template("nonexistent_template");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_template_ids() {
        let ids = list_template_ids();
        assert!(ids.contains(&"daily_standup".to_string()));
        assert!(ids.contains(&"standard_meeting".to_string()));
    }

    #[test]
    fn test_rejects_path_traversal_ids() {
        assert!(get_template("../../secrets").is_err());
        assert!(get_template(r"C:\x").is_err());
        assert!(!is_safe_id(""));
        assert!(is_safe_id("daily_standup-2"));
    }

    #[test]
    fn test_validate_invalid_json() {
        let result = validate_and_parse_template("invalid json");
        assert!(result.is_err());
    }
}
