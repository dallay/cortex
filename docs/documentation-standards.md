---
title: CORTEX Documentation Standard
status: accepted
updated: 2026-10-20
scope: cortex
format: markdown
audience:
  - maintainers
  - documentation-authors
tags:
  - cortex
  - documentation
  - obsidian
---

# CORTEX documentation standard

This is the canonical documentation-format policy for **CORTEX**, including Rook, Huginn, Muninn, cross-product architecture, research, ADRs, diagrams, and audit reports. The maintainer explicitly selected Markdown for reliable use in **Obsidian** on 2026-10-20.

## Non-negotiable format rule

1. **Author, edit, exchange, and preserve documentation as real UTF-8 `.md` files.** Markdown is the default and canonical format, both in the GitHub repository and in the project's Google Drive knowledge mirror.
2. **Never create native Google Docs (`.gdoc`, MIME type `application/vnd.google-apps.document`) unless the maintainer explicitly requests a Google Doc for that specific task.** This prohibition includes *temporary drafts, staging files, conversion intermediates, exports, and automatic import/conversion workflows*.
3. If a tool cannot write an actual Markdown file to the requested destination, **do not silently substitute a Google Doc**. Produce a valid local `.md` artifact, explain the destination limitation, or use an available raw-file upload path.
4. Do not change the canonical format merely because another app/editor is connected. Other formats (`.docx`, PDF, HTML, Slides) require an explicit request and do not supersede the maintained Markdown source.
5. Treat existing non-Markdown documents as migration candidates. Preserve content, diagrams, references, decisions, and provenance during conversion; verify the replacement before deleting or archiving an original. Never claim the migration is complete without checking the resulting files.

This rule applies to future work without needing to be repeated in each conversation.

## Portable Obsidian-compatible Markdown

Use **CommonMark and GitHub-Flavored Markdown (GFM)** with optional, deliberate Obsidian extensions. Prefer Markdown that also renders correctly in GitHub.

- Use **one H1** document title, then ordered H2/H3 sections. Do not skip heading levels solely for visual size.
- Separate block elements with **one blank line**, avoid multiple consecutive blank lines, and end files with a final newline.
- Write UTF-8 without a byte-order mark (BOM), with LF newlines. Never introduce CRLF, non-breaking space padding, or generated zero-width characters.
- Use standard `-` lists and correctly indented nested lists; keep tables valid, with a header separator and consistent columns.
- Use fenced code blocks with the appropriate language (`rust`, `toml`, `bash`, `json`, `mermaid`, etc.). Close every fence. Do not turn source code into screenshots.
- Prefer simple Markdown emphasis, lists, blockquotes, links, and images over raw HTML. Do not require external CSS or plugins to understand the document.
- Do not escape syntax unnecessarily: write `https://example.com` rather than `https\://example.com`, and use `## 1. Background` rather than a literal escaped numeral heading.
- Use readable paragraphs rather than forced hard-line breaks. Do not append two spaces to ordinary lines unless an intentional Markdown hard break is needed.
- Place Obsidian-only features such as `[[wikilinks]]`, embeds, properties, or callouts where they add genuine value; ordinary links remain the interoperable default.

### YAML frontmatter

Use valid YAML frontmatter on substantive documents that benefit from indexed metadata. Keep it at the very beginning of the file. Use meaningful values rather than placeholders; `README.md` and short reference notes need not carry frontmatter.

Recommended metadata:

```yaml
---
title: Huginn — Component Architecture
product: huginn
status: implemented
updated: 2026-10-20
verified_commit: 5271f510dc6da31bf99be4c00dfaf8bad92b187a
tags:
  - cortex
  - architecture
  - huginn
---
```

**Status semantics:** `implemented` means code/tests support the documented behavior; `accepted` means the decision was selected but may not be shipped; `proposed` means exploratory; `historical` means superseded or provenance-only; `acceptance-pending` means implementation does not yet have the stated human/platform acceptance evidence.

Do not automatically update `verified_commit` or `updated` without actually inspecting the source and changing/validating the document.

## Links, attachments, and vault structure

- When both documents exist in the **same exported Obsidian vault**, use repository/vault-relative Markdown links such as `[Plugin lifecycle](plugin-lifecycle.md)`. Resolve them from the current file and test the target exists.
- For material **outside** the vault, use a real canonical HTTPS URL (e.g., a GitHub permalink to code/ADR); don't fabricate relative references between independent Drive files.
- Keep stable lowercase-kebab-case filenames where practical; preserve established ADR numbers and historical filenames. Avoid renaming a referenced document without updating inbound links.
- Use `![Alt text](assets/diagram.svg)` with meaningful alt text for required static imagery. Keep editable diagram sources in the repository/vault.
- Do not use `file:///...`, machine-local home-directory paths, temporary workspace paths, personal absolute paths, or connector-only resource IDs as shared documentation links.
- Google Drive is a **raw Markdown knowledge mirror**, not the canonical editor representation. Preserve folder organization when exporting its Markdown files for Obsidian.

## Mermaid architecture diagrams

- Store diagrams in `mermaid` fenced blocks inside `.md` (preferred) or linked `.mmd` sources when necessary.
- Follow the same section hierarchy and surrounding blank lines as other fenced code blocks.
- Validate Mermaid syntax with a Mermaid renderer as well as checking fence balance. **Fence balance alone does not prove the diagram renders.**
- Keep diagrams accurate to an explicitly dated code/ADR snapshot, and label future proposals separately.
- Do not replace Mermaid with PNGs merely to fix formatting or renderer failures. Correct the source diagram.
- Avoid diagrams so large that navigation is impaired; use linked component-level diagrams rather than one all-encompassing image.

## Source-of-truth and evidence

- Current implemented behavior: repository source/tests at an identified commit.
- Accepted architecture: authoritative, versioned ADRs.
- Unexecuted test plans: `NOT_RUN` or the actual recorded state, not `PASS`.
- Research, old specs, experimental designs, and proposals: explicitly non-normative.
- Keep the existing product boundaries: Rook and Huginn are independent applications; Muninn is planned until implementation evidence exists.

The existence of a diagram or document is **not** acceptance evidence.

## Authoring, migration, and review checklist

Before delivering any newly created or modified document:

- [ ] The artifact is a real `.md` file (not a renamed `.gdoc`, Google Docs file, or rendered HTML export).
- [ ] If a temporary file was needed, it was also Markdown/plain text rather than a native Google Doc.
- [ ] UTF-8, LF, no BOM, one trailing newline, no repeated empty paragraphs or accidental escapes.
- [ ] Clear title and H1/H2/H3 hierarchy; valid optional YAML frontmatter.
- [ ] Lists, tables, code fences, and Mermaid fences parse and display in Obsidian.
- [ ] Relative links, asset paths, and external URLs are preserved and checked as far as tools permit.
- [ ] Implementation vs. accepted vs. proposed status is accurately labeled, with provenance.
- [ ] No substantive information, approval, ADR decision, code sample, or cited reference was lost.
- [ ] The resulting Markdown bytes were read back from their destination before claiming success.

Repository baseline lint: `pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md"`. Also render Mermaid diagrams and verify links as separate checks; Markdown lint alone is not enough.

## Exceptions

Only the maintainer's **explicit task-specific request** may authorize a Google Doc or another documentation output. An exception for one artifact does not change the Markdown-only project default, and no `.gdoc` may be generated automatically as a convenience in unrelated tasks.
