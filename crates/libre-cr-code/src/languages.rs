//! Language detection — cheap extension-based heuristic. Per spec §
//! "language-agnostic" tools and `detect_languages`.

/// Map an extension (without leading dot, lowercase) to a language name.
pub fn lang_for_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "Rust",
        "go" => "Go",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "py" | "pyi" => "Python",
        "java" => "Java",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "C++",
        "rb" => "Ruby",
        "php" => "PHP",
        "sh" | "bash" => "Bash",
        "cs" => "C#",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "scala" => "Scala",
        "md" => "Markdown",
        "yml" | "yaml" => "YAML",
        "json" => "JSON",
        "toml" => "TOML",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "sql" => "SQL",
        _ => return None,
    })
}

/// Detect the language for a file by extension; returns the language name or
/// `"Unknown"`.
pub fn language_of_file(file: &str) -> &'static str {
    let ext = std::path::Path::new(file)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    ext.and_then(|e| lang_for_extension(&e))
        .unwrap_or("Unknown")
}

/// Quick binary heuristic: NUL byte in the first 8 KiB.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map() {
        assert_eq!(language_of_file("main.rs"), "Rust");
        assert_eq!(language_of_file("foo.py"), "Python");
        assert_eq!(language_of_file("script.sh"), "Bash");
        assert_eq!(language_of_file("data.bin"), "Unknown");
    }

    #[test]
    fn binary_detection() {
        assert!(looks_binary(b"hello\x00world"));
        assert!(!looks_binary(b"plain text"));
    }
}
