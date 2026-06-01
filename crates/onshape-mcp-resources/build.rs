// Build scripts communicate errors by panicking — these lints are
// inappropriate for build-time code.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Build script for `onshape-mcp-resources`.
//!
//! Scans `docs/src/mcp-resources/*/index.md` for resource groups, parses each
//! index to extract resource entries, reads the referenced markdown files, and
//! generates a Rust source file with the full resource catalog embedded as
//! compile-time constants.
//!
//! ## Expected index.md format
//!
//! ```markdown
//! # Group Title
//!
//! Optional description paragraph.
//!
//! ## Section Heading (ignored)
//!
//! - [Resource Title](filename.md) — Description of the resource
//! - [Another Title](another.md) — Another description
//! ```
//!
//! The parser extracts from each list item:
//! - **title**: the link text
//! - **name**: the link target filename without `.md` extension
//! - **description**: text after ` — ` (em dash)

use std::env;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::Literal;
use pulldown_cmark::{Event, LinkType, Parser, Tag, TagEnd};

/// A single resource entry extracted from an index.md file.
struct ResourceEntry {
    title: String,
    name: String,
    description: String,
    content: String,
}

/// Read a markdown file referenced by a list-item link and build a [`ResourceEntry`].
fn read_resource_entry(
    dir: &Path,
    link_url: &str,
    link_title: &str,
    item_text: &str,
) -> ResourceEntry {
    // Extract description: text after " — " (em dash)
    let full_text = item_text.trim();
    let description = full_text
        .split(" — ")
        .nth(1)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // Derive name from filename: "shaded-views.md" -> "shaded-views"
    let name = link_url.strip_suffix(".md").unwrap_or(link_url).to_string();

    // Reject anything other than normal/curdir path components to prevent
    // reading files outside the group directory at build time.
    // A whitelist is used instead of a blacklist because on Windows,
    // paths like `\foo` or `C:foo` bypass `is_absolute()` yet cause
    // `PathBuf::join` to replace the base path.
    let link_path = Path::new(link_url);
    assert!(
        link_path.components().all(|c| matches!(
            c,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )),
        "Resource link URL must be relative and non-traversal, got: {link_url}"
    );

    // Read the referenced markdown file
    let content_path = dir.join(link_url);
    let content = fs::read_to_string(&content_path).unwrap_or_else(|e| {
        panic!(
            "Failed to read resource file {}: {e}",
            content_path.display()
        )
    });

    // Emit rerun-if-changed for the content file
    println!("cargo:rerun-if-changed={}", content_path.display());

    ResourceEntry {
        title: link_title.to_string(),
        name,
        description,
        content,
    }
}

/// Parse an index.md file to extract resource entries, then read each
/// referenced markdown file from the same directory.
fn parse_index(index_path: &Path) -> Vec<ResourceEntry> {
    let index_text = fs::read_to_string(index_path)
        .unwrap_or_else(|e| panic!("Failed to read index file {}: {e}", index_path.display()));

    let dir = index_path
        .parent()
        .expect("index.md should have a parent directory");

    let parser = Parser::new(&index_text);

    let mut entries = Vec::new();
    let mut in_link = false;
    let mut current_link_url = String::new();
    let mut current_link_title = String::new();
    let mut current_item_text = String::new();
    let mut in_list_item = false;

    for event in parser {
        match event {
            Event::Start(Tag::Item) => {
                in_list_item = true;
                current_link_url.clear();
                current_link_title.clear();
                current_item_text.clear();
            }
            Event::End(TagEnd::Item) => {
                if in_list_item && !current_link_url.is_empty() {
                    entries.push(read_resource_entry(
                        dir,
                        &current_link_url,
                        &current_link_title,
                        &current_item_text,
                    ));
                }
                in_list_item = false;
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                if in_list_item {
                    if link_type == LinkType::Inline {
                        in_link = true;
                        current_link_url = dest_url.to_string();
                    } else {
                        panic!(
                            "Non-inline link type {link_type:?} in list item is \
                             not supported, use inline links [text](url) instead: {}",
                            index_path.display()
                        );
                    }
                }
            }
            Event::End(TagEnd::Link) => {
                in_link = false;
            }
            Event::Text(text) => {
                if in_list_item {
                    current_item_text.push_str(&text);
                    if in_link {
                        current_link_title.push_str(&text);
                    }
                }
            }
            Event::Code(code) if in_list_item => {
                current_item_text.push('`');
                current_item_text.push_str(&code);
                current_item_text.push('`');
                if in_link {
                    current_link_title.push('`');
                    current_link_title.push_str(&code);
                    current_link_title.push('`');
                }
            }
            _ => {}
        }
    }

    entries
}

/// Format a string as a Rust string literal with proper escaping.
///
/// Uses `proc_macro2::Literal::string()` to produce a correctly escaped
/// string literal, avoiding the fragility of manually choosing raw string
/// delimiters.
fn rust_string_literal(s: &str) -> String {
    Literal::string(s).to_string()
}

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let resources_dir = manifest_dir.join("resources");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    // Watch the top-level mcp-resources directory for new groups
    println!("cargo:rerun-if-changed={}", resources_dir.display());

    let mut all_entries: Vec<(String, ResourceEntry)> = Vec::new();

    // Discover groups: each subdirectory with an index.md
    if resources_dir.is_dir() {
        let mut group_dirs: Vec<_> = fs::read_dir(&resources_dir)
            .unwrap_or_else(|e| {
                panic!(
                    "Failed to read mcp-resources directory {}: {e}",
                    resources_dir.display()
                )
            })
            .filter_map(|entry| {
                let entry = entry.unwrap_or_else(|e| {
                    panic!("Failed to read entry in {}: {e}", resources_dir.display())
                });
                let path = entry.path();
                if path.is_dir() && path.join("index.md").is_file() {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        // Sort for deterministic output
        group_dirs.sort();

        for group_dir in group_dirs {
            let group_name = group_dir
                .file_name()
                .expect("group dir should have a name")
                .to_string_lossy()
                .to_string();

            let index_path = group_dir.join("index.md");
            println!("cargo:rerun-if-changed={}", index_path.display());

            // Watch the group directory for new files
            println!("cargo:rerun-if-changed={}", group_dir.display());

            let entries = parse_index(&index_path);
            for entry in entries {
                all_entries.push((group_name.clone(), entry));
            }
        }
    } else {
        panic!(
            "mcp-resources directory not found at {}",
            resources_dir.display()
        );
    }

    // Generate Rust source
    let mut generated = String::new();

    writeln!(generated, "// @generated by onshape-mcp-resources/build.rs").expect("write failed");
    writeln!(generated, "// Do not edit manually.").expect("write failed");
    writeln!(generated).expect("write failed");
    writeln!(generated, "pub const RESOURCES: &[ResourceEntry] = &[").expect("write failed");

    for (group, entry) in &all_entries {
        let uri = format!("{group}:{}", entry.name);

        writeln!(generated, "    ResourceEntry {{").expect("write failed");
        writeln!(generated, "        group: {},", rust_string_literal(group))
            .expect("write failed");
        writeln!(
            generated,
            "        name: {},",
            rust_string_literal(&entry.name)
        )
        .expect("write failed");
        writeln!(
            generated,
            "        title: {},",
            rust_string_literal(&entry.title)
        )
        .expect("write failed");
        writeln!(
            generated,
            "        description: {},",
            rust_string_literal(&entry.description)
        )
        .expect("write failed");
        writeln!(generated, "        uri: {},", rust_string_literal(&uri)).expect("write failed");
        writeln!(
            generated,
            "        content: {},",
            rust_string_literal(&entry.content)
        )
        .expect("write failed");
        writeln!(generated, "    }},").expect("write failed");
    }

    writeln!(generated, "];").expect("write failed");

    let out_file = out_dir.join("resources_generated.rs");
    fs::write(&out_file, generated)
        .unwrap_or_else(|e| panic!("Failed to write {}: {e}", out_file.display()));
}
