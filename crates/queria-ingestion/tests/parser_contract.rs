use queria_ingestion::parser::{chunk_sections, parse_document};

#[test]
fn markdown_sections_preserve_heading_line_ranges() {
    let source = "intro\n# Deploy\nfirst\nsecond\n## Rollback\nundo\n";
    let parsed = parse_document("docs/runbook.mdx", source).expect("markdown should parse");

    assert_eq!(parsed.parser, "markdown");
    assert_eq!(parsed.sections.len(), 3);
    assert_eq!(parsed.sections[1].title, "Deploy");
    assert_eq!(
        (parsed.sections[1].line_start, parsed.sections[1].line_end),
        (2, 4)
    );
    assert_eq!(parsed.sections[2].title, "Rollback");
}

#[test]
fn astro_sections_include_frontmatter_and_markup_headings() {
    let source = "---\nconst title = 'Home';\n---\n<h1>Welcome</h1>\n<p>Hello</p>\n<h2>Setup</h2>\n<p>Run it</p>\n";
    let parsed = parse_document("src/pages/index.astro", source).expect("astro should parse");

    assert_eq!(parsed.parser, "astro");
    assert_eq!(parsed.sections[0].title, "Frontmatter");
    assert_eq!(parsed.sections[1].title, "Welcome");
    assert_eq!(parsed.sections[2].title, "Setup");
}

#[test]
fn typescript_sections_follow_exported_symbols() {
    let source = "const hidden = 1;\nexport interface User { id: string }\nexport function loadUser() {\n  return hidden;\n}\nexport const saveUser = () => true;\n";
    let parsed = parse_document("src/users.ts", source).expect("typescript should parse");

    assert_eq!(parsed.parser, "typescript");
    assert_eq!(parsed.sections.len(), 4);
    assert_eq!(parsed.sections[1].title, "User");
    assert_eq!(parsed.sections[2].title, "loadUser");
    assert_eq!(parsed.sections[2].line_end, 5);
    assert_eq!(parsed.sections[3].title, "saveUser");
}

#[test]
fn structured_configs_validate_and_extract_top_level_sections() {
    let json = parse_document(
        "config/app.json",
        "{\n  \"server\": {},\n  \"auth\": {}\n}\n",
    )
    .expect("json should parse");
    let yaml = parse_document(
        "config/app.yaml",
        "server:\n  port: 80\nauth:\n  enabled: true\n",
    )
    .expect("yaml should parse");
    let toml = parse_document(
        "config/app.toml",
        "[server]\nport = 80\n[auth]\nenabled = true\n",
    )
    .expect("toml should parse");

    assert_eq!(
        json.sections
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        ["server", "auth"]
    );
    assert_eq!(
        yaml.sections
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        ["server", "auth"]
    );
    assert_eq!(
        toml.sections
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        ["server", "auth"]
    );
    assert!(parse_document("config/bad.json", "{").is_err());
}

#[test]
fn json_config_accepts_jsonc_comments_and_trailing_commas() {
    let parsed = parse_document(
        "tsconfig.app.json",
        "{\n  // compiler settings\n  \"compilerOptions\": {\n    \"strict\": true,\n  },\n}\n",
    )
    .expect("JSONC config should parse");

    assert_eq!(parsed.parser, "jsonc");
    assert_eq!(parsed.sections[0].title, "compilerOptions");
}

#[test]
fn chunks_are_deterministic_and_keep_citations() {
    let source = "# Deploy\none\ntwo\nthree\nfour\nfive\n";
    let parsed = parse_document("docs/deploy.md", source).expect("document should parse");
    let first =
        chunk_sections("docs/deploy.md", &parsed.sections, 3, 1).expect("chunking should succeed");
    let second = chunk_sections("docs/deploy.md", &parsed.sections, 3, 1)
        .expect("chunking should be repeatable");

    assert_eq!(first, second);
    assert_eq!(first.len(), 3);
    assert_eq!((first[0].line_start, first[0].line_end), (1, 3));
    assert_eq!((first[1].line_start, first[1].line_end), (3, 5));
    assert_eq!(first[0].citation_path, "docs/deploy.md");
    assert_ne!(first[0].stable_key, first[1].stable_key);
    assert!(!first[0].content_hash.is_empty());
}
