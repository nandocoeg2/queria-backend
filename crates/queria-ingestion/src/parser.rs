use crate::model::{ParsedChunk, ParsedDocument, ParsedSection};
use queria_core::{QueriaError, QueriaResult};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

pub fn parse_document(path: &str, content: &str) -> QueriaResult<ParsedDocument> {
    validate_relative_path(path)?;
    if content.trim().is_empty() {
        return Err(QueriaError::Validation(
            "source document is empty".to_owned(),
        ));
    }

    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (parser, sections) = match extension.as_str() {
        "md" | "mdx" => ("markdown", parse_markdown(content)),
        "astro" => ("astro", parse_astro(content)),
        "ts" | "tsx" | "js" | "jsx" => ("typescript", parse_typescript(content)),
        "json" => {
            let parser = if serde_json::from_str::<serde_json::Value>(content).is_ok() {
                "json"
            } else {
                json5::from_str::<serde_json::Value>(content).map_err(|error| {
                    QueriaError::Validation(format!("invalid JSON/JSONC: {error}"))
                })?;
                "jsonc"
            };
            (parser, parse_json_sections(content))
        }
        "yaml" | "yml" => {
            serde_yaml::from_str::<serde_yaml::Value>(content)
                .map_err(|error| QueriaError::Validation(format!("invalid YAML: {error}")))?;
            ("yaml", parse_yaml_sections(content))
        }
        "toml" => {
            toml::from_str::<toml::Value>(content)
                .map_err(|error| QueriaError::Validation(format!("invalid TOML: {error}")))?;
            ("toml", parse_toml_sections(content))
        }
        _ => {
            return Err(QueriaError::Validation(format!(
                "unsupported source document: {path}"
            )));
        }
    };

    Ok(ParsedDocument {
        parser: parser.to_owned(),
        sections,
    })
}

pub fn chunk_sections(
    path: &str,
    sections: &[ParsedSection],
    max_lines: usize,
    overlap_lines: usize,
) -> QueriaResult<Vec<ParsedChunk>> {
    validate_relative_path(path)?;
    if max_lines == 0 || overlap_lines >= max_lines {
        return Err(QueriaError::Validation(
            "chunk lines must be positive and overlap must be smaller than the chunk".to_owned(),
        ));
    }

    let step = max_lines - overlap_lines;
    let mut chunks = Vec::new();
    let mut global_index = 0;
    for section in sections {
        let lines = section.body.lines().collect::<Vec<_>>();
        let mut start = 0;
        let mut section_chunk_index = 0;
        while start < lines.len() {
            let end = (start + max_lines).min(lines.len());
            let body = lines[start..end].join("\n");
            let stable_key = hash_parts(&[
                path.as_bytes(),
                section.key.as_bytes(),
                section_chunk_index.to_string().as_bytes(),
            ]);
            chunks.push(ParsedChunk {
                stable_key,
                title: section.title.clone(),
                content_hash: hash_parts(&[body.as_bytes()]),
                body,
                chunk_index: global_index,
                line_start: section.line_start + start,
                line_end: section.line_start + end - 1,
                citation_path: path.to_owned(),
            });
            global_index += 1;
            section_chunk_index += 1;
            if end == lines.len() {
                break;
            }
            start += step;
        }
    }
    Ok(chunks)
}

fn parse_markdown(content: &str) -> Vec<ParsedSection> {
    let lines = content.lines().collect::<Vec<_>>();
    let boundaries = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| markdown_heading(line).map(|title| (index, title)))
        .collect();
    sections_from_boundaries(&lines, boundaries, "Document", true)
}

fn parse_astro(content: &str) -> Vec<ParsedSection> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut boundaries = Vec::new();
    let frontmatter_end = if lines.first().is_some_and(|line| line.trim() == "---") {
        lines
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(index, line)| (line.trim() == "---").then_some(index))
    } else {
        None
    };
    if frontmatter_end.is_some() {
        boundaries.push((0, "Frontmatter".to_owned()));
    }
    boundaries.extend(
        lines
            .iter()
            .enumerate()
            .filter(|(index, _)| frontmatter_end.is_none_or(|end| *index > end))
            .filter_map(|(index, line)| html_heading(line).map(|title| (index, title))),
    );
    sections_from_boundaries(&lines, boundaries, "Astro component", true)
}

fn parse_typescript(content: &str) -> Vec<ParsedSection> {
    let lines = content.lines().collect::<Vec<_>>();
    let boundaries = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| exported_symbol(line).map(|title| (index, title)))
        .collect();
    sections_from_boundaries(&lines, boundaries, "Module preamble", true)
}

fn parse_json_sections(content: &str) -> Vec<ParsedSection> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut depth = 0_i32;
    let mut boundaries = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if depth == 1
            && let Some(key) = json_key(trimmed)
        {
            boundaries.push((index, key));
        }
        depth += brace_delta(line);
    }
    sections_from_boundaries(&lines, boundaries, "JSON document", false)
}

fn parse_yaml_sections(content: &str) -> Vec<ParsedSection> {
    let lines = content.lines().collect::<Vec<_>>();
    let boundaries = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.starts_with([' ', '\t']))
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            let (key, _) = trimmed.split_once(':')?;
            (!key.is_empty() && !key.starts_with(['#', '-', '{', '[']))
                .then(|| (index, key.trim_matches(['\'', '"']).to_owned()))
        })
        .collect();
    sections_from_boundaries(&lines, boundaries, "YAML document", false)
}

fn parse_toml_sections(content: &str) -> Vec<ParsedSection> {
    let lines = content.lines().collect::<Vec<_>>();
    let boundaries = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                Some((
                    index,
                    trimmed
                        .trim_matches(['[', ']'])
                        .trim()
                        .trim_matches(['\'', '"'])
                        .to_owned(),
                ))
            } else {
                None
            }
        })
        .collect();
    sections_from_boundaries(&lines, boundaries, "TOML document", false)
}

fn sections_from_boundaries(
    lines: &[&str],
    mut boundaries: Vec<(usize, String)>,
    fallback_title: &str,
    include_preamble: bool,
) -> Vec<ParsedSection> {
    if include_preamble && boundaries.first().is_none_or(|(index, _)| *index > 0) {
        boundaries.insert(0, (0, fallback_title.to_owned()));
    }
    if boundaries.is_empty() {
        boundaries.push((0, fallback_title.to_owned()));
    }

    boundaries
        .iter()
        .enumerate()
        .filter_map(|(boundary_index, (start, title))| {
            let end = boundaries
                .get(boundary_index + 1)
                .map_or(lines.len(), |(next_start, _)| *next_start);
            (*start < end).then(|| ParsedSection {
                key: format!("{}:{title}", start + 1),
                title: title.clone(),
                body: lines[*start..end].join("\n"),
                line_start: start + 1,
                line_end: end,
            })
        })
        .collect()
}

fn markdown_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let marker_count = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&marker_count) || trimmed.as_bytes().get(marker_count) != Some(&b' ') {
        return None;
    }
    let title = trimmed[marker_count..].trim().trim_end_matches('#').trim();
    (!title.is_empty()).then(|| title.to_owned())
}

fn html_heading(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let level = (1..=6).find(|level| trimmed.starts_with(&format!("<h{level}")))?;
    let body_start = trimmed.find('>')? + 1;
    let closing = format!("</h{level}>");
    let body_end = trimmed[body_start..].find(&closing)? + body_start;
    let title = strip_inline_tags(&trimmed[body_start..body_end]);
    (!title.is_empty()).then_some(title)
}

fn strip_inline_tags(value: &str) -> String {
    let mut output = String::new();
    let mut inside_tag = false;
    for character in value.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    output.trim().to_owned()
}

fn exported_symbol(line: &str) -> Option<String> {
    let mut rest = line.trim_start().strip_prefix("export ")?.trim_start();
    rest = rest.strip_prefix("default ").unwrap_or(rest).trim_start();
    rest = rest.strip_prefix("async ").unwrap_or(rest).trim_start();
    for keyword in [
        "interface",
        "class",
        "function",
        "type",
        "enum",
        "const",
        "let",
        "var",
    ] {
        if let Some(name) = rest.strip_prefix(keyword).and_then(|value| {
            value
                .trim_start()
                .split(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '<' | '(' | '=' | ':' | '{' | ';')
                })
                .next()
        }) && !name.is_empty()
        {
            return Some(name.to_owned());
        }
    }
    None
}

fn json_key(line: &str) -> Option<String> {
    let rest = line.strip_prefix('"')?;
    let quote = rest.find('"')?;
    rest[quote + 1..]
        .trim_start()
        .starts_with(':')
        .then(|| rest[..quote].to_owned())
}

fn brace_delta(line: &str) -> i32 {
    let mut delta = 0;
    let mut in_string = false;
    let mut escaped = false;
    for character in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && in_string {
            escaped = true;
        } else if character == '"' {
            in_string = !in_string;
        } else if !in_string {
            match character {
                '{' | '[' => delta += 1,
                '}' | ']' => delta -= 1,
                _ => {}
            }
        }
    }
    delta
}

fn hash_parts(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_relative_path(path: &str) -> QueriaResult<()> {
    if path.is_empty()
        || Path::new(path).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(QueriaError::Validation(
            "source path must be repository-relative".to_owned(),
        ));
    }
    Ok(())
}
