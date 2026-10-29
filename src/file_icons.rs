use gpui_kit::{AssetSource, SharedString, assets::AllAssets, *};
use std::{borrow::Cow, path::Path};

pub const APP_ICON: &str = "app-icon/patchlook-v2.png";

const ICONS: &[(&str, &[u8])] = &[
    (
        "ui-icons/folder.svg",
        include_bytes!("../assets/ui-icons/folder.svg"),
    ),
    (
        "file-icons/rust.svg",
        include_bytes!("../assets/file-icons/rust.svg"),
    ),
    (
        "file-icons/swift.svg",
        include_bytes!("../assets/file-icons/swift.svg"),
    ),
    (
        "file-icons/typescript.svg",
        include_bytes!("../assets/file-icons/typescript.svg"),
    ),
    (
        "file-icons/javascript.svg",
        include_bytes!("../assets/file-icons/javascript.svg"),
    ),
    (
        "file-icons/react.svg",
        include_bytes!("../assets/file-icons/react.svg"),
    ),
    (
        "file-icons/react_ts.svg",
        include_bytes!("../assets/file-icons/react_ts.svg"),
    ),
    (
        "file-icons/python.svg",
        include_bytes!("../assets/file-icons/python.svg"),
    ),
    (
        "file-icons/go.svg",
        include_bytes!("../assets/file-icons/go.svg"),
    ),
    (
        "file-icons/java.svg",
        include_bytes!("../assets/file-icons/java.svg"),
    ),
    (
        "file-icons/kotlin.svg",
        include_bytes!("../assets/file-icons/kotlin.svg"),
    ),
    (
        "file-icons/c.svg",
        include_bytes!("../assets/file-icons/c.svg"),
    ),
    (
        "file-icons/cpp.svg",
        include_bytes!("../assets/file-icons/cpp.svg"),
    ),
    (
        "file-icons/csharp.svg",
        include_bytes!("../assets/file-icons/csharp.svg"),
    ),
    (
        "file-icons/ruby.svg",
        include_bytes!("../assets/file-icons/ruby.svg"),
    ),
    (
        "file-icons/php.svg",
        include_bytes!("../assets/file-icons/php.svg"),
    ),
    (
        "file-icons/dart.svg",
        include_bytes!("../assets/file-icons/dart.svg"),
    ),
    (
        "file-icons/vue.svg",
        include_bytes!("../assets/file-icons/vue.svg"),
    ),
    (
        "file-icons/svelte.svg",
        include_bytes!("../assets/file-icons/svelte.svg"),
    ),
    (
        "file-icons/html.svg",
        include_bytes!("../assets/file-icons/html.svg"),
    ),
    (
        "file-icons/css.svg",
        include_bytes!("../assets/file-icons/css.svg"),
    ),
    (
        "file-icons/sass.svg",
        include_bytes!("../assets/file-icons/sass.svg"),
    ),
    (
        "file-icons/markdown.svg",
        include_bytes!("../assets/file-icons/markdown.svg"),
    ),
    (
        "file-icons/json.svg",
        include_bytes!("../assets/file-icons/json.svg"),
    ),
    (
        "file-icons/yaml.svg",
        include_bytes!("../assets/file-icons/yaml.svg"),
    ),
    (
        "file-icons/toml.svg",
        include_bytes!("../assets/file-icons/toml.svg"),
    ),
    (
        "file-icons/xml.svg",
        include_bytes!("../assets/file-icons/xml.svg"),
    ),
    (
        "file-icons/console.svg",
        include_bytes!("../assets/file-icons/console.svg"),
    ),
    (
        "file-icons/docker.svg",
        include_bytes!("../assets/file-icons/docker.svg"),
    ),
    (
        "file-icons/git.svg",
        include_bytes!("../assets/file-icons/git.svg"),
    ),
    (
        "file-icons/database.svg",
        include_bytes!("../assets/file-icons/database.svg"),
    ),
    (
        "file-icons/image.svg",
        include_bytes!("../assets/file-icons/image.svg"),
    ),
    (
        "file-icons/pdf.svg",
        include_bytes!("../assets/file-icons/pdf.svg"),
    ),
    (
        "file-icons/document.svg",
        include_bytes!("../assets/file-icons/document.svg"),
    ),
    (
        "file-icons/zip.svg",
        include_bytes!("../assets/file-icons/zip.svg"),
    ),
    (
        "file-icons/settings.svg",
        include_bytes!("../assets/file-icons/settings.svg"),
    ),
];

pub struct AppAssets;
impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/app-icon/patchlook-v2.png"
            ))));
        }
        if let Some((_, bytes)) = ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        AllAssets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = AllAssets.list(path)?;
        if APP_ICON.starts_with(path) {
            paths.push(APP_ICON.into());
        }
        paths.extend(
            ICONS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| (*name).into()),
        );
        Ok(paths)
    }
}

pub fn file_icon(path: &Path) -> Img {
    img(format!("file-icons/{}.svg", icon_name(path)))
        .size(px(16.))
        .flex_none()
}

fn icon_name(path: &Path) -> &'static str {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    match filename.as_str() {
        "dockerfile" | "containerfile" | "docker-compose.yml" | "docker-compose.yaml" => {
            return "docker";
        }
        ".gitignore" | ".gitattributes" | ".gitmodules" => return "git",
        "makefile" | "cmakelists.txt" | ".editorconfig" | ".env" => return "settings",
        _ => {}
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "swift" => "swift",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "react_ts",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "react",
        "py" | "pyi" | "pyw" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "rb" | "rake" => "ruby",
        "php" => "php",
        "dart" => "dart",
        "vue" => "vue",
        "svelte" => "svelte",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "sass",
        "md" | "markdown" | "mdx" => "markdown",
        "json" | "jsonc" | "json5" | "ipynb" | "xcstrings" => "json",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "xml" | "plist" | "xaml" | "storyboard" | "xib" => "xml",
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" => "console",
        "sql" | "db" | "sqlite" | "sqlite3" => "database",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" | "tif" | "tiff" => "image",
        "pdf" => "pdf",
        "zip" | "gz" | "tgz" | "tar" | "7z" | "rar" => "zip",
        "ini" | "cfg" | "conf" | "properties" => "settings",
        _ => "document",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[test]
    fn recognized_file_types_have_bundled_assets() {
        for (path, expected) in [
            ("src/main.rs", "rust"),
            ("View.SWIFT", "swift"),
            ("page.tsx", "react_ts"),
            ("AGENTS.md", "markdown"),
            ("index.html", "html"),
            ("Cargo.toml", "toml"),
            ("Dockerfile", "docker"),
            (".gitignore", "git"),
            ("unknown.xyz", "document"),
        ] {
            let name = icon_name(Path::new(path));
            assert_eq!(name, expected);
            assert!(
                AppAssets
                    .load(&format!("file-icons/{name}.svg"))
                    .unwrap()
                    .is_some()
            );
        }
    }
}
