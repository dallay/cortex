# AI Search Visibility

A class of AI search engines (ChatGPT search, Perplexity, Gemini Overviews) cite web pages from
their training and retrieval pipelines, not from the classic ranked results. As of 2026 this is an
unstable area — there are no confirmed ranking signals — but a few things are low-cost and won't
hurt:

- **Don't block AI crawlers wholesale.** `OAI-SearchBot`, `PerplexityBot`, `GoogleOther`,
  `Google-Extended`, `ClaudeBot`, etc. each have separate `robots.txt` user-agents. Decide per-bot
  rather than blanket-blocking — a `Disallow` removes you from that bot's citations.
- **Lean on schema.org `Article`/`Product`/`FAQPage`.** AI summarizers parse structured data more
  reliably than they parse prose layouts. The structured-data examples in the main skill also help
  here.
- **Make first-paragraph answers self-contained.** Both featured snippets and AI summaries pull
  short, coherent passages. A definition or direct answer in the first 1-2 sentences is more
  extractable than the same content buried under marketing prose.

## `llms.txt` — emerging, unproven

[`llms.txt`](https://llmstxt.org/) is a proposed convention (a Markdown index of your site's
important pages, served at `/llms.txt`) for LLMs to consume. As of mid-2026 adoption is ~0.015% of
sites and **no major AI vendor has confirmed they read it**. Treat it as a 5-minute speculative add
for content sites — not a meaningful ranking or citation factor — and don't reorganize content
around it.
