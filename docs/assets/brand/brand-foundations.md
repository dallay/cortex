---
title: CORTEX — Shared Visual Foundations
status: ACCEPTED_DIRECTION / SHARED_TYPOGRAPHY / WORKING_IDENTITIES
updated: 2026-11-21
---

## CORTEX — Shared brand foundations

## Brand architecture

- **CORTEX:** umbrella Rust-centric monorepo and possible future ecosystem. Its own logo has been deliberately deferred.
- **ROOK:** independent Rust AI Gateway supporting multiple model providers, routing, fallback, caching and observability.
- **HUGINN:** independent plugin-first coding agent.
- **MUNINN:** independent visual identity in exploration. Its current study pairs a black or white body with a Coral flame tip.

## Visual direction

**ACCEPTED:** `B — Modular Systems` as the brand family language. Coherence comes from precise proportions, meaningful negative space, solid geometry, restraint and shared colors; it does **not** require sharing isotypes, literal metaphors, a runtime architecture or a product logo shape.

| Color    | Hex       | Role                                                 |
|----------|-----------|------------------------------------------------------|
| Charcoal | `#1A1A1A` | Dark artwork, structural elements, text              |
| Ivory    | `#F7F5F2` | Light surfaces, inverse lettering                    |
| Coral    | `#E84A3C` | Focused brand accent, never small body text on Ivory |

Use independent semantic colors for status/feedback in web and terminal applications. Color-mapping for Ratatui and limited-color terminals is a **separate implementation decision**.

## Typography — selected for product interfaces (2026-11-02)

**Shared supporting typography for rook, Huginn and Muninn web, app and documentation:**

| Role                                           | Typeface                            | Scope                                                                |
|------------------------------------------------|-------------------------------------|----------------------------------------------------------------------|
| Interface / marketing / documentation text     | **Geist Sans**                      | UI copy, navigation, headlines, forms, labels and editorial content  |
| Code / configuration / logs / technical values | **Geist Mono**                      | Code blocks, commands, terminal examples, paths, IDs and diagnostics |
| Artwork wordmarks                              | **Product-specific vector artwork** | Use the master and exports for that product.                         |
| Text TUI (Ratatui)                             | **Terminal user's monospaced font** | **Geist Mono is a recommendation**, not something the TUI can force  |

Keep the supporting font choices, fallbacks and licensing guidance here. Maintain each product’s lettering and symbol geometry in its own Affinity master.

### Recommended web implementation

Install or serve Geist Sans and Geist Mono from authorized font assets, preferably self-hosted. Define shared tokens and resilient fallbacks:

```css
:root {
  --font-sans: "Geist", "Geist Sans", system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  --font-mono: "Geist Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
}

body {
  font-family: var(--font-sans);
}

code, pre, kbd, samp {
  font-family: var(--font-mono);
}
```

**Implementation caveat:** The effective family name is defined by the font files or framework package actually loaded. Check it in the browser's computed fonts before shipping. Do not assume the above CSS alone installs the font. Use existing design tokens when integrating; avoid separate copies per product.

### Suggested hierarchy — implementation defaults, not brand-certified metrics

- UI body: 16px / 1.5 line height; compact metadata: 12–14px with contrast and readability review.
- Heading weights: 600–700; body: 400–500; mono: 400–600 where appropriate.
- Prefer sentence case in product UI; reserve uppercase emphasis for official wordmarks and short identifiers.
- Avoid excessive tracking and forced monospaced text in long paragraphs.

### Terminal interface policy

Ratatui renders terminal cells; **it does not choose or install the user's terminal font**. The terminal emulator decides the actual font family, glyph coverage and text size. Recommend Geist Mono as an optional user setting for visual parity with the web; provide functional output with other common monospaced fonts. Do not assume Nerd Font glyphs or ligatures, and provide ASCII/Unicode fallbacks where symbols matter. CLI output should not fail merely because Geist Mono is not installed. Terminal colors, 24-bit/256/16-color capability, background selection and high-contrast themes require an independent implementation spec.

**Evidence / license:** Geist is published by Vercel under the **SIL Open Font License**. Keep the license alongside any distributed font assets if the product later bundles them; this documentation does not include or distribute font files.

- Vercel: <https://vercel.com/font>
- Official repository: <https://github.com/vercel/geist-font>
- Font package: <https://www.npmjs.com/package/geist>
- Ratatui font FAQ: <https://ratatui.rs/faq/>

## Shared and product-specific files

The root of `brand/` contains the [index](./README.md) and these shared foundations. Use this document for typography, palette roles, family principles and common delivery rules. Each brand folder owns its own Affinity master, symbols, usage guide, assets and presentation exports. The [brand index](./README.md#product-folders) links to the current deliverables and records their status.

## Master and export workflow

1. Edit the product’s Affinity master and use its linked symbols when the mark has been implemented as a component.
2. Check the visual result on light and dark backgrounds and at the intended sizes. Review the isotype on its own and inside the full mark.
3. Export reusable marks as self-contained SVG paths with matching transparent PNGs where appropriate. Keep presentation exports under `exports/artboards/`.
4. Keep studies labeled as studies until the mark is approved for rollout. Update the product guide when the master or its usage rules change.
5. Replace published assets after review and authorization. Preserve earlier files and established paths when they are still needed.

Trademark clearance, print proofs and production minimum sizes require their own review.

[Back to brand index](./README.md)
